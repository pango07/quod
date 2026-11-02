import type { Db, Queryable } from "@/lib/db/types";
import {
  jobRowFromPayload,
  runRowFromPayload,
  type GhInstallation,
  type InstallationPayload,
  type InstallationRepositoriesPayload,
  type InstallationTargetPayload,
  type WorkflowJobPayload,
  type WorkflowRunPayload,
} from "./payloads";
import { markProtectionStale } from "./protection";
import { verifySignature } from "./signature";
import {
  addRepos,
  deleteInstallation,
  ensureInstallation,
  recordDelivery,
  removeRepos,
  renameAccount,
  touchRepo,
  upsertInstallation,
  upsertJob,
  upsertRun,
} from "./store";

/** When an installation delivery happened, by GitHub's clock if it says. */
function eventTime(inst: GhInstallation, fallback: Date): Date {
  const d = inst.updated_at ? new Date(inst.updated_at) : null;
  return d && !Number.isNaN(d.getTime()) ? d : fallback;
}

// The webhook receiver's logic, without Next.js: verify, dedupe, apply. The
// route (app/api/github/webhook/route.ts) passes the raw body and headers.
// The dedupe row and the writes share one transaction, so a delivery that
// fails part way is not marked seen and GitHub's redelivery applies it.

/** GitHub's documented maximum payload size. */
export const MAX_BODY = 25 * 1024 * 1024;

export const HANDLED_EVENTS = [
  "installation",
  "installation_repositories",
  "installation_target",
  "workflow_run",
  "workflow_job",
  // Default-branch protection may have changed: the cached state is read again on the next view (lib/github/protection.ts).
  "branch_protection_rule",
  "repository_ruleset",
  "repository",
] as const;

/** Webhooks that can change a repo's default-branch protection (or which branch is the default). */
interface ProtectionPayload {
  action?: string;
  installation?: { id?: number };
  repository?: { id?: number } | null;
  changes?: Record<string, unknown>;
}

export interface WebhookResult {
  status: number;
  body: { ok: boolean; duplicate?: boolean; ignored?: boolean; error?: string };
  /** For the log line: ids and event types only. */
  delivery?: string;
  event?: string;
  action?: string;
  /** live_version of each applied run/job write (empty when nothing changed). */
  versions?: number[];
}

const DELIVERY = /^[A-Za-z0-9-]{1,100}$/;
const EVENT = /^[a-z_]{1,64}$/;

export async function handleWebhook(input: {
  db: Db;
  secret: string | undefined;
  headers: Headers;
  body: Uint8Array;
  now?: Date;
}): Promise<WebhookResult> {
  const { db, secret, headers, body } = input;
  if (!secret) return { status: 500, body: { ok: false, error: "webhook secret not configured" } };
  if (body.byteLength > MAX_BODY) return { status: 413, body: { ok: false, error: "too large" } };
  if (!verifySignature(secret, body, headers.get("x-hub-signature-256"))) return { status: 401, body: { ok: false, error: "bad signature" } };

  const delivery = headers.get("x-github-delivery") ?? "";
  const event = headers.get("x-github-event") ?? "";
  if (!DELIVERY.test(delivery) || !EVENT.test(event)) return { status: 400, body: { ok: false, error: "missing delivery or event header" } };

  let payload: { action?: unknown };
  try {
    payload = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(body));
  } catch {
    return { status: 400, body: { ok: false, error: "body is not JSON" }, delivery, event };
  }
  if (!payload || typeof payload !== "object") return { status: 400, body: { ok: false, error: "body is not an object" }, delivery, event };
  const action = typeof payload.action === "string" ? payload.action.slice(0, 64) : undefined;
  const now = input.now ?? new Date();

  return db.tx(async (q) => {
    if (!(await recordDelivery(q, delivery, event))) return { status: 200, body: { ok: true, duplicate: true }, delivery, event, action };
    const versions: number[] = [];
    const handled = await apply(q, event, payload, now, versions);
    return { status: 200, body: handled ? { ok: true } : { ok: true, ignored: true }, delivery, event, action, versions };
  });
}

async function apply(q: Queryable, event: string, payload: unknown, now: Date, versions: number[]): Promise<boolean> {
  switch (event) {
    case "installation": {
      const p = payload as InstallationPayload;
      if (!p.installation?.id) return false;
      if (p.action === "deleted") {
        // Terminal: an installation id is never reused, so no later delivery can undo it.
        await deleteInstallation(q, p.installation.id);
        return true;
      }
      const suspend = p.action === "suspend" || p.action === "unsuspend" ? p.action : undefined;
      const applied = await upsertInstallation(q, p.installation, now, suspend);
      if (applied && p.action === "created" && p.repositories?.length) await addRepos(q, p.installation.id, p.repositories, eventTime(p.installation, now));
      return true;
    }
    case "installation_repositories": {
      const p = payload as InstallationRepositoriesPayload;
      if (!p.installation?.id) return false;
      await upsertInstallation(q, { ...p.installation, repository_selection: p.repository_selection ?? p.installation.repository_selection }, now);
      // Each repo's membership is ordered on its own (repos.membership_at), whatever the installation row did.
      const at = eventTime(p.installation, now);
      await addRepos(q, p.installation.id, p.repositories_added ?? [], at);
      await removeRepos(q, p.installation.id, (p.repositories_removed ?? []).map((r) => r.id), at);
      return true;
    }
    case "installation_target": {
      // The account was renamed (the only action GitHub sends today).
      const p = payload as InstallationTargetPayload;
      if (!p.installation?.id || !p.account?.login) return false;
      await renameAccount(q, p.installation.id, p.account);
      return true;
    }
    case "workflow_run": {
      const p = payload as WorkflowRunPayload;
      const row = runRowFromPayload(p, now);
      if (!row) return false;
      await ensureInstallation(q, row.installation_id, p.repository.owner);
      await touchRepo(q, row.installation_id, p.repository);
      const applied = await upsertRun(q, row);
      if (applied) versions.push(applied.version);
      return true;
    }
    case "workflow_job": {
      const p = payload as WorkflowJobPayload;
      const row = jobRowFromPayload(p, now);
      if (!row) return false;
      await ensureInstallation(q, row.installation_id, p.repository.owner);
      await touchRepo(q, row.installation_id, p.repository);
      const v = await upsertJob(q, row);
      if (v !== null) versions.push(v);
      return true;
    }
    case "branch_protection_rule":
    case "repository_ruleset":
    case "repository": {
      const p = payload as ProtectionPayload;
      const installationId = typeof p.installation?.id === "number" ? p.installation.id : null;
      const repoId = typeof p.repository?.id === "number" ? p.repository.id : null;
      // repository: only a new default branch (or a rename, archive, transfer) matters, not every edit.
      if (event === "repository" && !(p.action === "renamed" || p.action === "transferred" || p.action === "archived" || p.action === "unarchived" || (p.action === "edited" && p.changes && "default_branch" in p.changes)))
        return false;
      // A ruleset without a repository is the organization's: every repo of the installation.
      await markProtectionStale(q, { repoId, installationId: repoId === null ? installationId : null });
      return true;
    }
    default:
      // ping, github_app_authorization and anything else: recorded, not stored.
      return false;
  }
}

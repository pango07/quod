import type { Db, Queryable } from "@/lib/db/types";
import {
  jobRowFromPayload,
  runRowFromPayload,
  type InstallationPayload,
  type InstallationRepositoriesPayload,
  type WorkflowJobPayload,
  type WorkflowRunPayload,
} from "./payloads";
import { verifySignature } from "./signature";
import {
  deleteInstallation,
  ensureInstallation,
  recordDelivery,
  removeRepos,
  setSuspended,
  upsertInstallation,
  upsertJob,
  upsertRepos,
  upsertRun,
} from "./store";

// The webhook receiver's logic, without Next.js: verify, dedupe, apply. The
// route (app/api/github/webhook/route.ts) passes the raw body and headers.
// The dedupe row and the writes share one transaction, so a delivery that
// fails part way is not marked seen and GitHub's redelivery applies it.

/** GitHub's documented maximum payload size. */
export const MAX_BODY = 25 * 1024 * 1024;

export const HANDLED_EVENTS = ["installation", "installation_repositories", "workflow_run", "workflow_job"] as const;

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
        await deleteInstallation(q, p.installation.id);
        return true;
      }
      await upsertInstallation(q, p.installation);
      if (p.action === "suspend") await setSuspended(q, p.installation.id, true);
      if (p.action === "unsuspend") await setSuspended(q, p.installation.id, false);
      if (p.action === "created" && p.repositories?.length) await upsertRepos(q, p.installation.id, p.repositories);
      return true;
    }
    case "installation_repositories": {
      const p = payload as InstallationRepositoriesPayload;
      if (!p.installation?.id) return false;
      await upsertInstallation(q, { ...p.installation, repository_selection: p.repository_selection ?? p.installation.repository_selection });
      await upsertRepos(q, p.installation.id, p.repositories_added ?? []);
      await removeRepos(q, p.installation.id, (p.repositories_removed ?? []).map((r) => r.id));
      return true;
    }
    case "workflow_run": {
      const p = payload as WorkflowRunPayload;
      const row = runRowFromPayload(p, now);
      if (!row) return false;
      await ensureInstallation(q, row.installation_id, p.repository.owner);
      await upsertRepos(q, row.installation_id, [p.repository], false);
      const applied = await upsertRun(q, row);
      if (applied) versions.push(applied.version);
      return true;
    }
    case "workflow_job": {
      const p = payload as WorkflowJobPayload;
      const row = jobRowFromPayload(p, now);
      if (!row) return false;
      await ensureInstallation(q, row.installation_id, p.repository.owner);
      await upsertRepos(q, row.installation_id, [p.repository], false);
      const v = await upsertJob(q, row);
      if (v !== null) versions.push(v);
      return true;
    }
    default:
      // ping, github_app_authorization and anything else: recorded, not stored.
      return false;
  }
}

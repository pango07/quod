// Test payload builders, shaped like GitHub's webhook payloads (only the fields Quod reads, plus a few it ignores).
import { signBody } from "./signature";

export const SECRET = "test-webhook-secret";
export const OWNER = { id: 500, login: "acme", type: "Organization" };
export const REPO = { id: 9001, name: "api", full_name: "acme/api", private: true, owner: OWNER };

/** Ten minutes before the test run started, so jobs are inside the live window. */
export const BASE = new Date(Math.floor(Date.now() / 60_000) * 60_000 - 10 * 60_000).toISOString();

let n = 0;
export const deliveryId = () => `00000000-0000-4000-8000-${String(++n).padStart(12, "0")}`;

export function delivery(event: string, payload: unknown, id = deliveryId()) {
  const body = new TextEncoder().encode(JSON.stringify(payload));
  const headers = new Headers({ "x-github-event": event, "x-github-delivery": id, "x-hub-signature-256": signBody(SECRET, body), "content-type": "application/json" });
  return { headers, body, id };
}

export function installationPayload(action: string, id = 77, extra: Record<string, unknown> = {}) {
  return { action, installation: { id, account: OWNER, repository_selection: "selected" }, repositories: [{ id: REPO.id, name: REPO.name, full_name: REPO.full_name, private: true }], ...extra };
}

export function jobPayload(status: "queued" | "in_progress" | "completed" | "waiting", o: { id?: number; runId?: number; installation?: number; attempt?: number; t?: string; conclusion?: string; repo?: typeof REPO } = {}) {
  const t = o.t ?? BASE;
  const at = (min: number) => new Date(new Date(t).getTime() + min * 60_000).toISOString();
  const steps =
    status === "queued" || status === "waiting"
      ? []
      : [
          { name: "Set up job", status: "completed", conclusion: "success", number: 1, started_at: at(1), completed_at: at(1) },
          { name: "Run tests", status: status === "completed" ? "completed" : "in_progress", conclusion: status === "completed" ? "success" : null, number: 2, started_at: at(1), completed_at: status === "completed" ? at(4) : null },
        ];
  return {
    action: status,
    installation: { id: o.installation ?? 77 },
    repository: o.repo ?? REPO,
    sender: { login: "octocat" },
    workflow_job: {
      id: o.id ?? 1234,
      run_id: o.runId ?? 555,
      run_attempt: o.attempt ?? 1,
      workflow_name: "CI",
      name: "test",
      head_branch: "main",
      head_sha: "a".repeat(40),
      status,
      conclusion: status === "completed" ? (o.conclusion ?? "success") : null,
      labels: ["ubuntu-latest"],
      runner_name: status === "queued" ? null : "GitHub Actions 2",
      runner_group_name: status === "queued" ? null : "GitHub Actions",
      html_url: `https://github.com/acme/api/actions/runs/${o.runId ?? 555}/job/${o.id ?? 1234}`,
      created_at: t,
      started_at: status === "queued" ? t : at(1),
      completed_at: status === "completed" ? at(4) : null,
      steps,
      // ignored by Quod
      check_run_url: "https://api.github.com/repos/acme/api/check-runs/1",
    },
  };
}

export function runPayload(status: "requested" | "in_progress" | "completed" | "queued", o: { id?: number; attempt?: number; updated?: string; installation?: number; number?: number } = {}) {
  return {
    action: status === "requested" || status === "queued" ? "requested" : status,
    installation: { id: o.installation ?? 77 },
    repository: REPO,
    workflow_run: {
      id: o.id ?? 555,
      name: "CI",
      display_title: "Fix the flaky test",
      run_number: o.number ?? 42,
      run_attempt: o.attempt ?? 1,
      event: "push",
      head_branch: "main",
      head_sha: "a".repeat(40),
      status,
      conclusion: status === "completed" ? "success" : null,
      html_url: `https://github.com/acme/api/actions/runs/${o.id ?? 555}`,
      actor: { login: "jamie" },
      run_started_at: "2026-11-01T10:00:00Z",
      created_at: "2026-11-01T10:00:00Z",
      updated_at: o.updated ?? "2026-11-01T10:00:00Z",
    },
  };
}

// The parts of GitHub's webhook payloads Quod reads, and their mapping to
// table rows. Pure: no database, no Next.js. Everything else in a payload is
// ignored and never stored.

export interface GhAccount {
  id: number;
  login: string;
  type?: string;
}

export interface GhRepo {
  id: number;
  name: string;
  full_name: string;
  private?: boolean;
  owner?: GhAccount;
}

export interface GhInstallation {
  id: number;
  account?: GhAccount | null;
  repository_selection?: string;
  suspended_at?: string | null;
  /** Orders installation deliveries (m2). */
  updated_at?: string | null;
}

export interface InstallationTargetPayload {
  action: "renamed" | string;
  installation: { id: number };
  account: GhAccount;
  target_type?: string;
}

export interface InstallationPayload {
  action: string;
  installation: GhInstallation;
  repositories?: Pick<GhRepo, "id" | "name" | "full_name" | "private">[];
}

export interface InstallationRepositoriesPayload {
  action: "added" | "removed" | string;
  installation: GhInstallation;
  repository_selection?: string;
  repositories_added?: Pick<GhRepo, "id" | "name" | "full_name" | "private">[];
  repositories_removed?: Pick<GhRepo, "id" | "name" | "full_name" | "private">[];
}

export interface GhStep {
  name: string;
  status: string;
  conclusion: string | null;
  number: number;
  started_at?: string | null;
  completed_at?: string | null;
}

export interface WorkflowJobPayload {
  action: string;
  installation?: { id: number };
  repository: GhRepo;
  sender?: { login: string };
  workflow_job: {
    id: number;
    run_id: number;
    run_attempt?: number;
    workflow_name?: string | null;
    name: string;
    head_branch?: string | null;
    head_sha?: string | null;
    status: string;
    conclusion?: string | null;
    labels?: string[];
    runner_name?: string | null;
    runner_group_name?: string | null;
    html_url?: string | null;
    created_at?: string | null;
    started_at?: string | null;
    completed_at?: string | null;
    steps?: GhStep[];
  };
}

export interface WorkflowRunPayload {
  action: string;
  installation?: { id: number };
  repository: GhRepo;
  workflow_run: {
    id: number;
    name?: string | null;
    display_title?: string | null;
    run_number?: number;
    run_attempt?: number;
    event?: string;
    head_branch?: string | null;
    head_sha?: string | null;
    status: string;
    conclusion?: string | null;
    html_url?: string | null;
    actor?: { login: string } | null;
    triggering_actor?: { login: string } | null;
    run_started_at?: string | null;
    created_at?: string | null;
    updated_at?: string | null;
  };
}

/**
 * Status order within one attempt. A delivery may only move a row forward
 * (or sideways with a later timestamp): queued-like 1, in_progress 2, completed 3.
 */
export function statusRank(status: string | null | undefined): number {
  switch (status) {
    case "requested":
    case "pending":
    case "waiting":
    case "queued":
      return 1;
    case "in_progress":
      return 2;
    case "completed":
      return 3;
    default:
      // A status GitHub added after this was written: treat it as not started yet, and say so.
      console.warn("[quod-cloud] unknown github status", { status: typeof status === "string" ? status.slice(0, 32) : status });
      return 1;
  }
}

const time = (v: string | null | undefined): Date | null => {
  if (!v) return null;
  const d = new Date(v);
  return Number.isNaN(d.getTime()) ? null : d;
};

const latest = (...ds: (Date | null)[]): Date | null => ds.reduce<Date | null>((a, d) => (d && (!a || d > a) ? d : a), null);

/** Only the step fields the dashboard shows. */
function cleanSteps(steps: GhStep[] | undefined): GhStep[] {
  return (steps ?? []).map((s) => ({
    name: String(s.name ?? ""),
    status: String(s.status ?? ""),
    conclusion: s.conclusion ?? null,
    number: Number(s.number ?? 0),
    started_at: s.started_at ?? null,
    completed_at: s.completed_at ?? null,
  }));
}

export interface JobRow {
  id: number;
  run_id: number;
  installation_id: number;
  repo_id: number;
  run_attempt: number;
  workflow_name: string | null;
  name: string;
  head_branch: string | null;
  head_sha: string | null;
  sender_login: string | null;
  status: string;
  conclusion: string | null;
  status_rank: number;
  labels: string[];
  runner_name: string | null;
  runner_group_name: string | null;
  html_url: string | null;
  created_at: Date | null;
  started_at: Date | null;
  completed_at: Date | null;
  steps: GhStep[];
  event_at: Date;
}

export function jobRowFromPayload(p: WorkflowJobPayload, receivedAt: Date): JobRow | null {
  const j = p.workflow_job;
  if (!p.installation?.id || !j?.id || !p.repository?.id) return null;
  // Columns that cannot be null: a payload without them is skipped, not a failed transaction.
  if (typeof j.run_id !== "number" || typeof j.name !== "string" || typeof j.status !== "string") return null;
  const created = time(j.created_at);
  const started = time(j.started_at);
  const completed = time(j.completed_at);
  const stepTimes = (j.steps ?? []).flatMap((s) => [time(s.started_at), time(s.completed_at)]);
  return {
    id: j.id,
    run_id: j.run_id,
    installation_id: p.installation.id,
    repo_id: p.repository.id,
    run_attempt: j.run_attempt ?? 1,
    workflow_name: j.workflow_name ?? null,
    name: j.name,
    head_branch: j.head_branch ?? null,
    head_sha: j.head_sha ?? null,
    sender_login: p.sender?.login ?? null,
    status: j.status,
    conclusion: j.conclusion ?? null,
    status_rank: statusRank(j.status),
    labels: Array.isArray(j.labels) ? j.labels.map(String) : [],
    runner_name: j.runner_name ?? null,
    runner_group_name: j.runner_group_name ?? null,
    html_url: j.html_url ?? null,
    created_at: created,
    started_at: started,
    completed_at: completed,
    steps: cleanSteps(j.steps),
    // The latest moment the payload describes; the delivery time only if it has none.
    event_at: latest(created, started, completed, ...stepTimes) ?? receivedAt,
  };
}

export interface RunRow {
  id: number;
  installation_id: number;
  repo_id: number;
  name: string | null;
  display_title: string | null;
  run_number: number | null;
  run_attempt: number;
  event: string | null;
  head_branch: string | null;
  head_sha: string | null;
  actor_login: string | null;
  status: string;
  conclusion: string | null;
  status_rank: number;
  html_url: string | null;
  run_started_at: Date | null;
  created_at: Date | null;
  gh_updated_at: Date;
}

export function runRowFromPayload(p: WorkflowRunPayload, receivedAt: Date): RunRow | null {
  const r = p.workflow_run;
  if (!p.installation?.id || !r?.id || !p.repository?.id) return null;
  if (typeof r.status !== "string") return null;
  const created = time(r.created_at);
  return {
    id: r.id,
    installation_id: p.installation.id,
    repo_id: p.repository.id,
    name: r.name ?? null,
    display_title: r.display_title ?? null,
    run_number: r.run_number ?? null,
    run_attempt: r.run_attempt ?? 1,
    event: r.event ?? null,
    head_branch: r.head_branch ?? null,
    head_sha: r.head_sha ?? null,
    actor_login: r.actor?.login ?? r.triggering_actor?.login ?? null,
    status: r.status,
    conclusion: r.conclusion ?? null,
    status_rank: statusRank(r.status),
    html_url: r.html_url ?? null,
    run_started_at: time(r.run_started_at),
    created_at: created,
    gh_updated_at: time(r.updated_at) ?? created ?? receivedAt,
  };
}

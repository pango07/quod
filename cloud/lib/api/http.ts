// The HTTP adapter: Quod's data from quod-server, protocol quod.v1
// (docs/api.md; the server is crates/quod-server, its dashboard routes in
// src/dash.rs). Server-side only: the bearer token never reaches a browser,
// and no cookie is ever sent to the server (the headers below are the only
// ones it gets).
//
// - Path ids: the team is quod-server's team id for the dashboard team
//   (`credentials`), repos are quod-server's repo slugs (the composite maps
//   them to the dashboard's), step keys are 64 hex digits.
// - Timeouts on every request; GETs, and only GETs, are retried on a network
//   error, a timeout, 429, 502, 503 or 504.
// - Errors are typed (lib/api/types.ts): 401 sign in again (only when the
//   viewer's own token expired; otherwise the server is misconfigured and it
//   is "unavailable", so a server problem never loops a viewer through
//   sign-in), 403 not found, 404 a missing entity (or a team quod-server does
//   not have yet: Quod's numbers are empty), 501 "not available yet", and
//   anything else "unavailable".
// - The event stream reconnects by itself with Last-Event-ID (after `bye`,
//   a dropped connection or a silent one), until the caller aborts.

import { SseDecoder } from "@/lib/live/sse";
import { isSlug } from "@/lib/params";
import { lite } from "@/lib/savings";
import { NoQuodCloudApi } from "./none";
import { ApiForbiddenError, ApiInvalidRequestError, ApiNotAvailableError, ApiUnavailableError } from "./types";
import type {
  Activity,
  CloudApi,
  ConnectSetup,
  Host,
  LiveEvent,
  LiveScope,
  OrgRef,
  Overview,
  Range,
  RepoDetail,
  RepoSummary,
  RunDetail,
  Runner,
  RunSummary,
  SearchIndex,
  StepSummary,
  StepTrust,
  Store,
  Team,
  TeamHealth,
  TestRunDetail,
  TestRunSummary,
  TestRunLite,
  TestStats,
  TestSuite,
} from "./types";

export const PROTOCOL = "quod.v1";
export const PROTOCOL_HEADER = "quod-protocol";

/** How the dashboard reaches quod-server for one of its teams. */
export interface ServerCredentials {
  /** quod-server's id for the team: the path segment of /v1/teams/{team}. */
  team: string;
  /** The bearer token: the viewer's WorkOS access token, or the team's static viewer token (lib/api/credentials.ts). */
  token: string;
  /** "viewer": the viewer's own token, so a 401 means "sign in again". "static": a deploy secret, so a 401 is a misconfiguration. */
  kind: "viewer" | "static";
}

export interface HttpOptions {
  baseUrl: string;
  /** Credentials for a dashboard team (by its slug), or null: quod-server is not set up for that team, and Quod's side is empty. */
  credentials: (team: string) => Promise<ServerCredentials | null>;
  fetch?: typeof fetch;
  /** Per request, headers included. Default 8 s. */
  timeoutMs?: number;
  /** Extra attempts for a GET. Default 2. */
  retries?: number;
  /** Delay before retry `n` (1-based). */
  retryDelayMs?: (n: number) => number;
  /** Delay before the n-th reconnect of the event stream in a row (1-based). */
  reconnectDelayMs?: (n: number) => number;
  /** An event stream that sends nothing (not even its 15 s ping) for this long is reconnected. Default 45 s. */
  streamIdleMs?: number;
  sleep?: (ms: number, signal?: AbortSignal) => Promise<void>;
  now?: () => number;
}

type Method = "GET" | "POST" | "PUT";

const seg = encodeURIComponent;

/** Team, repo and step-key path segments are checked, so nothing can walk the backend path. */
function id(v: string): string {
  if (!isSlug(v)) throw new ApiInvalidRequestError("Invalid path segment");
  return seg(v);
}

/** A run number is a positive integer; anything else never reaches a URL. */
function runSeg(run: number): string {
  if (!Number.isSafeInteger(run) || run < 1 || run > 999_999_999) throw new ApiInvalidRequestError("Invalid run number");
  return String(run);
}

const defaultSleep = (ms: number, signal?: AbortSignal) =>
  new Promise<void>((resolve) => {
    if (signal?.aborted) return resolve();
    const t = setTimeout(resolve, ms);
    signal?.addEventListener("abort", () => (clearTimeout(t), resolve()), { once: true });
  });

/** The `exp` of a JWT, unverified (only to tell an expired viewer token from a server that refuses good ones). */
export function jwtExpiresAt(token: string): number | null {
  const part = token.split(".")[1];
  if (!part) return null;
  try {
    const exp = (JSON.parse(Buffer.from(part, "base64url").toString("utf8")) as { exp?: unknown }).exp;
    return typeof exp === "number" ? exp * 1000 : null;
  } catch {
    return null;
  }
}

// ---------------------------------------------------------------- Shape checks
// quod-server is ours, but a deploy can run another version: a body that is
// not what quod.v1 says is "unavailable", never a crash in a component.

class ShapeError extends Error {}

function obj(v: unknown, what: string): Record<string, unknown> {
  if (!v || typeof v !== "object" || Array.isArray(v)) throw new ShapeError(`${what} is not an object`);
  return v as Record<string, unknown>;
}

function has(v: unknown, what: string, fields: Record<string, "string" | "number" | "boolean" | "object" | "array" | "string|null" | "number|null" | "object|null">) {
  const o = obj(v, what);
  for (const [k, t] of Object.entries(fields)) {
    const x = o[k];
    const ok = t.split("|").some((one) => (one === "null" ? x === null : one === "array" ? Array.isArray(x) : one === "object" ? !!x && typeof x === "object" && !Array.isArray(x) : typeof x === one));
    if (!ok) throw new ShapeError(`${what}.${k} is not ${t}`);
  }
  return o;
}

function list<T>(v: unknown, what: string, each: (x: unknown, what: string) => T): T[] {
  if (!Array.isArray(v)) throw new ShapeError(`${what} is not an array`);
  return v.map((x, i) => each(x, `${what}[${i}]`));
}

const repoSummary = (v: unknown, w: string) => has(v, w, { slug: "string", fullName: "string", replayRate: "number", savedMinutes: "number", lastRunAt: "string|null", pausedSteps: "number" }) as unknown as RepoSummary;
const runSummary = (v: unknown, w: string) =>
  has(v, w, { repoSlug: "string", number: "number", title: "string", branch: "string", sha: "string", status: "string", startedAt: "string", tookSeconds: "number", stepsReplayed: "number", stepsTotal: "number" }) as unknown as RunSummary;
const stepSummary = (v: unknown, w: string) => {
  const o = has(v, w, { repoSlug: "string", key: "string", command: "string", trust: "object", ranSeconds: "number", savedMinutes: "number" });
  has(o.trust, `${w}.trust`, { kind: "string" });
  return o as unknown as StepSummary;
};
const runDetail = (v: unknown, w: string): RunDetail => {
  const o = runSummary(v, w) as unknown as Record<string, unknown>;
  list(o.jobs, `${w}.jobs`, (j, jw) => {
    const job = has(j, jw, { name: "string", steps: "array" });
    list(job.steps, `${jw}.steps`, (s, sw) => has(s, sw, { name: "string", outcome: "string", seconds: "number", why: "object" }));
  });
  return o as unknown as RunDetail;
};

const testRunSummary = (v: unknown, w: string): TestRunSummary => {
  const o = has(v, w, {
    id: "string",
    at: "string",
    repoSlug: "string",
    repository: "string",
    suite: "string",
    kind: "string",
    tool: "string",
    files: "object",
    timing: "object",
    githubUrl: "string|null",
    branch: "string|null",
    sha: "string|null",
    exit: "number|null",
    ledger: "object|null",
    audits: "object|null",
  });
  has(o.files, `${w}.files`, { total: "number", ran: "number", replayed: "number", narrowed: "number" });
  has(o.timing, `${w}.timing`, { wallMs: "number|null", selectMs: "number|null", estPlainMs: "number|null", savedMs: "number|null" });
  if (o.ledger) has(o.ledger, `${w}.ledger`, { found: "boolean", sha: "string|null", branch: "string|null" });
  if (o.audits) has(o.audits, `${w}.audits`, { checked: "number", mismatched: "number" });
  return o as unknown as TestRunSummary;
};

/** A GET /test-history row: a test run cut to what the savings need (lib/savings.ts). */
const testRunLite = (v: unknown, w: string): TestRunLite => {
  const o = has(v, w, { id: "string", at: "string", repoSlug: "string", repository: "string", suite: "string", kind: "string", files: "object", timing: "object" });
  has(o.files, `${w}.files`, { total: "number", ran: "number", replayed: "number" });
  has(o.timing, `${w}.timing`, { wallMs: "number|null", estPlainMs: "number|null", savedMs: "number|null" });
  return lite(o as unknown as TestRunLite);
};

const shapes = {
  testHistory: (v: unknown) => list(has(v, "test-history", { runs: "array" }).runs, "test-history.runs", testRunLite),
  testRuns: (v: unknown) => list(has(v, "test-runs", { runs: "array" }).runs, "test-runs.runs", testRunSummary),
  testRun: (v: unknown): TestRunDetail => {
    const o = testRunSummary(v, "test-run") as unknown as Record<string, unknown>;
    // Legacy fields (the retired background-recording design): a server that stops sending them still parses.
    if (o.auditFiles === undefined) o.auditFiles = [];
    if (o.shadow === undefined) o.shadow = null;
    has(o, "test-run", { ranFiles: "array", auditFiles: "array", shadow: "object|null" });
    list(o.ranFiles, "test-run.ranFiles", (f, fw) => has(f, fw, { file: "string", why: "string", durationMs: "number|null" }));
    list(o.auditFiles, "test-run.auditFiles", (f, fw) => has(f, fw, { file: "string", ok: "boolean" }));
    if (o.shadow) testRunSummary(o.shadow, "test-run.shadow");
    return o as unknown as TestRunDetail;
  },
  testStats: (v: unknown): TestStats => {
    const o = has(v, "test-stats", { asOf: "string", today: "object", week: "object", wrongResults: "number", audits: "object", recent: "array" });
    has(o.today, "test-stats.today", { savedMs: "number", runs: "number" });
    has(o.week, "test-stats.week", { savedMs: "number", runs: "number", filesTotal: "number", filesSkipped: "number", skippedPct: "number|null" });
    has(o.audits, "test-stats.audits", { checked: "number", passed: "number" });
    list(o.recent, "test-stats.recent", testRunSummary);
    return o as unknown as TestStats;
  },
  testSuites: (v: unknown): TestSuite[] =>
    list(has(v, "test-suites", { suites: "array" }).suites, "test-suites.suites", (x, w) => {
      const o = has(x, w, { repoSlug: "string", repository: "string", suite: "string", ledger: "object|null", totals: "object", files: "array" });
      if (o.ledger) has(o.ledger, `${w}.ledger`, { at: "string", sha: "string|null", branch: "string|null" });
      has(o.totals, `${w}.totals`, { files: "number", learning: "number", ready: "number", skipping: "number", always: "number", paused: "number", savedMs: "number" });
      list(o.files, `${w}.files`, (f, fw) =>
        has(f, fw, { file: "string", state: "string", reason: "string|null", lastResult: "string|null", skipped: "number", ran: "number", durationMs: "number|null", savedMs: "number|null" }),
      );
      return o as unknown as TestSuite;
    }),
  health: (v: unknown) => has(v, "health", { wrongReplays: "number", spotChecks: "number", pausedSteps: "number", repos: "number" }) as unknown as TeamHealth,
  overview: (v: unknown) => {
    const o = has(v, "overview", { asOf: "string", totals: "object", days: "array", repos: "array", trust: "object", runners: "array" });
    has(o.totals, "overview.totals", { savedMinutes: "number", stepsReplayed: "number", stepsTotal: "number", costSavedUsd: "number", usdPerMinute: "number", wrongReplays: "number", spotChecks: "number", spotChecksIdentical: "number" });
    list(o.repos, "overview.repos", repoSummary);
    has(o.trust, "overview.trust", { proven: "number", proving: "number", watching: "number", spotChecksPassed: "number", paused: "array" });
    return o as unknown as Overview;
  },
  repos: (v: unknown) => list(has(v, "repos", { repos: "array" }).repos, "repos", repoSummary),
  repo: (v: unknown) => {
    const o = has(v, "repo", { asOf: "string", summary: "object", defaultBranch: "string", latestRun: "object|null", runTimes: "array", steps: "array" });
    repoSummary(o.summary, "repo.summary");
    if (o.latestRun) runDetail(o.latestRun, "repo.latestRun");
    list(o.steps, "repo.steps", stepSummary);
    return o as unknown as RepoDetail;
  },
  runs: (v: unknown) => {
    const o = has(v, "runs", { asOf: "string", runs: "array" });
    list(o.runs, "runs.runs", runSummary);
    return o as unknown as { asOf: string; runs: RunSummary[] };
  },
  run: (v: unknown) => runDetail(v, "run"),
  steps: (v: unknown) => list(has(v, "steps", { steps: "array" }).steps, "steps", stepSummary),
  step: (v: unknown) => {
    const o = stepSummary(v, "step") as unknown as Record<string, unknown>;
    has(o, "step", { asOf: "string", proofsNeeded: "number", history: "array", inputs: "object", divergence: "object|null" });
    return o as unknown as StepTrust;
  },
  runners: (v: unknown) => list(has(v, "runners", { runners: "array" }).runners, "runners", (r, w) => has(r, w, { name: "string", kind: "string", status: "string" }) as unknown as Runner),
  connect: (v: unknown) => has(v, "connect", { setupAction: "string", runnerLabels: "array", installCommand: "string", selfHostedExample: "string" }) as unknown as ConnectSetup,
  searchIndex: (v: unknown) => has(v, "search-index", { repos: "array", runs: "array", steps: "array" }) as unknown as SearchIndex,
  activity: (v: unknown) => {
    const o = has(v, "activity", { asOf: "string", jobs: "array", cursor: "string|null" });
    return { ...(o as unknown as Activity), simulated: false };
  },
  rerun: (v: unknown) => has(v, "rerun", { accepted: "boolean", message: "string" }) as unknown as { accepted: boolean; message: string },
};

/**
 * One point per UTC day of the range, oldest first, as the chart expects:
 * quod-server sends only the days that had steps.
 */
export function fillDays(days: Overview["days"], range: Range, asOf: string): Overview["days"] {
  const n = Number(range.slice(0, -1));
  const end = Date.parse(asOf);
  const today = Number.isFinite(end) ? end : Date.now();
  const by = new Map(days.map((d) => [d.day, d]));
  const out: Overview["days"] = [];
  for (let i = n - 1; i >= 0; i--) {
    const day = new Date(today - i * 86_400_000).toISOString().slice(0, 10);
    out.push(by.get(day) ?? { day, replayedMinutes: 0, ranMinutes: 0 });
  }
  return out;
}

const LIVE_TYPES = new Set(["job.queued", "job.started", "job.finished", "run.started", "run.finished", "step.started", "step.finished", "trust.changed", "stats.updated", "test.run"]);

/** One stream message as a LiveEvent, or null when it is not one (unknown type, bad JSON, missing fields). */
export function parseLiveEvent(type: string | undefined, data: string, sseId: string | undefined): LiveEvent | null {
  if (!type || !LIVE_TYPES.has(type)) return null;
  let v: Record<string, unknown>;
  try {
    v = obj(JSON.parse(data), "event");
  } catch {
    return null;
  }
  const evId = sseId ?? (typeof v.id === "string" ? v.id : undefined);
  if (!evId || typeof v.at !== "string") return null;
  const e: Record<string, unknown> = { ...v, id: evId, type };
  // quod-server writes "" for a step without a key; quod.v1 says null.
  if ((type === "step.started" || type === "step.finished") && e.stepKey === "") e.stepKey = null;
  if (type === "step.finished" && (!e.step || typeof e.step !== "object" || typeof e.repoSlug !== "string" || typeof e.run !== "number")) return null;
  if (type === "trust.changed" && (typeof e.repoSlug !== "string" || typeof e.key !== "string" || !e.trust)) return null;
  if (type.startsWith("job.") && (!e.job || typeof e.job !== "object")) return null;
  if (type.startsWith("run.") && (!e.run || typeof e.run !== "object")) return null;
  if (type === "test.run") {
    try {
      testRunSummary(e.run, "test.run.run");
    } catch {
      return null;
    }
    if (typeof e.repoSlug !== "string") e.repoSlug = (e.run as TestRunSummary).repoSlug;
  }
  return e as unknown as LiveEvent;
}

const NOT_FOUND = Symbol("not found");

export class HttpCloudApi implements CloudApi {
  private readonly f: typeof fetch;
  private readonly timeoutMs: number;
  private readonly retries: number;
  private readonly sleep: (ms: number, signal?: AbortSignal) => Promise<void>;
  private readonly now: () => number;
  /** Teams quod-server has no credentials for: Quod's side is empty. */
  private readonly off = new NoQuodCloudApi();

  constructor(private readonly opts: HttpOptions) {
    this.f = opts.fetch ?? fetch;
    this.timeoutMs = opts.timeoutMs ?? 5_000;
    this.retries = opts.retries ?? 1;
    this.sleep = opts.sleep ?? defaultSleep;
    this.now = opts.now ?? Date.now;
  }

  private url(path: string): URL {
    return new URL(`${this.opts.baseUrl}/v1${path}`);
  }

  private headers(c: ServerCredentials, extra: Record<string, string> = {}): Record<string, string> {
    // Built from nothing: no cookie, no forwarded header, nothing from the browser's request.
    return { [PROTOCOL_HEADER]: PROTOCOL, authorization: `Bearer ${c.token}`, ...extra };
  }

  /** A refusal of our credentials: "sign in again" only when the viewer's own token has expired. */
  private refused(c: ServerCredentials, status: 401 | 403, what: string): Error {
    if (status === 403) return new ApiForbiddenError(`quod-server refused ${what}`, 403);
    const exp = c.kind === "viewer" ? jwtExpiresAt(c.token) : null;
    if (exp !== null && exp <= this.now() + 30_000) return new ApiForbiddenError(`quod-server: the session expired (${what})`, 401);
    return new ApiUnavailableError(`quod-server refused the dashboard's credentials (${what}); check its viewer auth`, 401, "unauthorized");
  }

  private async errorFrom(res: Response): Promise<{ code: string | null; message: string }> {
    try {
      const b = (await res.json()) as { error?: unknown; message?: unknown };
      return { code: typeof b.error === "string" ? b.error : null, message: typeof b.message === "string" ? b.message.slice(0, 300) : "" };
    } catch {
      return { code: null, message: "" };
    }
  }

  /**
   * One call. Returns the parsed body (null for 204), or NOT_FOUND for a 404.
   * GETs are retried on a network error, a timeout, 429, 502, 503 and 504.
   * `strict404`: a 404 is NOT_FOUND only with a quod.v1 error body
   * (`{error, message}`); a bare one (a proxy in front of a server that is
   * down or redeploying) is "unavailable", never "no data".
   */
  private async call(c: ServerCredentials, method: Method, path: string, body?: unknown, opts: { strict404?: boolean } = {}): Promise<unknown> {
    const what = `${method} ${path.split("?")[0]}`;
    const attempts = method === "GET" ? this.retries + 1 : 1;
    let last: Error = new ApiUnavailableError(`quod-server: ${what} failed`);
    for (let n = 1; n <= attempts; n++) {
      if (n > 1) await this.sleep(this.opts.retryDelayMs?.(n - 1) ?? 250 * 3 ** (n - 2));
      let res: Response;
      try {
        res = await this.f(this.url(path), {
          method,
          headers: this.headers(c, { accept: "application/json", ...(body === undefined ? {} : { "content-type": "application/json" }) }),
          body: body === undefined ? undefined : JSON.stringify(body),
          signal: AbortSignal.timeout(this.timeoutMs),
          cache: "no-store",
          credentials: "omit",
          redirect: "error",
        });
      } catch (e) {
        const timeout = e instanceof Error && (e.name === "TimeoutError" || e.name === "AbortError");
        last = new ApiUnavailableError(timeout ? `quod-server did not answer ${what} in ${this.timeoutMs / 1000} s` : `quod-server is unreachable (${what})`);
        // A server that hangs would hang again: a page waits one timeout, not several.
        if (timeout) throw last;
        continue;
      }
      if (res.ok) {
        if (res.status === 204) return null;
        try {
          return await res.json();
        } catch {
          throw new ApiUnavailableError(`quod-server sent a body that is not JSON (${what})`, res.status);
        }
      }
      if (res.status === 404) {
        if (!opts.strict404) return NOT_FOUND;
        if ((await this.errorFrom(res)).code) return NOT_FOUND;
        throw new ApiUnavailableError(`quod-server's ${what} answered 404 without a quod.v1 error (a proxy, or a server starting up)`, 404);
      }
      if (res.status === 401 || res.status === 403) throw this.refused(c, res.status, what);
      const err = await this.errorFrom(res);
      if (res.status === 501) throw new ApiNotAvailableError("Not available yet", err.message);
      last = new ApiUnavailableError(`quod-server answered ${res.status}${err.code ? ` (${err.code})` : ""} to ${what}`, res.status, err.code);
      if (![429, 502, 503, 504].includes(res.status)) throw last;
    }
    throw last;
  }

  /** A call whose body goes through a shape check; a mismatch is "unavailable". */
  private async read<T>(c: ServerCredentials, method: Method, path: string, shape: (v: unknown) => T, body?: unknown, opts: { strict404?: boolean } = {}): Promise<T | typeof NOT_FOUND> {
    const v = await this.call(c, method, path, body, opts);
    if (v === NOT_FOUND) return NOT_FOUND;
    try {
      return shape(v);
    } catch (e) {
      throw new ApiUnavailableError(`quod-server's answer to ${method} ${path.split("?")[0]} is not quod.v1: ${e instanceof ShapeError ? e.message : "bad body"}`);
    }
  }

  /** Credentials and the team's path prefix, or null when quod-server is not set up for this team. */
  private async team(team: string): Promise<{ c: ServerCredentials; t: string } | null> {
    id(team); // the dashboard's slug: checked before anything else
    const c = await this.opts.credentials(team);
    return c ? { c, t: `/teams/${id(c.team)}` } : null;
  }

  // POST /v1/teams/resolve is not used: the dashboard owns team slugs (lib/api/teams.ts, docs/api.md).
  resolveTeams(orgs: OrgRef[]): Promise<Team[]> {
    return this.off.resolveTeams(orgs);
  }

  // GET /v1/teams/:team/health -> TeamHealth
  async getTeamHealth(team: string): Promise<TeamHealth> {
    const s = await this.team(team);
    if (!s) return this.off.getTeamHealth();
    const v = await this.read(s.c, "GET", `${s.t}/health`, shapes.health);
    return v === NOT_FOUND ? this.off.getTeamHealth() : v;
  }

  // GET /v1/teams/:team/overview?range=7d -> Overview
  async getOverview(team: string, range: Range): Promise<Overview> {
    if (!["7d", "30d", "90d"].includes(range)) throw new ApiInvalidRequestError("Invalid range");
    const s = await this.team(team);
    if (!s) return this.off.getOverview(team, range);
    const v = await this.read(s.c, "GET", `${s.t}/overview?range=${range}`, shapes.overview);
    // A team quod-server does not have (yet) has no Quod data: empty, like a team without a server.
    if (v === NOT_FOUND) return { ...(await this.off.getOverview(team, range)), quod: { state: "connected" } };
    return { ...v, days: fillDays(v.days, range, v.asOf), quod: { state: "connected" } };
  }

  // GET /v1/teams/:team/repos -> { repos: RepoSummary[] }
  async listRepos(team: string): Promise<RepoSummary[]> {
    const s = await this.team(team);
    if (!s) return [];
    const v = await this.read(s.c, "GET", `${s.t}/repos`, shapes.repos);
    return v === NOT_FOUND ? [] : v;
  }

  // GET /v1/teams/:team/repos/:repo -> RepoDetail | 404
  async getRepo(team: string, repo: string): Promise<RepoDetail | null> {
    const s = await this.team(team);
    if (!s) return null;
    const v = await this.read(s.c, "GET", `${s.t}/repos/${id(repo)}`, shapes.repo);
    return v === NOT_FOUND ? null : { ...v, quod: { state: "connected" } };
  }

  // GET /v1/teams/:team/runs?repo=&limit= -> { asOf, runs: RunSummary[] } newest first. No paging: `limit` (default 50, at most 500).
  async listRuns(team: string, opts: { repo?: string; limit?: number } = {}): Promise<{ asOf: string; runs: RunSummary[]; quod?: { state: "connected" | "off" } }> {
    const s = await this.team(team);
    if (!s) return this.off.listRuns();
    const q = new URLSearchParams();
    if (opts.repo) q.set("repo", id(opts.repo));
    if (opts.limit) q.set("limit", String(Math.min(Math.max(Math.trunc(opts.limit), 1), 500)));
    const v = await this.read(s.c, "GET", `${s.t}/runs${q.size ? `?${q}` : ""}`, shapes.runs);
    // 404: the team, or the repo, has no Quod runs.
    return v === NOT_FOUND ? { asOf: new Date(this.now()).toISOString(), runs: [], quod: { state: "connected" } } : { ...v, quod: { state: "connected" } };
  }

  // GET /v1/teams/:team/repos/:repo/runs/:number -> RunDetail | 404
  async getRun(team: string, repo: string, run: number): Promise<RunDetail | null> {
    const s = await this.team(team);
    if (!s) return null;
    const v = await this.read(s.c, "GET", `${s.t}/repos/${id(repo)}/runs/${runSeg(run)}`, shapes.run);
    return v === NOT_FOUND ? null : v;
  }

  // GET /v1/teams/:team/steps?repo= -> { steps: StepSummary[] }
  async listSteps(team: string, opts: { repo?: string } = {}): Promise<StepSummary[]> {
    const s = await this.team(team);
    if (!s) return [];
    const v = await this.read(s.c, "GET", `${s.t}/steps${opts.repo ? `?repo=${id(opts.repo)}` : ""}`, shapes.steps);
    return v === NOT_FOUND ? [] : v;
  }

  // GET /v1/teams/:team/repos/:repo/steps/:key -> StepTrust | 404
  async getStepTrust(team: string, repo: string, key: string): Promise<StepTrust | null> {
    const s = await this.team(team);
    if (!s) return null;
    const v = await this.read(s.c, "GET", `${s.t}/repos/${id(repo)}/steps/${id(key)}`, shapes.step);
    return v === NOT_FOUND ? null : v;
  }

  // PUT /v1/teams/:team/repos/:repo/steps/:key/proofs  { need } -> StepTrust (team admins). 501 on today's server.
  async setProofsNeeded(team: string, repo: string, key: string, need: number): Promise<StepTrust> {
    if (!Number.isInteger(need) || need < 2 || need > 10) throw new Error("Proofs needed must be a whole number from 2 to 10.");
    const s = await this.team(team);
    if (!s) return this.off.setProofsNeeded();
    const v = await this.read(s.c, "PUT", `${s.t}/repos/${id(repo)}/steps/${id(key)}/proofs`, shapes.step, { need });
    if (v === NOT_FOUND) throw new ApiForbiddenError("No such step", 403);
    return v;
  }

  // POST /v1/teams/:team/repos/:repo/runs/:number/rerun  { quod: false } -> { accepted, message }. 501 on today's server.
  async requestRerunWithoutQuod(team: string, repo: string, run: number) {
    const s = await this.team(team);
    if (!s) return this.off.requestRerunWithoutQuod();
    const v = await this.read(s.c, "POST", `${s.t}/repos/${id(repo)}/runs/${runSeg(run)}/rerun`, shapes.rerun, { quod: false });
    if (v === NOT_FOUND) return { accepted: false, message: "Quod has no record of this run." };
    return v;
  }

  // GET /v1/teams/:team/runners -> { runners: Runner[] }. 501 on today's server (runners come from GitHub).
  async listRunners(team: string): Promise<Runner[]> {
    const s = await this.team(team);
    if (!s) return [];
    const v = await this.read(s.c, "GET", `${s.t}/runners`, shapes.runners);
    return v === NOT_FOUND ? [] : v;
  }

  // GET /v1/teams/:team/connect -> ConnectSetup. 501 on today's server.
  async getConnectSetup(team: string): Promise<ConnectSetup> {
    const s = await this.team(team);
    if (!s) return this.off.getConnectSetup();
    const v = await this.read(s.c, "GET", `${s.t}/connect`, shapes.connect);
    return v === NOT_FOUND ? this.off.getConnectSetup() : v;
  }

  // PUT /v1/teams/:team/connect  { store, host } -> 204. 501 on today's server.
  async saveConnectChoice(team: string, choice: { store: Store; host: Host }): Promise<void> {
    const s = await this.team(team);
    if (!s) return this.off.saveConnectChoice();
    await this.call(s.c, "PUT", `${s.t}/connect`, { store: choice.store, host: choice.host });
  }

  // GET /v1/teams/:team/search-index -> SearchIndex
  async getSearchIndex(team: string): Promise<SearchIndex> {
    const s = await this.team(team);
    if (!s) return this.off.getSearchIndex();
    const v = await this.read(s.c, "GET", `${s.t}/search-index`, shapes.searchIndex);
    return v === NOT_FOUND ? this.off.getSearchIndex() : v;
  }

  /**
   * A read of the test-run endpoints, which servers before test-level replay
   * do not have: 404 and 501 are null ("not served"), so the UI hides the
   * test sections instead of failing. Other failures are thrown as usual.
   */
  private async optional<T>(c: ServerCredentials, path: string, shape: (v: unknown) => T): Promise<T | null> {
    try {
      // These endpoints are optional (older servers 404 them), so a bare 404 from a proxy must not read as "no data".
      const v = await this.read(c, "GET", path, shape, undefined, { strict404: true });
      return v === NOT_FOUND ? null : v;
    } catch (e) {
      if (e instanceof ApiNotAvailableError) return null;
      throw e;
    }
  }

  // GET /v1/teams/:team/test-runs?repo=&limit= -> { runs: TestRunSummary[] } newest first. limit: default 50, at most 200.
  async listTestRuns(team: string, opts: { repo?: string; limit?: number } = {}): Promise<TestRunSummary[] | null> {
    const s = await this.team(team);
    if (!s) return null;
    const q = new URLSearchParams();
    if (opts.repo) q.set("repo", id(opts.repo));
    if (opts.limit) q.set("limit", String(Math.min(Math.max(Math.trunc(opts.limit), 1), 200)));
    return this.optional(s.c, `${s.t}/test-runs${q.size ? `?${q}` : ""}`, shapes.testRuns);
  }

  // GET /v1/teams/:team/test-runs/:id -> TestRunDetail | 404
  async getTestRun(team: string, runId: string): Promise<TestRunDetail | null> {
    const s = await this.team(team);
    if (!s) return null;
    return this.optional(s.c, `${s.t}/test-runs/${id(runId)}`, shapes.testRun);
  }

  // GET /v1/teams/:team/test-stats -> TestStats
  async getTestStats(team: string): Promise<TestStats | null> {
    const s = await this.team(team);
    if (!s) return null;
    return this.optional(s.c, `${s.t}/test-stats`, shapes.testStats);
  }

  // GET /v1/teams/:team/test-suites -> { suites: TestSuite[] }
  async listTestSuites(team: string): Promise<TestSuite[] | null> {
    const s = await this.team(team);
    if (!s) return null;
    return this.optional(s.c, `${s.t}/test-suites`, shapes.testSuites);
  }

  // GET /v1/teams/:team/test-history?days= -> { runs: TestRunLite[] } newest first (quod-server after cloud/test-history).
  async listTestHistory(team: string, opts: { days: number }): Promise<TestRunLite[] | null> {
    const s = await this.team(team);
    if (!s) return null;
    const days = Math.min(Math.max(Math.trunc(opts.days), 1), 365);
    return this.optional(s.c, `${s.t}/test-history?days=${days}`, shapes.testHistory);
  }

  // GET /v1/teams/:team/activity -> Activity. quod-server has no jobs (GitHub's come from the composite); its `cursor` is the stream position.
  async getActivity(team: string, opts: { repo?: string } = {}): Promise<Activity> {
    const s = await this.team(team);
    if (!s) return this.off.getActivity();
    const v = await this.read(s.c, "GET", `${s.t}/activity${opts.repo ? `?repo=${id(opts.repo)}` : ""}`, shapes.activity);
    return v === NOT_FOUND ? { ...(await this.off.getActivity()), quod: { state: "connected" } } : { ...v, quod: { state: "connected" } };
  }

  /**
   * GET /v1/teams/:team/events?scope=... (text/event-stream). Each message:
   * `id`, `event` (the LiveEvent type), `data` (the LiveEvent JSON).
   * Reconnects with Last-Event-ID after `event: bye`, a dropped or a silent
   * connection, with back-off on failures, until `signal` aborts. Throws only
   * when the viewer is refused (401/403); a team or endpoint the server does
   * not have (404, 501) holds the stream open quietly.
   */
  async *subscribe(team: string, scope: LiveScope, opts: { lastEventId?: string | null; signal: AbortSignal }): AsyncIterable<LiveEvent> {
    const q = new URLSearchParams({ scope: scope.kind });
    if (scope.kind !== "team") q.set("repo", id(scope.repo));
    if (scope.kind === "run") q.set("run", runSeg(scope.run));
    if (scope.kind === "step") q.set("key", id(scope.key));
    const first = await this.team(team);
    if (!first) return yield* this.off.subscribe(team, scope, opts);
    let s = first;
    const { signal } = opts;
    const idleMs = this.opts.streamIdleMs ?? 45_000;
    let last = opts.lastEventId ?? null;
    let failures = 0;
    for (let n = 0; !signal.aborted; n++) {
      // Fresh credentials for each connection: a viewer's access token can expire between rotations.
      if (n > 0) s = (await this.team(team)) ?? s;
      const ac = new AbortController();
      const stop = () => ac.abort();
      signal.addEventListener("abort", stop, { once: true });
      // No byte (not even a ping) for `idleMs`, or no headers in `timeoutMs`: give up on this connection.
      let idle = setTimeout(stop, this.timeoutMs);
      const touch = () => {
        clearTimeout(idle);
        idle = setTimeout(stop, idleMs);
      };
      let quiet = false;
      try {
        const res = await this.f(this.url(`${s.t}/events?${q}`), {
          headers: this.headers(s.c, { accept: "text/event-stream", ...(last ? { "last-event-id": last } : {}) }),
          signal: ac.signal,
          cache: "no-store",
          credentials: "omit",
          redirect: "error",
        });
        if (res.status === 401 || res.status === 403) throw this.refused(s.c, res.status, "the event stream");
        if (res.status === 404 || res.status === 501) {
          await res.body?.cancel().catch(() => undefined);
          quiet = true;
        } else if (!res.ok || !res.body) {
          await res.body?.cancel().catch(() => undefined);
          throw new ApiUnavailableError(`quod-server events: ${res.status}`, res.status);
        } else {
          touch();
          const decoder = new SseDecoder();
          const text = new TextDecoder();
          let bye = false;
          for await (const chunk of res.body as unknown as AsyncIterable<Uint8Array>) {
            touch();
            for (const m of decoder.push(text.decode(chunk, { stream: true }))) {
              if (m.event === "bye") {
                bye = true;
                break;
              }
              const e = parseLiveEvent(m.event, m.data, m.id);
              if (m.id) last = m.id;
              if (e) {
                failures = 0;
                yield e;
              }
            }
            if (bye) break;
          }
          if (bye) failures = 0;
          else failures++;
        }
      } catch (e) {
        if (signal.aborted) return;
        if (e instanceof ApiForbiddenError) throw e;
        if (e instanceof ApiUnavailableError && e.status === 401) throw e;
        failures++;
      } finally {
        clearTimeout(idle);
        signal.removeEventListener("abort", stop);
        ac.abort();
      }
      if (quiet) {
        // Nothing to stream for this team yet; look again in a minute.
        await this.sleep(60_000, signal);
        continue;
      }
      // A planned close (bye) reconnects at once, but never in a tight loop; failures back off.
      await this.sleep(failures > 0 ? (this.opts.reconnectDelayMs?.(failures) ?? Math.min(30_000, 1000 * 2 ** (failures - 1))) : 250, signal);
    }
  }
}

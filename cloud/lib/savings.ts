// What Quod saves a team in CI time and money, from its test runs. Pure (no
// React, no fetching): the Overview's savings panel computes it on the
// server for the first paint and again in the browser as live test runs
// arrive, so both always agree.
//
// The method, in short (SAVINGS_METHOD says it in full, for the tooltip):
// - Each check run (kind "critical") is compared with what the same suite
//   takes when every test file runs. That plain time comes from recorded
//   full runs of the suite (usually the background recording, which runs
//   every file), scaled to the check's file count; when there is none, from
//   the check's own time plus the skipped files' recorded times.
// - Time saved is the plain time minus the check's own time.
// - Money is runner minutes saved times a per-minute rate (GitHub's
//   published Linux 2-core rate unless the team set its own), minus the
//   runner time the background recordings spent running tests.
// Setup (checkout, install) is not in any of it: test runs do not report it.

import type { TestRunSummary } from "@/lib/api/types";

/** The fields of a test run this needs: what GET /test-history sends, and a subset of every TestRunSummary. */
export type TestRunLite = Pick<TestRunSummary, "id" | "at" | "repoSlug" | "repository" | "suite" | "kind" | "event" | "branch" | "runId" | "sha"> & {
  files: Pick<TestRunSummary["files"], "total" | "ran" | "replayed">;
  timing: Pick<TestRunSummary["timing"], "wallMs" | "estPlainMs" | "savedMs">;
};

/** GitHub's published per-minute price for a standard Linux 2-core hosted runner (2026). */
export const DEFAULT_USD_PER_MINUTE = 0.006;
export const DEFAULT_RATE_NOTE = "GitHub's published price for a Linux 2-core runner";

export const SAVINGS_METHOD =
  "Estimates. For each check, Quod compares its test time with what the same suite takes when every test file runs: recorded full runs of that suite (usually the background recording), scaled to the number of files. Without one, it adds the skipped files' recorded times to the check's own time, as if they had run one after another. Money is the runner minutes saved times your per-minute rate, minus the time background recordings spent running tests. Checkout and install time are not counted.";

export type RunClass = "pr" | "branch" | "default";
export const RUN_CLASSES: RunClass[] = ["pr", "branch", "default"];
export const CLASS_LABEL: Record<RunClass, string> = { pr: "Pull requests", branch: "Feature branches", default: "Default branch" };
export const CLASS_HELP: Record<RunClass, string> = {
  pr: "Runs for pull requests.",
  branch: "Pushes to branches other than the default branch.",
  default: "Pushes to the repository's default branch.",
};

/**
 * Pull request, a push to a feature branch, or a push to the default branch.
 * `defaultBranch`: the repository's, when known (GitHub); otherwise main and master count.
 */
export function classify(r: Pick<TestRunLite, "event" | "branch">, defaultBranch?: string | null): RunClass {
  const ev = (r.event ?? "").toLowerCase();
  const br = (r.branch ?? "").replace(/^refs\/heads\//, "");
  if (ev.startsWith("pull_request") || ev === "merge_group" || br.startsWith("refs/pull/") || br.startsWith("gh-readonly-queue/")) return "pr";
  if (defaultBranch ? br === defaultBranch : br === "main" || br === "master") return "default";
  return "branch";
}

export type Method = "full-run" | "file-times";

/** One check run, priced. */
export interface CheckEstimate {
  id: string;
  at: string;
  repoSlug: string;
  repository: string;
  suite: string;
  cls: RunClass;
  /** The check's own test time. */
  actualMs: number;
  /** What it would have taken with every file run (estimate). */
  plainMs: number;
  /** plainMs - actualMs, never below 0. */
  savedMs: number;
  method: Method;
  filesSkipped: number;
  filesTotal: number;
}

/** One CI run (a GitHub workflow run): its suites added up. */
export interface CiRunPoint {
  key: string;
  at: string;
  cls: RunClass;
  repository: string;
  withMs: number;
  withoutMs: number;
}

export interface ClassSummary {
  ciRuns: number;
  /** Medians per CI run. */
  typicalWithMs: number | null;
  typicalWithoutMs: number | null;
}

export interface DayPoint {
  /** YYYY-MM-DD, UTC. */
  day: string;
  /** Median per CI run that day, per class and for all; null: no runs. */
  with: Record<RunClass | "all", number | null>;
  without: Record<RunClass | "all", number | null>;
  /** Net money saved up to and including this day, from the range's start. */
  cumulativeUsd: number;
}

export interface SavingsModel {
  usdPerMinute: number;
  /** Check runs priced, in the range. */
  checks: number;
  /** CI runs those belong to. */
  ciRuns: number;
  /** Wall time not waited for on checks (estimate). */
  savedMs: number;
  /** Test time the background recordings spent (extra runner time Quod adds). */
  recordingMs: number;
  /** savedMs - recordingMs: runner time saved overall. May be negative. */
  netMs: number;
  usd: { saved: number; recording: number; net: number };
  files: { skipped: number; total: number };
  /** Skipped / total, 0..1, null without files. */
  unchangedShare: number | null;
  methods: Record<Method, number>;
  /** Checks that report no time at all (counted in files, not in time). */
  unmeasured: number;
  byClass: Record<RunClass, ClassSummary>;
  days: DayPoint[];
  points: CiRunPoint[];
  /** The oldest test run in the data (any kind): when Quod started on this team, as far as the data goes. */
  firstAt: string | null;
  /** The newest check, for "latest" lines. */
  latest: CheckEstimate | null;
}

const DAY = 86_400_000;
const suiteKey = (r: Pick<TestRunLite, "repository" | "suite">) => `${r.repository.toLowerCase()}\u0000${r.suite}`;

export function median(xs: number[]): number | null {
  if (!xs.length) return null;
  const s = [...xs].sort((a, b) => a - b);
  const m = Math.floor(s.length / 2);
  return s.length % 2 ? s[m] : (s[m - 1] + s[m]) / 2;
}

const finite = (v: number | null | undefined): v is number => typeof v === "number" && Number.isFinite(v) && v >= 0;

interface FullRun {
  at: number;
  wallMs: number;
  total: number;
}

/** Runs where every test file ran, with a wall time: the suite's plain time. */
function fullRuns(runs: TestRunLite[]): Map<string, FullRun[]> {
  const out = new Map<string, FullRun[]>();
  for (const r of runs) {
    if (r.files.total <= 0 || r.files.replayed > 0 || !finite(r.timing.wallMs) || r.timing.wallMs <= 0) continue;
    const k = suiteKey(r);
    out.set(k, [...(out.get(k) ?? []), { at: Date.parse(r.at), wallMs: r.timing.wallMs, total: r.files.total }]);
  }
  return out;
}

/** The plain time of one check: up to 5 recorded full runs nearest in time, scaled to its file count; else its own time plus the skipped files' times. */
export function plainTime(r: TestRunLite, full: Map<string, FullRun[]>): { actualMs: number; plainMs: number; method: Method } | null {
  const wall = finite(r.timing.wallMs) ? r.timing.wallMs : null;
  const saved = finite(r.timing.savedMs) ? r.timing.savedMs : null;
  const est = finite(r.timing.estPlainMs) ? r.timing.estPlainMs : null;
  // The check's own time: its wall time, else (not reported) the ran files' recorded times.
  const actualMs = wall ?? (est !== null ? Math.max(0, est - (saved ?? 0)) : null);
  if (actualMs === null) return null;
  if (r.files.replayed === 0) return { actualMs, plainMs: actualMs, method: "full-run" };
  const at = Date.parse(r.at);
  const cands = (full.get(suiteKey(r)) ?? [])
    .map((f) => ({ f, d: Math.abs(f.at - at) }))
    .sort((a, b) => a.d - b.d)
    .slice(0, 5)
    .map(({ f }) => (f.wallMs * r.files.total) / f.total);
  const fromFull = median(cands);
  if (fromFull !== null) return { actualMs, plainMs: Math.max(actualMs, fromFull), method: "full-run" };
  if (saved !== null) return { actualMs, plainMs: actualMs + saved, method: "file-times" };
  return null;
}

const dayOf = (iso: string) => new Date(iso).toISOString().slice(0, 10);

/** The UTC days from `from` to `to` (both included), oldest first. */
export function daysBetween(from: number, to: number): string[] {
  const start = Date.UTC(new Date(from).getUTCFullYear(), new Date(from).getUTCMonth(), new Date(from).getUTCDate());
  const out: string[] = [];
  for (let t = start; t <= to; t += DAY) out.push(new Date(t).toISOString().slice(0, 10));
  return out;
}

const emptyByClass = (): Record<RunClass | "all", number | null> => ({ pr: null, branch: null, default: null, all: null });

/**
 * The savings of the runs in [from, to]. `runs` may reach further back:
 * older full runs still give a suite its plain time. `defaultBranches`:
 * lower-case owner/name to the repository's default branch, when known.
 */
export function savingsModel(
  runs: TestRunLite[],
  opts: { from: number; to: number; usdPerMinute: number; defaultBranches?: Record<string, string | null | undefined> },
): SavingsModel {
  const { from, to, usdPerMinute } = opts;
  const rate = finite(usdPerMinute) ? usdPerMinute : DEFAULT_USD_PER_MINUTE;
  const full = fullRuns(runs);
  const inRange = runs.filter((r) => {
    const t = Date.parse(r.at);
    return t >= from && t <= to;
  });
  const defaultOf = (repo: string) => opts.defaultBranches?.[repo.toLowerCase()] ?? null;

  const checks: CheckEstimate[] = [];
  const methods: Record<Method, number> = { "full-run": 0, "file-times": 0 };
  let unmeasured = 0;
  let filesSkipped = 0;
  let filesTotal = 0;
  let recordingMs = 0;
  for (const r of inRange) {
    if (r.kind === "shadow") {
      const w = finite(r.timing.wallMs) ? r.timing.wallMs : finite(r.timing.estPlainMs) ? r.timing.estPlainMs : 0;
      recordingMs += w;
      continue;
    }
    if (r.kind !== "critical") continue;
    filesSkipped += Math.min(r.files.replayed, r.files.total);
    filesTotal += r.files.total;
    const p = plainTime(r, full);
    if (!p) {
      unmeasured++;
      continue;
    }
    methods[p.method]++;
    checks.push({
      id: r.id,
      at: r.at,
      repoSlug: r.repoSlug,
      repository: r.repository,
      suite: r.suite,
      cls: classify(r, defaultOf(r.repository)),
      actualMs: p.actualMs,
      plainMs: p.plainMs,
      savedMs: Math.max(0, p.plainMs - p.actualMs),
      method: p.method,
      filesSkipped: Math.min(r.files.replayed, r.files.total),
      filesTotal: r.files.total,
    });
  }

  // CI runs: a workflow run's suites added up (by GitHub run id, else commit and class).
  const byRun = new Map<string, CiRunPoint>();
  const runOf = new Map(inRange.map((r) => [r.id, r]));
  for (const c of checks) {
    const r = runOf.get(c.id)!;
    const key = `${c.repository.toLowerCase()}\u0000${r.runId ?? `${r.sha ?? c.id}\u0000${c.cls}`}`;
    const p = byRun.get(key);
    if (p) {
      p.withMs += c.actualMs;
      p.withoutMs += c.plainMs;
      if (c.at < p.at) p.at = c.at;
    } else byRun.set(key, { key, at: c.at, cls: c.cls, repository: c.repository, withMs: c.actualMs, withoutMs: c.plainMs });
  }
  const points = [...byRun.values()].sort((a, b) => a.at.localeCompare(b.at));

  const byClass = Object.fromEntries(
    RUN_CLASSES.map((k) => {
      const own = points.filter((p) => p.cls === k);
      return [k, { ciRuns: own.length, typicalWithMs: median(own.map((p) => p.withMs)), typicalWithoutMs: median(own.map((p) => p.withoutMs)) }];
    }),
  ) as Record<RunClass, ClassSummary>;

  // Per day: the median CI run, per class and for all; and money saved so far.
  const savedByDay = new Map<string, number>();
  for (const c of checks) savedByDay.set(dayOf(c.at), (savedByDay.get(dayOf(c.at)) ?? 0) + c.savedMs);
  for (const r of inRange)
    if (r.kind === "shadow") savedByDay.set(dayOf(r.at), (savedByDay.get(dayOf(r.at)) ?? 0) - (finite(r.timing.wallMs) ? r.timing.wallMs : finite(r.timing.estPlainMs) ? r.timing.estPlainMs : 0));
  let running = 0;
  const days: DayPoint[] = daysBetween(from, to).map((day) => {
    const own = points.filter((p) => dayOf(p.at) === day);
    const w = emptyByClass();
    const wo = emptyByClass();
    for (const k of [...RUN_CLASSES, "all"] as const) {
      const sel = k === "all" ? own : own.filter((p) => p.cls === k);
      w[k] = median(sel.map((p) => p.withMs));
      wo[k] = median(sel.map((p) => p.withoutMs));
    }
    running += ((savedByDay.get(day) ?? 0) / 60_000) * rate;
    return { day, with: w, without: wo, cumulativeUsd: running };
  });

  const savedMs = checks.reduce((a, c) => a + c.savedMs, 0);
  const netMs = savedMs - recordingMs;
  const firstAt = runs.reduce<string | null>((a, r) => (a === null || r.at < a ? r.at : a), null);
  const latest = checks.reduce<CheckEstimate | null>((a, c) => (!a || c.at > a.at ? c : a), null);
  const usd = (ms: number) => (ms / 60_000) * rate;
  return {
    usdPerMinute: rate,
    checks: checks.length + unmeasured,
    ciRuns: points.length,
    savedMs,
    recordingMs,
    netMs,
    usd: { saved: usd(savedMs), recording: usd(recordingMs), net: usd(netMs) },
    files: { skipped: filesSkipped, total: filesTotal },
    unchangedShare: filesTotal > 0 ? filesSkipped / filesTotal : null,
    methods,
    unmeasured,
    byClass,
    days,
    points,
    firstAt,
    latest,
  };
}

/** Adds (or replaces, by id) live runs to the server's list, newest first. */
export function mergeRuns(base: TestRunLite[], extra: TestRunLite[]): TestRunLite[] {
  if (!extra.length) return base;
  const ids = new Set(extra.map((r) => r.id));
  return [...extra, ...base.filter((r) => !ids.has(r.id))].sort((a, b) => b.at.localeCompare(a.at));
}

/** A TestRunSummary (or a /test-history row) cut to what the savings need, so a page sends the browser no more than that. */
export function lite(r: TestRunLite): TestRunLite {
  return {
    id: r.id,
    at: r.at,
    repoSlug: r.repoSlug,
    repository: r.repository,
    suite: r.suite,
    kind: r.kind,
    event: r.event ?? null,
    branch: r.branch ?? null,
    runId: r.runId ?? null,
    sha: r.sha ?? null,
    files: { total: r.files.total, ran: r.files.ran, replayed: r.files.replayed },
    timing: { wallMs: r.timing.wallMs ?? null, estPlainMs: r.timing.estPlainMs ?? null, savedMs: r.timing.savedMs ?? null },
  };
}

// ---------------------------------------------------------------- Words

/** "$12.48", "$0.42", "<$0.01"; negative "−$0.42". */
export function formatMoney(usd: number): string {
  if (!Number.isFinite(usd)) return "n/a";
  const a = Math.abs(usd);
  const sign = usd < 0 ? "−" : "";
  if (a > 0 && a < 0.005) return `${sign}<$0.01`;
  return `${sign}$${a >= 1000 ? Math.round(a).toLocaleString("en-US") : a.toFixed(2)}`;
}

/** "41 s", "12 min", "3 h 12 min", "26 h". Whole units, for headline numbers. */
export function formatSpan(ms: number): string {
  if (!Number.isFinite(ms)) return "n/a";
  const s = Math.abs(ms) / 1000;
  const sign = ms < 0 ? "−" : "";
  if (s < 90) return `${sign}${s < 10 ? s.toFixed(1).replace(/\.0$/, "") : Math.round(s)} s`;
  const m = Math.round(s / 60);
  if (m < 60) return `${sign}${m} min`;
  const h = Math.floor(m / 60);
  const r = m % 60;
  return h >= 24 || r === 0 ? `${sign}${h} h` : `${sign}${h} h ${r} min`;
}

/** "$0.006 per minute" */
export function rateText(usdPerMinute: number): string {
  return `$${usdPerMinute.toFixed(usdPerMinute < 0.01 ? 4 : 3).replace(/0+$/, "").replace(/\.$/, "")} per minute`;
}

/** "-85%" style change from without to with. */
export function changeText(withMs: number | null, withoutMs: number | null): string | null {
  if (withMs === null || withoutMs === null || withoutMs <= 0) return null;
  const pct = Math.round(((withMs - withoutMs) / withoutMs) * 100);
  return pct === 0 ? "same" : `${pct > 0 ? "+" : "−"}${Math.abs(pct)}%`;
}

/** The validated rate a team may set: more than 0, at most $1 a minute. */
export function parseRate(v: unknown): number | null {
  const n = typeof v === "number" ? v : typeof v === "string" ? Number(v.trim().replace(/^\$/, "")) : NaN;
  if (!Number.isFinite(n) || n <= 0 || n > 1) return null;
  const r = Math.round(n * 1e6) / 1e6;
  // A rate that rounds to 0 would fail the database's > 0 check: reject it here with the form's own message.
  return r > 0 ? r : null;
}

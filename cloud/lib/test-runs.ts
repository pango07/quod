// Words and numbers for test runs (test-level replay): one place, so the
// Overview card, the Tests tab and a run's page say the same thing. Plain
// language, as on the Trust pages: a skipped file is "skipped (unchanged)",
// never "replayed". Pure; shared with tests.

import type { DayMinutes, TestRunDetail, TestRunSummary, TestStats } from "@/lib/api/types";
import { formatCount } from "@/lib/format";

/**
 * A time given in milliseconds: "0 s" (none at all), "<0.1 s", "0.4 s", "79 s", "12 min", "3.4 h".
 * Under a second always has one decimal: a real time is never rounded to "0 s".
 */
export function formatSavedMs(ms: number | null | undefined): string {
  if (ms === null || ms === undefined || !Number.isFinite(ms) || ms < 0) return "n/a";
  const s = ms / 1000;
  if (s > 0 && s < 0.05) return "<0.1 s";
  if (s > 0 && s < 9.95) return `${s.toFixed(1).replace(/\.0$/, "")} s`;
  if (s < 90) return `${Math.round(s)} s`;
  const min = s / 60;
  if (min < 90) return `${Math.round(min)} min`;
  const h = min / 60;
  return `${h < 10 ? h.toFixed(1).replace(/\.0$/, "") : Math.round(h)} h`;
}

/**
 * Time saved, which is an estimate (the skipped files' recorded times):
 * "≈ 5.7 s". Exactly nothing is "0 s"; unknown is "n/a". Shown next to
 * SAVED_EXPLAINED (a tooltip or caption) wherever it appears.
 */
export function formatSaved(ms: number | null | undefined): string {
  const t = formatSavedMs(ms);
  return t === "n/a" || t === "0 s" ? t : `≈ ${t}`;
}

const plural = (n: number, one: string, many = `${one}s`) => `${formatCount(n)} ${n === 1 ? one : many}`;

/** The suite as shown: the vitest project dir, or the repository for the repo root ("."). */
export function suiteLabel(r: Pick<TestRunSummary, "suite" | "repository">): string {
  const s = r.suite.trim();
  return s === "" || s === "." || s === "./" ? r.repository : s;
}

/** "vitest · packages/backend · 184 test files: 171 skipped (unchanged), 13 ran, saved 79 s" */
export function testRunHeadline(r: Pick<TestRunSummary, "tool" | "suite" | "repository" | "files" | "timing">): string {
  const { total, replayed, ran } = r.files;
  const saved = r.timing.savedMs === null ? "" : `, saved ${formatSaved(r.timing.savedMs)}`;
  return `${r.tool} · ${suiteLabel(r)} · ${plural(total, "test file")}: ${formatCount(replayed)} skipped (unchanged), ${formatCount(ran)} ran${saved}`;
}

/** What "saved" means, for the caption and the tooltip. */
export const SAVED_EXPLAINED =
  "Saved is the time the skipped test files took when they were last recorded, added up across all workers. Tests run in parallel, so the wait you actually saved can be shorter.";

// ---------------------------------------------------------------- What a run was
//
// quod-action v2 records inside the test job itself (the "second witness"
// design): a file's recording is reused only after a second, independent run
// of it agrees. Those runs reach Quod Cloud as kind "shadow", the name the
// retired design gave its separate background recording job, because both
// record. Quod's older setup paired every commit's check (kind "critical")
// with such a background job. So a "shadow" report is told apart by its
// neighbours: one with a "critical" report of the same repository, suite and
// commit is a legacy background recording; one without is the developer's
// own test job, recorded inline.

/** check: a test job that recorded nothing. recorded: a test job that recorded inline (v2). legacy-recording: the retired background job. */
export type RunRole = "check" | "recorded" | "legacy-recording";

/** What pairing reads; a run without its commit (or repository) is never a legacy recording. */
type PairFields = Pick<TestRunSummary, "id" | "kind"> & Partial<Pick<TestRunSummary, "repository" | "suite" | "sha">>;

const normSuite = (s: string) => {
  const t = s.trim().replace(/^\.\/+/, "").replace(/\/+$/, "");
  return t === "" || t === "." ? "." : t;
};
const commitKey = (r: PairFields) => (r.sha && r.repository ? `${r.repository.toLowerCase()}\u0000${normSuite(r.suite ?? ".")}\u0000${r.sha}` : null);

/** The ids of legacy background recordings among `runs`: "shadow" reports with a "critical" report of the same repository, suite and commit. */
export function legacyRecordingIds(runs: readonly PairFields[]): Set<string> {
  const checked = new Set(runs.filter((r) => r.kind === "critical").map(commitKey));
  checked.delete(null);
  return new Set(runs.filter((r) => r.kind === "shadow" && commitKey(r) !== null && checked.has(commitKey(r))).map((r) => r.id));
}

/** A run's role; `legacy` is legacyRecordingIds of the runs it came with. */
export function runRole(r: Pick<TestRunSummary, "id" | "kind">, legacy: ReadonlySet<string>): RunRole {
  if (r.kind === "critical") return "check";
  return legacy.has(r.id) ? "legacy-recording" : "recorded";
}

/** The runs a developer waited for: every run but the legacy background recordings. */
export function checkRuns<T extends PairFields>(runs: readonly T[]): T[] {
  const legacy = legacyRecordingIds(runs);
  return runs.filter((r) => !legacy.has(r.id));
}

export function roleLabel(role: RunRole): string {
  if (role === "legacy-recording") return "Legacy background recording";
  return role === "recorded" ? "Your tests · recorded" : "Your tests";
}

export const ROLE_EXPLAINED: Record<RunRole, string> = {
  check: "The test job your push or pull request waits for. It reused the results of test files whose code did not change, and recorded nothing new.",
  recorded:
    "The test job your push or pull request waits for. It reused the results of test files whose code did not change, and recorded the files it ran, in the same job. A new recording is reused only after a second run of that file agrees with it.",
  "legacy-recording":
    "Quod's older setup ran a separate background job for each commit to record results and re-run a few skipped files. Current versions record inside your test job instead. Kept here so older runs still read correctly.",
};

export const roleTone = (role: RunRole): "gray" | "blue" => (role === "legacy-recording" ? "gray" : "blue");

/** What a recorded run means for its files, in one paragraph. */
export const RECORDING_EXPLAINED =
  "The files that ran were recorded in this job, so recording adds a little time to it. Quod reuses a new recording only after a second, independent run of the file gives the same results. Until then the file runs again, and a file that disagrees keeps running until runs agree.";

/** "Tests passed", "Tests failed (exit code 1)", or "Result not reported". */
export function exitLabel(exit: number | null): { text: string; tone: "green" | "red" | "dim" } {
  if (exit === null) return { text: "Result not reported", tone: "dim" };
  return exit === 0 ? { text: "Tests passed", tone: "green" } : { text: `Tests failed (exit code ${exit})`, tone: "red" };
}

/** The spot-checks of a run, in one sentence; null when there were none. */
export function spotCheckSentence(r: Pick<TestRunDetail, "audits" | "auditFiles">): string | null {
  const files = r.auditFiles ?? [];
  const checked = files.length || r.audits?.checked || 0;
  const bad = files.length ? files.filter((f) => !f.ok).length : (r.audits?.mismatched ?? 0);
  if (!checked) return null;
  const head = checked === 1 ? "1 skipped file was run anyway, to check its reused result" : `${formatCount(checked)} skipped files were run anyway, to check their reused results`;
  if (bad === 0) return `${head}: ${checked === 1 ? "it matched" : "all matched"}.`;
  if (checked === 1) return `${head}: it gave a different result.`;
  return `${head}: ${formatCount(bad)} did not match.`;
}

/** Does a run's page show a spot-check section? Only when it had spot-checks: they run in the same job now, so there is nothing to wait for. */
export function showSpotChecks(r: Pick<TestRunDetail, "audits" | "auditFiles">): boolean {
  return spotCheckSentence(r) !== null;
}

/**
 * Spot-checks over the 7 days before now. Counted from the runs that ran
 * them ("shadow" reports: inline recordings, and legacy background
 * recordings); a legacy check that carries its recording's numbers is not
 * counted again.
 */
export function spotChecksWithinWeek(runs: Pick<TestRunSummary, "kind" | "at" | "audits">[], now: number): { checked: number; mismatched: number } {
  let checked = 0;
  let mismatched = 0;
  for (const r of runs) {
    const at = Date.parse(r.at);
    if (r.kind !== "shadow" || !(at <= now && now - at <= WEEK_MS)) continue;
    checked += r.audits?.checked ?? 0;
    mismatched += r.audits?.mismatched ?? 0;
  }
  return { checked, mismatched };
}

/** Which earlier recording a run reused, in one sentence. */
export function ledgerSentence(l: TestRunSummary["ledger"]): string {
  if (!l) return "Not reported for this run.";
  if (!l.found) return "No earlier recording was found, so every test file ran. This run's results are recorded for next time.";
  const sha = l.sha ? l.sha.slice(0, 7) : null;
  if (l.branch && sha) return `Results reused from the recording of ${l.branch} at ${sha}.`;
  if (sha) return `Results reused from the recording at ${sha}.`;
  if (l.branch) return `Results reused from the latest recording of ${l.branch}.`;
  return "Results reused from an earlier recording.";
}

/** Share of test files skipped this week, from the counts: "93%", or null when there were none. */
export function weekSkipped(s: TestStats): string | null {
  const { filesTotal, filesSkipped } = s.week;
  if (filesTotal > 0) return `${Math.round((filesSkipped / filesTotal) * 100)}%`;
  return null;
}

/**
 * The Overview's test numbers, with the runs' own numbers where the server's
 * miss them. quod-server's /test-stats adds up only kind "critical" runs, so
 * a team on inline recording (kind "shadow", no "critical" partner) would
 * see no time saved and no recent runs. When `runs` has such runs, today,
 * the week and the recent runs come from `runs` (the newest 200) instead;
 * spot-checks and wrong results stay the server's (it counts them from every
 * "shadow" report already).
 */
export function statsWithRuns(stats: TestStats, runs: TestRunSummary[] | null | undefined, now: number): TestStats {
  if (!runs?.length) return stats;
  const legacy = legacyRecordingIds(runs);
  if (!runs.some((r) => runRole(r, legacy) === "recorded")) return stats;
  const checks = runs.filter((r) => !legacy.has(r.id)).sort((a, b) => Date.parse(b.at) - Date.parse(a.at));
  const midnight = Date.UTC(new Date(now).getUTCFullYear(), new Date(now).getUTCMonth(), new Date(now).getUTCDate());
  const since = (from: number) => checks.filter((r) => Date.parse(r.at) >= from && Date.parse(r.at) <= now);
  const today = since(midnight);
  const week = since(now - WEEK_MS);
  const sum = (xs: TestRunSummary[], f: (r: TestRunSummary) => number) => xs.reduce((a, r) => a + f(r), 0);
  const filesTotal = sum(week, (r) => r.files.total);
  const filesSkipped = sum(week, (r) => Math.min(r.files.replayed, r.files.total));
  return {
    ...stats,
    today: { savedMs: sum(today, (r) => r.timing.savedMs ?? 0), runs: today.length },
    week: { savedMs: sum(week, (r) => r.timing.savedMs ?? 0), runs: week.length, filesTotal, filesSkipped, skippedPct: filesTotal > 0 ? (filesSkipped / filesTotal) * 100 : null },
    recent: checks.slice(0, 10),
  };
}

/** Does the Overview card have anything to say? A team that never ran tests through Quod sees nothing. */
export function hasTestActivity(s: TestStats | null): s is TestStats {
  return !!s && (s.recent.length > 0 || s.week.runs > 0 || s.today.runs > 0 || s.audits.checked > 0);
}

/**
 * Should a run's page refresh for a `test.run` event? Its own id (a re-sent
 * report), or a "shadow" report of the same commit and suite (for a legacy
 * check, its background recording, whose spot-checks are merged into it).
 */
export function concernsTestRun(e: Pick<TestRunSummary, "id" | "kind" | "repoSlug" | "sha" | "suite">, page: Pick<TestRunSummary, "id" | "repoSlug" | "sha" | "suite">): boolean {
  if (e.id === page.id) return true;
  return e.kind === "shadow" && !!page.sha && e.sha === page.sha && e.suite === page.suite && e.repoSlug.toLowerCase() === page.repoSlug.toLowerCase();
}

/** The page of a test run: under its repository, like a CI run. */
export const testRunHref = (team: string, r: Pick<TestRunSummary, "repoSlug" | "id">) => `/${team}/${r.repoSlug}/tests/${encodeURIComponent(r.id)}`;

/** "% of test files skipped" is over this many of the newest check runs. */
export const LAST_CHECKS = 20;

/** The share of test files skipped over a set of check runs: the files, added up. */
export interface SkippedShare {
  /** Test files skipped (replayed) and in all, summed over the checks. */
  replayed: number;
  total: number;
  /** Check runs counted (at most LAST_CHECKS). */
  checks: number;
  /** replayed / total, 0-100; null when there are no files. */
  pct: number | null;
}

/**
 * The share of test files skipped over the newest `n` check runs (the test
 * jobs developers waited for; legacy background recordings never count): sum(replayed) /
 * sum(total), so a small one-suite run or a first run after a rename moves
 * it by its size, not as one vote of n. Order-independent: the runs are
 * sorted newest first here (ties by id).
 */
export function skippedShare(runs: (Pick<TestRunSummary, "id" | "at" | "kind" | "files"> & PairFields)[], n = LAST_CHECKS): SkippedShare {
  const checks = checkRuns(runs)
    .filter((r) => r.files.total > 0)
    .sort((a, b) => Date.parse(b.at) - Date.parse(a.at) || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0))
    .slice(0, n);
  const replayed = checks.reduce((a, r) => a + Math.min(r.files.replayed, r.files.total), 0);
  const total = checks.reduce((a, r) => a + r.files.total, 0);
  return { replayed, total, checks: checks.length, pct: total > 0 ? (replayed / total) * 100 : null };
}

/** "86%", or "n/a" without checks. */
export const shareText = (s: Pick<SkippedShare, "pct">) => (s.pct === null ? "n/a" : `${Math.round(s.pct)}%`);

/** The tooltip: "1,234 of 1,450 files across 20 checks". */
export function shareTitle(s: SkippedShare): string {
  if (s.checks === 0) return "No check runs yet";
  return `${formatCount(s.replayed)} of ${formatCount(s.total)} files across ${plural(s.checks, "check")}`;
}

/** A repository's test-level numbers for the Overview's Repositories card. */
export interface RepoTests {
  /** Test files skipped over the repository's last LAST_CHECKS check runs (skippedShare). */
  skipped: SkippedShare;
  /** savedMs of its check runs in the last 7 days. */
  savedMs: number;
  /** Check runs in the last 7 days. */
  runs: number;
  /** The newest run (of any kind and age). */
  last: Pick<TestRunSummary, "id" | "repoSlug" | "at" | "exit"> | null;
}

const WEEK_MS = 7 * 24 * 60 * 60 * 1000;

/**
 * Test runs added up per repository, keyed by lower-case full name
 * (owner/name), for matching to the repo list by full name, never by slug.
 */
export function testsByRepo(runs: TestRunSummary[], now: number): Record<string, RepoTests> {
  const byRepo = new Map<string, TestRunSummary[]>();
  for (const r of runs) {
    const key = r.repository.toLowerCase();
    byRepo.set(key, [...(byRepo.get(key) ?? []), r]);
  }
  const out: Record<string, RepoTests> = {};
  const legacy = legacyRecordingIds(runs);
  for (const [key, own] of byRepo) {
    let last: RepoTests["last"] = null;
    let savedMs = 0;
    let week = 0;
    for (const r of own) {
      const at = Date.parse(r.at);
      if (!last || at > Date.parse(last.at)) last = { id: r.id, repoSlug: r.repoSlug, at: r.at, exit: r.exit };
      if (legacy.has(r.id) || !(at <= now && now - at <= WEEK_MS)) continue;
      savedMs += r.timing.savedMs ?? 0;
      week++;
    }
    out[key] = { skipped: skippedShare(own), savedMs, runs: week, last };
  }
  return out;
}

/** "1,234 of 1,450 files, last 20 checks", or "No checks yet". */
export function repoSkippedNote(s: SkippedShare): string {
  return s.checks > 0 ? `${formatCount(s.replayed)} of ${formatCount(s.total)} files, last ${plural(s.checks, "check")}` : "No checks yet";
}

/** The last `n` UTC days up to and including `now`'s, oldest first: "2026-10-27" ... "2026-11-02". */
export function lastUtcDays(n: number, now: number): string[] {
  const today = Date.UTC(new Date(now).getUTCFullYear(), new Date(now).getUTCMonth(), new Date(now).getUTCDate());
  return Array.from({ length: n }, (_, i) => new Date(today - (n - 1 - i) * 86_400_000).toISOString().slice(0, 10));
}

/** A check run's own running time: its wall time, or (not reported) the ran files' recorded time, estPlainMs − savedMs. */
export function ranMs(r: Pick<TestRunSummary, "timing">): number {
  const { wallMs, estPlainMs, savedMs } = r.timing;
  if (wallMs !== null) return Math.max(0, wallMs);
  if (estPlainMs !== null) return Math.max(0, estPlainMs - (savedMs ?? 0));
  return 0;
}

/**
 * The Overview's "CI minutes per day" from test runs: per UTC day of `days`,
 * check runs only (legacy background recordings never count), saved = sum of
 * timing.savedMs (an estimate, shown "≈ saved"), ran = ranMs. Minutes are not
 * rounded here: test runs are seconds, and the chart formats them.
 */
export function testMinutesPerDay(runs: (Pick<TestRunSummary, "kind" | "at" | "timing"> & Partial<Pick<TestRunSummary, "id" | "repository" | "suite" | "sha">>)[], days: string[]): DayMinutes[] {
  const by = new Map(days.map((d) => [d, { day: d, replayedMinutes: 0, ranMinutes: 0 }]));
  for (const r of checkRuns(runs.map((r, i) => ({ ...r, id: r.id ?? `#${i}` })))) {
    const d = by.get(new Date(r.at).toISOString().slice(0, 10));
    if (!d) continue;
    d.replayedMinutes += (r.timing.savedMs ?? 0) / 60_000;
    d.ranMinutes += ranMs(r) / 60_000;
  }
  return days.map((d) => by.get(d)!);
}

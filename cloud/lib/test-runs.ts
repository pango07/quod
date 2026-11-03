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

export function kindLabel(kind: TestRunSummary["kind"]): string {
  return kind === "shadow" ? "Background recording" : "Your check";
}

export const KIND_EXPLAINED: Record<TestRunSummary["kind"], string> = {
  critical: "The test job your pull request or push waits for. It skips test files whose code did not change.",
  shadow: "A job that runs in the background for the same commit. It records results for later runs and re-runs a few skipped files to check them.",
};

/** "Tests passed", "Tests failed (exit code 1)", or "Result not reported". */
export function exitLabel(exit: number | null): { text: string; tone: "green" | "red" | "dim" } {
  if (exit === null) return { text: "Result not reported", tone: "dim" };
  return exit === 0 ? { text: "Tests passed", tone: "green" } : { text: `Tests failed (exit code ${exit})`, tone: "red" };
}

/** The spot-checks of a run, in one sentence; null when there were none (yet). */
export function spotCheckSentence(r: Pick<TestRunDetail, "audits" | "auditFiles">): string | null {
  const files = r.auditFiles ?? [];
  const checked = files.length || r.audits?.checked || 0;
  const bad = files.length ? files.filter((f) => !f.ok).length : (r.audits?.mismatched ?? 0);
  if (!checked) return null;
  const head = checked === 1 ? "1 skipped file was re-run in the background to check" : `${formatCount(checked)} skipped files were re-run in the background to check`;
  if (bad === 0) return `${head}: ${checked === 1 ? "it matched" : "all matched"}.`;
  if (checked === 1) return `${head}: it gave a different result.`;
  return `${head}: ${formatCount(bad)} did not match.`;
}

/**
 * Does a run's page show a spot-check section? Not when there is nothing to
 * say: no spot-checks, and either nothing was skipped (nothing to check), or
 * the background recording already reported (or this is it) without any.
 * A check that skipped files and is still waiting for its recording keeps
 * the section ("None yet ...").
 */
export function showSpotChecks(r: Pick<TestRunDetail, "audits" | "auditFiles" | "files" | "kind" | "shadow">): boolean {
  if (spotCheckSentence(r) !== null) return true;
  if (r.files.replayed === 0) return false;
  return r.kind === "critical" && !r.shadow;
}

/** Spot-checks over the 7 days before now, from the background recordings (shadow runs), so a check and its shadow count once. */
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

/** Does the Overview card have anything to say? A team that never ran tests through Quod sees nothing. */
export function hasTestActivity(s: TestStats | null): s is TestStats {
  return !!s && (s.recent.length > 0 || s.week.runs > 0 || s.today.runs > 0 || s.audits.checked > 0);
}

/**
 * Should a run's page refresh for a `test.run` event? Its own id (a re-sent
 * report), or the background recording of the same commit and suite (its
 * spot-checks are merged into this run).
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
 * The share of test files skipped over the newest `n` check runs (kind
 * "critical"; background recordings never count): sum(replayed) /
 * sum(total), so a small one-suite run or a first run after a rename moves
 * it by its size, not as one vote of n. Order-independent: the runs are
 * sorted newest first here (ties by id).
 */
export function skippedShare(runs: Pick<TestRunSummary, "id" | "at" | "kind" | "files">[], n = LAST_CHECKS): SkippedShare {
  const checks = runs
    .filter((r) => r.kind === "critical" && r.files.total > 0)
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
  /** The newest run (critical or not, any age). */
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
  for (const [key, own] of byRepo) {
    let last: RepoTests["last"] = null;
    let savedMs = 0;
    let week = 0;
    for (const r of own) {
      const at = Date.parse(r.at);
      if (!last || at > Date.parse(last.at)) last = { id: r.id, repoSlug: r.repoSlug, at: r.at, exit: r.exit };
      if (r.kind !== "critical" || !(at <= now && now - at <= WEEK_MS)) continue;
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
 * check runs only (background recordings never count), saved = sum of
 * timing.savedMs (an estimate, shown "≈ saved"), ran = ranMs. Minutes are not
 * rounded here: test runs are seconds, and the chart formats them.
 */
export function testMinutesPerDay(runs: Pick<TestRunSummary, "kind" | "at" | "timing">[], days: string[]): DayMinutes[] {
  const by = new Map(days.map((d) => [d, { day: d, replayedMinutes: 0, ranMinutes: 0 }]));
  for (const r of runs) {
    if (r.kind !== "critical") continue;
    const d = by.get(new Date(r.at).toISOString().slice(0, 10));
    if (!d) continue;
    d.replayedMinutes += (r.timing.savedMs ?? 0) / 60_000;
    d.ranMinutes += ranMs(r) / 60_000;
  }
  return days.map((d) => by.get(d)!);
}

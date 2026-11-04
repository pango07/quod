// Words for test files on the Trust page (test-level replay): one place, so
// Trust, the Overview's "Test files" line and the tests say the same thing.
// Same vocabulary as the plain trust pages: learning, ready to skip,
// skipping, always runs, paused. Pure; shared with tests.

import type { TestFile, TestFileState, TestRunSummary, TestStats, TestSuite } from "@/lib/api/types";
import type { Section } from "@/lib/api/section";
import type { Tone } from "@/lib/trust";
import { formatCount } from "@/lib/format";
import { formatSaved, type SkippedShare, skippedShare, spotChecksWithinWeek, testRunHref } from "@/lib/test-runs";

const plural = (n: number, one: string, many = `${one}s`) => `${formatCount(n)} ${n === 1 ? one : many}`;

export interface FileStateLabel {
  label: string;
  /** The tooltip: what it means and what happens next. */
  help: string;
  tone: Tone;
}

export const FILE_LEGEND: { state: TestFileState; label: string; help: string }[] = [
  { state: "learning", label: "Learning", help: "Not recorded yet. It runs every time until a run on a writer branch records it." },
  { state: "ready", label: "Ready to skip", help: "Recorded. The next time nothing it depends on has changed, Quod reuses the recorded result instead of running it." },
  { state: "skipping", label: "Skipping", help: "Quod skips it when nothing it depends on changed and reuses the recorded result. A few skipped files are re-run in the background to check." },
  { state: "always", label: "Always runs", help: "Quod runs it every time, for the reason shown." },
  { state: "paused", label: "Paused", help: "A background re-run gave a different result from the recorded one. It runs every time until it is consistent again." },
];

const HELP = Object.fromEntries(FILE_LEGEND.map((l) => [l.state, l.help])) as Record<TestFileState, string>;

/** The state of a test file in words: "Skipping: skipped 12 times", "Always runs: it failed last time". */
export function fileStateLabel(f: Pick<TestFile, "state" | "reason" | "skipped">): FileStateLabel {
  const reason = f.reason?.trim() || null;
  switch (f.state) {
    case "learning":
      return { label: "Learning", help: reason ? `${HELP.learning} (${reason})` : HELP.learning, tone: "gray" };
    case "ready":
      return { label: "Ready to skip", help: HELP.ready, tone: "blue" };
    case "skipping":
      return { label: f.skipped > 0 ? `Skipping: skipped ${plural(f.skipped, "time")}` : "Skipping", help: HELP.skipping, tone: "blue" };
    case "always":
      return { label: reason ? `Always runs: ${reason}` : "Always runs", help: HELP.always, tone: "dim" };
    case "paused":
      return { label: "Paused: a spot-check differed", help: HELP.paused, tone: "amber" };
    default:
      // A state a newer server added: show it as it is rather than guess.
      return { label: String(f.state), help: "", tone: "gray" };
  }
}

/** vitest's state at the last recording, in words. */
export function lastResultText(r: string | null): string {
  if (r === "pass") return "Passed";
  if (r === "fail") return "Failed";
  if (r === "skip") return "Skipped by vitest";
  if (!r) return "Not recorded yet";
  return r;
}

/** "class-management · packages/backend"; the root suite (".") is "class-management · repo root". */
export function suiteHeading(s: Pick<TestSuite, "repository" | "suite">): string {
  const repo = s.repository.split("/").pop() || s.repository;
  const dir = s.suite.trim();
  return `${repo} · ${dir === "" || dir === "." || dir === "./" ? "repo root" : dir}`;
}

/** A suite without a recording. Any writer branch records (not only the default branch), so it names none. */
export const NO_RECORDING = "No recording yet: the first run on a writer branch records these files.";

/** Which recording a suite reads: "Reads the recording of main at 1d7e5b2, 2 h ago." */
export function suiteLedgerText(l: TestSuite["ledger"], ago: (iso: string) => string): string {
  if (!l) return NO_RECORDING;
  const sha = l.sha ? l.sha.slice(0, 7) : null;
  const at = ago(l.at);
  if (l.branch && sha) return `Reads the recording of ${l.branch} at ${sha}, ${at}.`;
  if (sha) return `Reads the recording at ${sha}, ${at}.`;
  if (l.branch) return `Reads the latest recording of ${l.branch}, ${at}.`;
  return `Reads the latest recording, ${at}.`;
}

/** The counts that matter, in order, zeros left out: "152 skipping, 9 ready, 3 always run". */
export function stateCounts(t: Pick<TestSuite["totals"], "learning" | "ready" | "skipping" | "always" | "paused">): string {
  const parts: [number, string][] = [
    [t.skipping, "skipping"],
    [t.ready, "ready"],
    [t.learning, "learning"],
    [t.always, "always run"],
    [t.paused, "paused"],
  ];
  return parts
    .filter(([n]) => n > 0)
    .map(([n, w]) => `${formatCount(n)} ${w}`)
    .join(", ");
}

/** A suite's totals in one line: "184 test files: 170 skipping, 10 ready, 1 learning, 2 always run, 1 paused. Saved 3.4 h." */
export function suiteTotalsText(t: TestSuite["totals"]): string {
  const counts = stateCounts(t);
  const saved = t.savedMs > 0 ? ` Saved ${formatSaved(t.savedMs)}.` : "";
  return `${plural(t.files, "test file")}${counts ? `: ${counts}` : ""}.${saved}`;
}

/** Every suite added up, for the Overview: "280 test files: 258 skipping, 17 ready, 3 always run"; null when there are none. */
export function testFilesLine(suites: TestSuite[] | null): string | null {
  if (!suites?.length) return null;
  const t = suites.reduce(
    (a, s) => ({
      files: a.files + s.totals.files,
      learning: a.learning + s.totals.learning,
      ready: a.ready + s.totals.ready,
      skipping: a.skipping + s.totals.skipping,
      always: a.always + s.totals.always,
      paused: a.paused + s.totals.paused,
    }),
    { files: 0, learning: 0, ready: 0, skipping: 0, always: 0, paused: 0 },
  );
  if (t.files === 0) return null;
  const counts = stateCounts(t);
  return `${plural(t.files, "test file")}${counts ? `: ${counts}` : ""}`;
}


/**
 * What the Trust page's test part shows, from how /test-suites and
 * /test-runs answered:
 * - "error": either failed (down, timeout, a bare 404, no session): "Couldn't
 *   load test files" with Retry, never the empty state.
 * - "suites" / "runs": data.
 * - "empty": BOTH answered 200 with nothing (the only way to the empty state).
 * - "not-available": an older server (quod.v1 404 or 501) on either, and no
 *   data from the other: hidden, as before (a card when nothing else shows).
 */
export type TestsView = { kind: "error"; message: string } | { kind: "suites" } | { kind: "runs" } | { kind: "empty" } | { kind: "not-available" };

export function testsView(suites: Section<TestSuite[] | null>, runs: Section<TestRunSummary[] | null>): TestsView {
  for (const s of [suites, runs]) if (!s.ok && s.kind === "unavailable") return { kind: "error", message: s.message };
  if (suites.ok && suites.value?.some((s) => s.files.length > 0 || s.totals.files > 0)) return { kind: "suites" };
  if (runs.ok && runs.value?.length) return { kind: "runs" };
  if (suites.ok && suites.value !== null && runs.ok && runs.value !== null) return { kind: "empty" };
  return { kind: "not-available" };
}

/** Which sections the Trust page shows, top to bottom. */
export interface TrustSections {
  /** Test files per suite (test-first). */
  suites: boolean;
  /** No suites to show, but test runs exist (a server without /test-suites, or nothing recorded yet): the latest runs. */
  testRuns: boolean;
  /** Step-level trust, below the tests; left out when there are no steps. */
  steps: boolean;
  /** Nothing at all: say how to start. */
  empty: boolean;
}

/** Unavailable sources count as nothing here; the page shows their own card. */
export function trustSections(p: { suites: TestSuite[] | null; testRuns: number; steps: number }): TrustSections {
  const suites = !!p.suites?.some((s) => s.files.length > 0 || s.totals.files > 0);
  const testRuns = !suites && p.testRuns > 0;
  const steps = p.steps > 0;
  return { suites, testRuns, steps, empty: !suites && !testRuns && !steps };
}

// ---------------------------------------------------------------- The compact Trust page
// A summary strip, the files that need a look, then one collapsed row per
// suite. Built on the server from /test-suites, /test-runs and /test-stats;
// the client only filters and expands (components/test-suites.tsx).

/** States that need a look: paused (a spot-check differed) and always runs. */
export const NEEDS_LOOK: readonly TestFileState[] = ["paused", "always"];
export const needsLook = (f: Pick<TestFile, "state">) => NEEDS_LOOK.includes(f.state);

/** The stacked bars' order, left to right: learning, ready, skipping, always, paused. */
export const BAR_ORDER = ["learning", "ready", "skipping", "always", "paused"] as const satisfies readonly TestFileState[];

/** An expanded suite lists at most this many files until "Show all". */
export const MAX_FILES = 8;

const WEEK_MS = 7 * 24 * 60 * 60 * 1000;

/** Needs a look first (paused, then always runs), then most time saved, then by path. */
export function sortFiles(files: TestFile[]): TestFile[] {
  const rank = (f: TestFile) => (f.state === "paused" ? 0 : f.state === "always" ? 1 : 2);
  return [...files].sort((a, b) => rank(a) - rank(b) || (b.savedMs ?? 0) - (a.savedMs ?? 0) || a.file.localeCompare(b.file));
}

/** The first `max` files (all when showAll), and how many are left out. */
export function visibleFiles<T>(files: T[], showAll: boolean, max = MAX_FILES): { shown: T[]; hidden: number } {
  if (showAll || files.length <= max) return { shown: files, hidden: 0 };
  return { shown: files.slice(0, max), hidden: files.length - max };
}

export type StateFilter = "all" | "look" | "skipping" | "ready" | "learning";

export const FILTERS: { key: StateFilter; label: string }[] = [
  { key: "all", label: "All" },
  { key: "look", label: "Needs a look" },
  { key: "skipping", label: "Skipping" },
  { key: "ready", label: "Ready" },
  { key: "learning", label: "Learning" },
];

/** Does a file pass the state filter and the search (case-insensitive, on its path)? */
export function matchesFilter(f: Pick<TestFile, "file" | "state">, filter: StateFilter, query = ""): boolean {
  const q = query.trim().toLowerCase();
  if (q && !f.file.toLowerCase().includes(q)) return false;
  if (filter === "all") return true;
  if (filter === "look") return needsLook(f);
  return f.state === filter;
}

/** The count on each filter pill: the listed files that match the search, per filter. */
export function filterCounts(files: Pick<TestFile, "file" | "state">[], query = ""): Record<StateFilter, number> {
  const out: Record<StateFilter, number> = { all: 0, look: 0, skipping: 0, ready: 0, learning: 0 };
  for (const f of files) for (const { key } of FILTERS) if (matchesFilter(f, key, query)) out[key]++;
  return out;
}

function normSuite(s: string): string {
  const t = s.trim().replace(/^\.\/+/, "").replace(/\/+$/, "");
  return t === "" || t === "." ? "." : t;
}

/** A suite's runs: the same repository (case-insensitive) and the same vitest project dir. */
export function runsOfSuite<T extends Pick<TestRunSummary, "repository" | "suite">>(runs: T[], s: Pick<TestSuite, "repository" | "suite">): T[] {
  const repo = s.repository.toLowerCase();
  const dir = normSuite(s.suite);
  return runs.filter((r) => r.repository.toLowerCase() === repo && normSuite(r.suite) === dir);
}

/** Time saved by critical runs in the 7 days before now. */
export function savedWithinWeek(runs: Pick<TestRunSummary, "kind" | "at" | "timing">[], now: number): { savedMs: number; runs: number } {
  const week = runs.filter((r) => {
    const at = Date.parse(r.at);
    return r.kind === "critical" && at <= now && now - at <= WEEK_MS;
  });
  return { savedMs: week.reduce((a, r) => a + (r.timing.savedMs ?? 0), 0), runs: week.length };
}

export type FileCounts = Pick<TestSuite["totals"], "learning" | "ready" | "skipping" | "always" | "paused" | "recordDiffers">;

/** A bar segment: a file state, or "recordDiffers" (files whose recording behaved differently from the check; quarantined). */
export type BarKey = TestFileState | "recordDiffers";

export interface BarSegment {
  state: BarKey;
  n: number;
  label: string;
}

const BAR_WORDS: Record<BarKey, string> = { learning: "learning", ready: "ready", skipping: "skipping", always: "always run", recordDiffers: "recording differed", paused: "paused" };

/**
 * "Recording differed" files (`totals.recordDiffers`, from newer servers;
 * absent is 0). Whether the server also counts them under "always" is read
 * from the totals: when the states already add up to `files`, they are part
 * of "always" (and come out of it here); otherwise they are counted apart.
 */
export function recordDiffers(t: FileCounts & { files: number }): { n: number; inAlways: boolean } {
  const n = Math.max(0, t.recordDiffers ?? 0);
  if (n === 0) return { n: 0, inAlways: false };
  const states = t.learning + t.ready + t.skipping + t.always + t.paused;
  const inAlways = states + n > t.files;
  return { n: inAlways ? Math.min(n, t.always) : n, inAlways };
}

/** The non-empty segments of a stacked state bar: learning, ready, skipping, always, recording differed, paused. */
export function barSegments(t: FileCounts & { files: number }): BarSegment[] {
  const rd = recordDiffers(t);
  const n: Record<BarKey, number> = { learning: t.learning, ready: t.ready, skipping: t.skipping, always: t.always - (rd.inAlways ? rd.n : 0), recordDiffers: rd.n, paused: t.paused };
  const order: BarKey[] = ["learning", "ready", "skipping", "always", "recordDiffers", "paused"];
  return order.filter((k) => n[k] > 0).map((k) => ({ state: k, n: n[k], label: BAR_WORDS[k] }));
}

/** Files that need a look: paused, always runs and recording differed (each counted once). */
export function needLookCount(t: FileCounts & { files: number }): number {
  const rd = recordDiffers(t);
  return t.paused + t.always + (rd.inAlways ? 0 : rd.n);
}

/** "184 files · 170 skipping · 3 need a look (1 recording differed)" (zero counts left out). */
export function suiteRowSummary(t: TestSuite["totals"]): string {
  const look = needLookCount(t);
  const rd = recordDiffers(t).n;
  const parts = [plural(t.files, "file")];
  if (t.skipping > 0) parts.push(`${formatCount(t.skipping)} skipping`);
  if (look > 0) parts.push(`${formatCount(look)} need${look === 1 ? "s" : ""} a look${rd > 0 ? ` (${formatCount(rd)} recording differed)` : ""}`);
  return parts.join(" · ");
}

/** Why a file needs a look, in plain words. An always-runs reason (which may be the recording's own, "uses the network") is shown as it is. */
export function lookReason(f: Pick<TestFile, "state" | "reason">): string {
  if (f.state === "paused") return "A spot-check got a different result. It runs every time until it matches again.";
  const r = f.reason?.trim();
  return r || "Quod runs it every time.";
}

export interface LookItem {
  file: string;
  state: TestFileState;
  suite: string;
  reason: string;
  href: string;
  linkText: string;
}

export interface SuiteRow {
  key: string;
  /** "class-management · apps/web": where the repository is not shown around it. */
  heading: string;
  /** owner/name, for grouping by repository. */
  repository: string;
  /** The suite alone: "apps/web", or "repo root". */
  name: string;
  ledger: string;
  /** Has a recording to read (ledger not null). */
  recorded: boolean;
  /** The repository's test runs. */
  href: string;
  totals: TestSuite["totals"];
  bar: BarSegment[];
  summary: string;
  /** Test files skipped over the last LAST_CHECKS check runs (skippedShare). */
  skipped: SkippedShare;
  lastRunAt: string | null;
  savedWeekMs: number;
  /** Listed files, sorted (sortFiles). */
  files: TestFile[];
  /** Files the server counts but does not list. */
  unlisted: number;
}

export interface TrustSummary {
  wrongResults: number;
  spotChecksPassed: number;
  /** "week": spot-checks of the last 7 days (from the runs); "all": the stats' all-time count. */
  spotChecksWindow: "week" | "all";
  savedWeekMs: number;
  weekRuns: number;
  /** Test files skipped over the last LAST_CHECKS check runs (skippedShare). */
  skipped: SkippedShare;
  totals: FileCounts & { files: number };
  bar: BarSegment[];
}

export interface TrustModel {
  summary: TrustSummary;
  look: LookItem[];
  suites: SuiteRow[];
}

/**
 * Everything the compact Trust page shows, from the three endpoints. Runs
 * come newest first (up to 200); stats may be null (an older server): the
 * week and the spot-checks are then added up from the runs.
 */
export function trustModel(p: { team: string; suites: TestSuite[]; runs: TestRunSummary[]; stats: TestStats | null; now: number; ago: (iso: string) => string }): TrustModel {
  const { team, suites, runs, stats, now } = p;
  const totals = { files: 0, learning: 0, ready: 0, skipping: 0, always: 0, paused: 0, recordDiffers: 0 };
  for (const s of suites) {
    for (const k of ["files", ...BAR_ORDER] as const) totals[k] += s.totals[k];
    totals.recordDiffers += s.totals.recordDiffers ?? 0;
  }
  const week = savedWithinWeek(runs, now);
  const critical = runs.filter((r) => r.kind === "critical");
  const wrong = critical.reduce((a, r) => a + (r.audits?.mismatched ?? 0), 0);
  // Spot-checks of the last 7 days, from the background recordings among the runs (already fetched); the stats' count is all time.
  const spot = spotChecksWithinWeek(runs, now);
  const spotFromRuns = runs.length > 0 || !stats;
  const summary: TrustSummary = {
    wrongResults: stats ? stats.wrongResults : wrong,
    spotChecksPassed: spotFromRuns ? spot.checked - spot.mismatched : stats.audits.passed,
    spotChecksWindow: spotFromRuns ? "week" : "all",
    savedWeekMs: stats ? stats.week.savedMs : week.savedMs,
    weekRuns: stats ? stats.week.runs : week.runs,
    skipped: skippedShare(runs),
    totals,
    bar: barSegments(totals),
  };
  const look: LookItem[] = [];
  const rows: SuiteRow[] = suites.map((s) => {
    const own = runsOfSuite(runs, s);
    const heading = suiteHeading(s);
    const testsHref = `/${team}/${s.repoSlug}/tests`;
    const files = sortFiles(s.files);
    for (const f of files.filter(needsLook)) {
      // Paused: the run whose spot-check differed, when it is still among the runs; otherwise the suite's latest run.
      const differed = f.state === "paused" ? own.find((r) => (r.audits?.mismatched ?? 0) > 0) : undefined;
      const run = differed ?? own.find((r) => r.kind === "critical") ?? own[0];
      look.push({
        file: f.file,
        state: f.state,
        suite: heading,
        reason: lookReason(f),
        href: run ? testRunHref(team, run) : testsHref,
        linkText: differed ? "See what differed" : run ? "Latest run" : "Test runs",
      });
    }
    const dir = s.suite.trim();
    return {
      key: `${s.repository}:${s.suite}`,
      heading,
      repository: s.repository,
      name: dir === "" || dir === "." || dir === "./" ? "repo root" : dir,
      ledger: suiteLedgerText(s.ledger, p.ago),
      recorded: !!s.ledger,
      href: testsHref,
      totals: s.totals,
      bar: barSegments(s.totals),
      summary: suiteRowSummary(s.totals),
      skipped: skippedShare(own),
      lastRunAt: own[0]?.at ?? null,
      savedWeekMs: savedWithinWeek(own, now).savedMs,
      files,
      unlisted: Math.max(0, s.totals.files - s.files.length),
    };
  });
  // Paused before always runs, across suites (stable within each).
  look.sort((a, b) => (a.state === "paused" ? 0 : 1) - (b.state === "paused" ? 0 : 1));
  return { summary, look, suites: rows };
}

/** One repository's suites, for the Trust page's repo headings. */
export interface RepoSuites {
  repository: string;
  /** The repository's name without its owner. */
  name: string;
  suites: SuiteRow[];
  files: number;
  savedWeekMs: number;
}

/** Suites grouped by repository: the repository with the most test files first, then by name; suites keep their order. */
export function suitesByRepo(rows: SuiteRow[]): RepoSuites[] {
  const by = new Map<string, RepoSuites>();
  for (const r of rows) {
    const k = r.repository.toLowerCase();
    let g = by.get(k);
    if (!g) {
      g = { repository: r.repository, name: r.repository.split("/").pop() || r.repository, suites: [], files: 0, savedWeekMs: 0 };
      by.set(k, g);
    }
    g.suites.push(r);
    g.files += r.totals.files;
    g.savedWeekMs += r.savedWeekMs;
  }
  return [...by.values()].sort((a, b) => b.files - a.files || a.name.localeCompare(b.name));
}

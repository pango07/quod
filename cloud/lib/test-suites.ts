// Words for test files on the Trust page (test-level replay): one place, so
// Trust, the Overview's "Test files" line and the tests say the same thing.
// Same vocabulary as the plain trust pages: learning, ready to skip,
// skipping, always runs, paused. Pure; shared with tests.

import type { TestFile, TestFileState, TestSuite } from "@/lib/api/types";
import type { Tone } from "@/lib/trust";
import { formatCount } from "@/lib/format";
import { formatSavedMs } from "@/lib/test-runs";

const plural = (n: number, one: string, many = `${one}s`) => `${formatCount(n)} ${n === 1 ? one : many}`;

export interface FileStateLabel {
  label: string;
  /** The tooltip: what it means and what happens next. */
  help: string;
  tone: Tone;
}

export const FILE_LEGEND: { state: TestFileState; label: string; help: string }[] = [
  { state: "learning", label: "Learning", help: "Not recorded yet. It runs every time until a run on the default branch records it." },
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

/** Which recording a suite reads: "Reads the recording of main at 1d7e5b2, 2 h ago." */
export function suiteLedgerText(l: TestSuite["ledger"], ago: (iso: string) => string): string {
  if (!l) return "No recording yet. Once a run on the default branch records these files, unchanged ones can be skipped.";
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
  const saved = t.savedMs > 0 ? ` Saved ${formatSavedMs(t.savedMs)}.` : "";
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

const ATTENTION: TestFileState[] = ["paused", "always", "learning"];

/** Files to show first (paused, always runs, learning) and the rest (skipping, ready), each in the server's order. */
export function splitFiles(files: TestFile[]): { attention: TestFile[]; rest: TestFile[] } {
  const attention = files.filter((f) => ATTENTION.includes(f.state));
  const rest = files.filter((f) => !ATTENTION.includes(f.state));
  return { attention, rest };
}

/** Which sections the Trust page shows, top to bottom. */
export interface TrustSections {
  /** Test files per suite (test-first). */
  suites: boolean;
  /** No suites to show, but test runs exist (a server without /test-suites, or nothing recorded on the default branch yet): the latest runs. */
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

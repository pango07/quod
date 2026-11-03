// Words and numbers for test runs (test-level replay): one place, so the
// Overview card, the Tests tab and a run's page say the same thing. Plain
// language, as on the Trust pages: a skipped file is "skipped (unchanged)",
// never "replayed". Pure; shared with tests.

import type { TestRunDetail, TestRunSummary, TestStats } from "@/lib/api/types";
import { formatCount } from "@/lib/format";

/** A time given in milliseconds: "0 s", "0.4 s", "79 s", "12 min", "3.4 h". */
export function formatSavedMs(ms: number | null | undefined): string {
  if (ms === null || ms === undefined || !Number.isFinite(ms) || ms < 0) return "n/a";
  const s = ms / 1000;
  if (s > 0 && s < 9.95) return `${s.toFixed(1).replace(/\.0$/, "")} s`;
  if (s < 90) return `${Math.round(s)} s`;
  const min = s / 60;
  if (min < 90) return `${Math.round(min)} min`;
  const h = min / 60;
  return `${h < 10 ? h.toFixed(1).replace(/\.0$/, "") : Math.round(h)} h`;
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
  const saved = r.timing.savedMs === null ? "" : `, saved ${formatSavedMs(r.timing.savedMs)}`;
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

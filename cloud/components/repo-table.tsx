import Link from "next/link";
import type { RepoSummary } from "@/lib/api/types";
import { formatAgo, formatMinutes, formatPercent } from "@/lib/format";
import { formatSaved, repoSkippedNote, type RepoTests, SAVED_EXPLAINED, testRunHref } from "@/lib/test-runs";
import { ProtectionBadge } from "@/components/section";
import { Dot, Icon } from "@/components/ui";

// quod-server does not know a repo's language (it sends ""): just the runner then.
const LANG_RUNNER = (r: RepoSummary) => (r.hasQuodData === false ? "from GitHub" : [r.language, r.runner === "self-hosted" ? "self-hosted" : "GitHub-hosted"].filter(Boolean).join(" · "));

/** Quod's numbers for a repo it has not run in yet. */
const NOT_YET = "Appears once Quod runs in this repo";

export function lastRunText(r: RepoSummary, asOf: string): { text: string; tone: "green" | "amber" | "red" | "blue" | "gray" } {
  if (r.pausedSteps > 0) return { text: r.pausedSteps === 1 ? "1 step paused" : `${r.pausedSteps} steps paused`, tone: "amber" };
  const gh = r.github?.latestRun;
  if (gh) {
    const when = gh.at ? ` · ${formatAgo(gh.at, asOf)}` : "";
    if (gh.status !== "completed") return { text: `${gh.status === "queued" ? "Queued" : "Running"}${when}`, tone: "blue" };
    if (gh.conclusion === "success") return { text: `Passed${when}`, tone: "green" };
    if (gh.conclusion === "cancelled" || gh.conclusion === "skipped") return { text: `${gh.conclusion === "cancelled" ? "Cancelled" : "Skipped"}${when}`, tone: "gray" };
    return { text: `Failed${when}`, tone: "red" };
  }
  if (!r.lastRunAt) return { text: "No runs yet", tone: "gray" };
  const when = formatAgo(r.lastRunAt, asOf);
  if (r.lastRunStatus === "failed") return { text: `Failed · ${when}`, tone: "red" };
  if (r.lastRunStatus === "running") return { text: `Running · ${when}`, tone: "blue" };
  return { text: `Ready · ${when}`, tone: "green" };
}

/** The newest test run, in words: "Tests passed · 4m ago". */
export function lastTestRunText(last: NonNullable<RepoTests["last"]>, asOf: string): { text: string; tone: "green" | "red" | "gray" } {
  const when = formatAgo(last.at, asOf);
  if (last.exit === null) return { text: `Tests ran · ${when}`, tone: "gray" };
  return last.exit === 0 ? { text: `Tests passed · ${when}`, tone: "green" } : { text: `Tests failed · ${when}`, tone: "red" };
}

/** Test numbers for a repo, matched by full name (owner/name), never by slug. */
const testsOf = (tests: Record<string, RepoTests> | undefined, r: RepoSummary) => tests?.[r.fullName.toLowerCase()];

/**
 * The repositories, one row each. With `tests` (per full name, from
 * /test-runs), a repo with test runs shows test files skipped and time saved
 * over the last 7 days and its last test run; the step columns stay for repos
 * with step data and no test runs.
 */
export function RepoTable({ team, repos, asOf, savedLabel = "Saved", tests }: { team: string; repos: RepoSummary[]; asOf: string; savedLabel?: string; tests?: Record<string, RepoTests> }) {
  const anyTests = repos.some((r) => testsOf(tests, r));
  return (
    <div>
      <div className="table-head cols-repos" aria-hidden="true">
        <span>Repository</span>
        <span>{anyTests ? "Skipped" : "Steps replayed"}</span>
        <span>{anyTests ? "Saved" : savedLabel}</span>
        <span>Last run</span>
        <span />
      </div>
      {repos.map((r) => {
        const t = testsOf(tests, r);
        const last = t?.last ? lastTestRunText(t.last, asOf) : lastRunText(r, asOf);
        const share = t && t.total > 0 ? t.skipped / t.total : 0;
        return (
          <div key={r.slug} className="trow cols-repos linked">
            <span className="cell-main">
              <Link href={`/${team}/${r.slug}`} className="stretch" style={{ fontWeight: 500 }}>
                {r.fullName}
              </Link>
              <span className="mono cell-sub">{LANG_RUNNER(r)}</span>
              {r.protection ? (
                <span className="over" style={{ marginTop: 4 }}>
                  <ProtectionBadge p={r.protection} />
                </span>
              ) : null}
            </span>
            {t ? (
              <>
                <span>
                  <span className="label-sm">Test files skipped </span>
                  <span className="meter">
                    <span className="meter-track">
                      <span className="meter-fill" style={{ width: formatPercent(share) }} />
                    </span>
                    <span className="mono" style={{ fontSize: 12, width: 36, textAlign: "right" }}>
                      {t.total > 0 ? formatPercent(share) : "n/a"}
                    </span>
                  </span>
                  <span className="faint" style={{ fontSize: 12, display: "block", marginTop: 4 }}>
                    Test files: {repoSkippedNote(t)}
                  </span>
                </span>
                <span className="mono" style={{ fontSize: 13 }} title={SAVED_EXPLAINED}>
                  <span className="label-sm" style={{ fontFamily: "var(--sans)" }}>
                    Saved{" "}
                  </span>
                  {formatSaved(t.savedMs)}
                  <span className="faint" style={{ fontSize: 12, display: "block", fontFamily: "var(--sans)" }}>
                    last 7 days
                  </span>
                </span>
              </>
            ) : r.hasQuodData === false ? (
              <span className="faint" style={{ fontSize: 12, gridColumn: "span 2" }}>
                {NOT_YET}
              </span>
            ) : (
              <>
                <span>
                  <span className="label-sm">Steps replayed </span>
                  <span className="meter">
                    <span className="meter-track">
                      <span className="meter-fill" style={{ width: formatPercent(r.replayRate) }} />
                    </span>
                    <span className="mono" style={{ fontSize: 12, width: 36, textAlign: "right" }}>
                      {formatPercent(r.replayRate)}
                    </span>
                  </span>
                  {anyTests ? (
                    <span className="faint" style={{ fontSize: 12, display: "block", marginTop: 4 }}>
                      Steps
                    </span>
                  ) : null}
                </span>
                <span className="mono" style={{ fontSize: 13 }}>
                  <span className="label-sm" style={{ fontFamily: "var(--sans)" }}>
                    {savedLabel}{" "}
                  </span>
                  {formatMinutes(r.savedMinutes)}
                </span>
              </>
            )}
            <span style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 13, color: "var(--muted)" }}>
              <Dot tone={last.tone === "gray" ? "dim" : last.tone} />
              <span className="sr-only">Last run: </span>
              {t?.last ? (
                <Link href={testRunHref(team, t.last)} className="over link-muted">
                  {last.text}
                </Link>
              ) : (
                last.text
              )}
            </span>
            <span className="chev" aria-hidden="true">
              <Icon name="chevron" stroke="#8f8f8f" />
            </span>
          </div>
        );
      })}
    </div>
  );
}

import Link from "next/link";
import type { RepoSummary } from "@/lib/api/types";
import { formatAgo, formatMinutes, formatPercent } from "@/lib/format";
import { Dot, Icon } from "@/components/ui";

const LANG_RUNNER = (r: RepoSummary) => `${r.language} · ${r.runner === "self-hosted" ? "self-hosted" : "GitHub-hosted"}`;

export function lastRunText(r: RepoSummary, asOf: string): { text: string; tone: "green" | "amber" | "red" | "blue" | "gray" } {
  if (r.pausedSteps > 0) return { text: r.pausedSteps === 1 ? "1 step paused" : `${r.pausedSteps} steps paused`, tone: "amber" };
  if (!r.lastRunAt) return { text: "No runs yet", tone: "gray" };
  const when = formatAgo(r.lastRunAt, asOf);
  if (r.lastRunStatus === "failed") return { text: `Failed · ${when}`, tone: "red" };
  if (r.lastRunStatus === "running") return { text: `Running · ${when}`, tone: "blue" };
  return { text: `Ready · ${when}`, tone: "green" };
}

export function RepoTable({ team, repos, asOf, savedLabel = "Saved" }: { team: string; repos: RepoSummary[]; asOf: string; savedLabel?: string }) {
  return (
    <div>
      <div className="table-head cols-repos" aria-hidden="true">
        <span>Repository</span>
        <span>Steps replayed</span>
        <span>{savedLabel}</span>
        <span>Last run</span>
        <span />
      </div>
      {repos.map((r) => {
        const last = lastRunText(r, asOf);
        return (
          <Link key={r.slug} className="trow cols-repos" href={`/${team}/${r.slug}`}>
            <span className="cell-main">
              <span style={{ fontWeight: 500 }}>{r.fullName}</span>
              <span className="mono cell-sub">{LANG_RUNNER(r)}</span>
            </span>
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
            </span>
            <span className="mono" style={{ fontSize: 13 }}>
              <span className="label-sm" style={{ fontFamily: "var(--sans)" }}>
                {savedLabel}{" "}
              </span>
              {formatMinutes(r.savedMinutes)}
            </span>
            <span style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 13, color: "var(--muted)" }}>
              <Dot tone={last.tone === "gray" ? "dim" : last.tone} />
              <span className="sr-only">Last run: </span>
              {last.text}
            </span>
            <span className="chev" aria-hidden="true">
              <Icon name="chevron" stroke="#8f8f8f" />
            </span>
          </Link>
        );
      })}
    </div>
  );
}

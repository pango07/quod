import { notFound } from "next/navigation";
import { RerunButton } from "@/components/rerun-button";
import { Elapsed, LiveRefresh } from "@/components/live/parts";
import { RunSteps } from "@/components/run-steps";
import { Dot } from "@/components/ui";
import { SectionUnavailable } from "@/components/section";
import { api } from "@/lib/api";
import { section } from "@/lib/api/section";
import { requireTeam } from "@/lib/dal";
import { formatDuration, shortSha } from "@/lib/format";
import { one, parseRunNumber } from "@/lib/params";

export const metadata = { title: "Run" };

export default async function RunPage({ params, searchParams }: PageProps<"/[team]/[repo]/runs/[run]">) {
  const [{ team: slug, repo: repoSlug, run: runParam }, sp] = await Promise.all([params, searchParams]);
  const number = parseRunNumber(runParam);
  if (number === null) notFound();
  const { team } = await requireTeam(slug);
  const found = await section(api().getRun(team.slug, repoSlug, number));
  if (!found.ok) return <SectionUnavailable s={found} what="This run" />;
  const run = found.value;
  if (!run) notFound();

  const steps = run.jobs.flatMap((j) => j.steps);
  // The render time, handed to the ticking clocks so server HTML and hydration agree.
  const asOf = new Date().toISOString();
  // Default selection: the step from ?step=, else the first that ran because something changed, else the first replay.
  const fromUrl = Number(one(sp.step));
  const firstChanged = steps.findIndex((s) => s.why.kind === "changed" || s.why.kind === "paused" || (s.why.kind === "spot-check" && !s.why.identical));
  const firstReplay = steps.findIndex((s) => s.outcome === "replayed");
  const running = steps.findIndex((s) => s.outcome === "running");
  const initial =
    Number.isInteger(fromUrl) && fromUrl >= 1 && fromUrl <= steps.length ? fromUrl - 1 : running >= 0 ? running : firstChanged >= 0 ? firstChanged : Math.max(0, firstReplay);
  const spot = run.spotChecks === 0 ? "None" : run.spotChecks === run.spotChecksIdentical ? `${run.spotChecks}, identical` : `${run.spotChecks}, ${run.spotChecks - run.spotChecksIdentical} different`;

  return (
    <div className="page tight">
      {run.status === "running" ? <LiveRefresh scope={{ kind: "run", repo: repoSlug, run: run.number }} /> : null}
      <div className="row-between" style={{ alignItems: "flex-start", gap: 16 }}>
        <div style={{ display: "flex", flexDirection: "column", gap: 8, minWidth: 0 }}>
          <h1 style={{ fontSize: 26, fontWeight: 600, letterSpacing: "-0.02em", lineHeight: 1.25 }}>{run.title}</h1>
          <div style={{ display: "flex", gap: 14, alignItems: "center", fontSize: 13, color: "var(--muted)", flexWrap: "wrap" }}>
            <span style={{ display: "flex", alignItems: "center", gap: 6 }}>
              <Dot tone={run.status === "passed" ? "green" : run.status === "failed" ? "red" : "blue"} />
              {run.status === "passed" ? "Passed" : run.status === "failed" ? "Failed" : "Running"}
            </span>
            <span className="mono">
              {run.branch} · {shortSha(run.sha)}
            </span>
            <span>
              {run.event === "pull_request" ? "pull request" : run.event.replace("_", " ")} by {run.actor}
            </span>
            <span className="mono">{run.runner}</span>
          </div>
        </div>
        <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
          <a className="btn" href={run.githubUrl} rel="noreferrer" target="_blank">
            View on GitHub
          </a>
          <RerunButton team={team.slug} repo={repoSlug} run={run.number} />
        </div>
      </div>

      <section aria-label="Time" className="stats sm">
        <div className="card stat">
          <span className="label">{run.status === "running" ? "Running for" : "Took"}</span>
          <span className="value">{run.status === "running" ? <Elapsed since={run.startedAt} asOf={asOf} /> : formatDuration(run.tookSeconds)}</span>
        </div>
        <div className="card stat">
          <span className="label">Would have taken</span>
          <span className="value" style={{ color: "var(--muted)" }}>
            {formatDuration(run.wouldHaveTakenSeconds)}
          </span>
        </div>
        <div className="card stat">
          <span className="label">Steps replayed</span>
          <span className="value">
            {run.stepsReplayed} of {run.stepsTotal}
          </span>
        </div>
        <div className="card stat">
          <span className="label">Spot checks</span>
          <span className="value">{spot}</span>
        </div>
      </section>

      <RunSteps team={team.slug} repo={repoSlug} run={run.number} jobs={run.jobs} initial={initial} asOf={asOf} />
    </div>
  );
}

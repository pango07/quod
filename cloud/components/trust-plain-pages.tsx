// Server parts of the plain trust pages (cloud/trust-plain; lib/ui-version.ts
// switches them on). The classic pages render as before when it is off.

import Link from "next/link";
import { ciState, QuodPending } from "@/components/github-ci";
import { ProtectedBranchHelp } from "@/components/protected-branch-help";
import { SectionUnavailable } from "@/components/section";
import { ProtectionNote, TrustList } from "@/components/trust-plain";
import { Dot, EmptyState } from "@/components/ui";
import { api } from "@/lib/api";
import { section } from "@/lib/api/section";
import type { StepSummary, Team } from "@/lib/api/types";
import { teamGitHub } from "@/lib/github/team-data";
import { RECENT_RUNS, recentRunDetails } from "@/lib/recent-runs";
import { groupSteps, recentByKey } from "@/lib/trust-plain";

/** /[team]/trust, plain. */
export async function PlainTrust({ team }: { team: Team }) {
  const [found, gh, runs] = await Promise.all([section(api().listSteps(team.slug)), teamGitHub(team.orgId), recentRunDetails(team.slug)]);
  const groups = groupSteps(found.ok ? found.value : [], recentByKey(runs));
  const paused = groups.filter((g) => g.current.state.kind === "paused");
  const asOf = new Date().toISOString();
  return (
    <div className="page">
      <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
        <h1 className="h1">Trust</h1>
        <p className="muted" style={{ maxWidth: "72ch", lineHeight: 1.6 }}>
          Quod skips a step when nothing it reads has changed since a good run, and gives back that run&apos;s result. Each step learns first, then its first skips are re-run
          and compared, and a few more at random after that. If a re-check finds different output, skipping pauses and the step runs every time.
        </p>
      </div>
      {paused.map((g) => (
        <Link key={g.id} href={`/${team.slug}/${g.repoSlug}/steps/${g.current.step.key}#diff`} className="notice">
          <Dot tone="amber" />
          <span style={{ display: "flex", flexDirection: "column", gap: 2 }}>
            <span style={{ fontWeight: 500 }}>
              {g.repoSlug} · {g.job ? `${g.job} / ` : ""}
              <span className={g.untitled ? "" : "mono"}>{g.title}</span>: paused
            </span>
            <span className="soft">A re-check got different output from the same inputs. It runs every time until it matches again. Open it to see what differed.</span>
          </span>
        </Link>
      ))}
      {!found.ok ? (
        <SectionUnavailable s={found} what="Steps" />
      ) : groups.length ? (
        <TrustList team={team.slug} groups={groups} asOf={asOf} showRepo recentRuns={RECENT_RUNS} />
      ) : (
        <section className="card">
          {gh.sample ? (
            <EmptyState title="Nothing here yet">Steps appear here after their first run with Quod.</EmptyState>
          ) : (
            <QuodPending team={team.slug} state={ciState(gh.link, false)}>
              Steps show up here once Quod runs them, and running Quod in your CI is in early access.
            </QuodPending>
          )}
        </section>
      )}
      <ProtectionNote>
        <ProtectedBranchHelp bare />
      </ProtectionNote>
    </div>
  );
}

/** /[team]/[repo]/steps, plain. */
export async function PlainRepoSteps({ team, repo, steps }: { team: Team; repo: string; steps: StepSummary[] }) {
  const runs = await recentRunDetails(team.slug, { repo });
  const groups = groupSteps(steps, recentByKey(runs));
  return (
    <div className="page">
      <h1 className="sr-only">Steps of {repo}</h1>
      <TrustList team={team.slug} groups={groups} asOf={new Date().toISOString()} recentRuns={RECENT_RUNS} />
    </div>
  );
}

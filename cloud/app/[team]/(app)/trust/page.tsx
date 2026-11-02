import Link from "next/link";
import { ciState, QuodPending } from "@/components/github-ci";
import { StepsTable } from "@/components/steps-table";
import { Dot, EmptyState } from "@/components/ui";
import { ProtectedBranchHelp } from "@/components/protected-branch-help";
import { SectionUnavailable } from "@/components/section";
import { api } from "@/lib/api";
import { section } from "@/lib/api/section";
import { requireTeam } from "@/lib/dal";
import { teamGitHub } from "@/lib/github/team-data";
import { parseStepFilter } from "@/lib/params";

export const metadata = { title: "Trust" };

const ORDER = { paused: 0, proving: 1, watching: 2, replaying: 3 } as const;

export default async function TrustPage({ params, searchParams }: PageProps<"/[team]/trust">) {
  const [{ team: slug }, sp] = await Promise.all([params, searchParams]);
  const { team } = await requireTeam(slug);
  const [found, gh] = await Promise.all([section(api().listSteps(team.slug)), teamGitHub(team.orgId)]);
  const steps = (found.ok ? found.value : []).sort((a, b) => ORDER[a.trust.kind] - ORDER[b.trust.kind]);
  const paused = steps.filter((s) => s.trust.kind === "paused");
  return (
    <div className="page">
      <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
        <h1 className="h1">Trust</h1>
        <p className="muted" style={{ maxWidth: "70ch" }}>
          A step can replay once it has run with the same inputs before. Its first replays are re-run and compared, and a few more at random after that. If one ever differs,
          the step is paused and runs every time until it is consistent again.
        </p>
      </div>
      {paused.map((s) => (
        <Link key={s.key} href={`/${team.slug}/${s.repoSlug}/steps/${s.key}`} className="notice">
          <Dot tone="amber" />
          <span style={{ display: "flex", flexDirection: "column", gap: 2 }}>
            <span style={{ fontWeight: 500 }}>
              {s.repoSlug} · {s.job} / <span className="mono">{s.command}</span> is paused
            </span>
            <span className="soft">A spot check gave a different result from the same inputs. Open it to see what differed.</span>
          </span>
        </Link>
      ))}
      {!found.ok ? (
        <SectionUnavailable s={found} what="Steps" />
      ) : steps.length ? (
        <StepsTable team={team.slug} steps={steps} initial={parseStepFilter(sp.state)} showRepo />
      ) : (
        <section className="card">
          {gh.sample ? (
            <EmptyState title="Nothing to trust yet">Steps appear here after their first run with Quod.</EmptyState>
          ) : (
            <QuodPending team={team.slug} state={ciState(gh.link, false)}>
              Trust follows the steps Quod replays, and running Quod in your CI is in early access.
            </QuodPending>
          )}
        </section>
      )}
      <ProtectedBranchHelp />
    </div>
  );
}

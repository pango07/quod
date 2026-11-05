import Link from "next/link";
import { LiveRefresh, LiveTotals, NowRunning } from "@/components/live/parts";
import { GitHubStatusCard } from "@/components/github-status";
import { QuodNote } from "@/components/section";
import { RangeToggle } from "@/components/range-toggle";
import { RecentCi } from "@/components/savings-summary";
import { overviewCi } from "@/lib/overview-ci";
import { Dot, EmptyState } from "@/components/ui";
import { api } from "@/lib/api";
import { requireTeam } from "@/lib/dal";
import { parseRange, RANGE_FROM } from "@/lib/params";
import { plainUi } from "@/lib/ui";

export const metadata = { title: "Overview" };

export default async function OverviewPage({ params, searchParams }: PageProps<"/[team]">) {
  const [{ team: slug }, sp] = await Promise.all([params, searchParams]);
  const range = parseRange(sp.range);
  const { team, admin } = await requireTeam(slug);
  const [ov, activity, ci] = await Promise.all([api().getOverview(team.slug, range), api().getActivity(team.slug), overviewCi(team, range)]);
  const base = `/${team.slug}`;

  const github = activity.github;
  if (ov.repos.length === 0) {
    return (
      <div className="page">
        <h1 className="h1">Overview</h1>
        <QuodNote status={ov.quod} />
        {/* No repos yet: say how CI gets here (GitHub App), not a dead dashboard. */}
        {github && github.state !== "linked" ? <GitHubStatusCard link={github} admin={admin} /> : null}
        {github?.state === "linked" ? <NowRunning team={team.slug} initial={activity} /> : null}
        {github?.state === "linked" ? (
          <section className="card">
            <EmptyState
              title="No repositories yet"
              action={
                <Link className="btn lg" href={`${base}/settings#github-h`}>
                  Manage GitHub
                </Link>
              }
            >
              GitHub is connected, but the Quod CI App can&apos;t see any repositories. Choose some on GitHub, from Settings.
            </EmptyState>
          </section>
        ) : github ? null : (
          <section className="card">
            <EmptyState
              title="No repositories yet"
              action={
                <Link className="btn solid lg" href={`${base}/connect`}>
                  Connect repository
                </Link>
              }
            >
              Connect a repository and push. The first run only watches; from the second, steps whose inputs did not change come back in under a second.
            </EmptyState>
          </section>
        )}
      </div>
    );
  }

  const t = ov.totals;
  const plain = await plainUi(sp);
  const paused = ov.trust.paused;
  // Repos known only from GitHub have no Quod numbers yet.
  const quod = ov.repos.some((r) => r.hasQuodData !== false);
  return (
    <div className="page">
      <div className="row-between" style={{ gap: 16 }}>
        <h1 className="h1">Overview</h1>
        <div style={{ display: "flex", gap: 10, alignItems: "center", flexWrap: "wrap" }}>
          <RangeToggle base={base} current={range} />
          {/* The sample team's demo flow; a real team connects from the GitHub card or Settings. */}
          {github ? null : (
            <Link className="btn solid" href={`${base}/connect`} style={{ fontSize: 14 }}>
              Connect repository
            </Link>
          )}
        </div>
      </div>

      <QuodNote status={ov.quod} />
      {github && github.state !== "linked" ? <GitHubStatusCard link={github} admin={admin} /> : null}
      <LiveRefresh scope={{ kind: "team" }} />
      <LiveTotals key={range} totals={t} days={ov.days} from={RANGE_FROM[range]} reseed={!activity.simulated} plain={plain} ci={ci} hasQuodData={quod} />
      {paused.length > 0 || t.wrongReplays > 0 ? (
        <Link href={`${base}/trust`} className="notice">
          <Dot tone="amber" />
          <span>A result differed during verification. Affected work runs again. View details.</span>
        </Link>
      ) : null}
      <NowRunning team={team.slug} initial={activity} hideWhenEmpty />
      <RecentCi ci={ci} />
      <div className="row-between" style={{fontSize:14}}>
        <Link href={`${base}/runs`} className="link-muted">All CI runs</Link>
        <Link href={`${base}/repositories`} className="link-muted">Repositories</Link>
      </div>
    </div>
  );
}

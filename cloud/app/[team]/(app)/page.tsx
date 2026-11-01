import Link from "next/link";
import { LiveRefresh, LiveTotals, NowRunning } from "@/components/live/parts";
import { GitHubStatusCard } from "@/components/github-status";
import { RangeToggle } from "@/components/range-toggle";
import { RepoTable } from "@/components/repo-table";
import { Dot, EmptyState } from "@/components/ui";
import { api } from "@/lib/api";
import { requireTeam } from "@/lib/dal";
import { formatCount } from "@/lib/format";
import { parseRange, RANGE_FROM } from "@/lib/params";

export const metadata = { title: "Overview" };

export default async function OverviewPage({ params, searchParams }: PageProps<"/[team]">) {
  const [{ team: slug }, sp] = await Promise.all([params, searchParams]);
  const range = parseRange(sp.range);
  const { team, admin } = await requireTeam(slug);
  const [ov, activity] = await Promise.all([api().getOverview(team.slug, range), api().getActivity(team.slug)]);
  const base = `/${team.slug}`;

  const github = activity.github;
  if (ov.repos.length === 0) {
    return (
      <div className="page">
        <h1 className="h1">Overview</h1>
        {/* No repos yet: say how CI gets here (GitHub App), not a dead dashboard. */}
        {github && github.state !== "linked" ? <GitHubStatusCard link={github} admin={admin} /> : null}
        {github?.state === "linked" ? <NowRunning team={team.slug} initial={activity} /> : null}
        {github && github.state !== "linked" ? null : (
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
  const paused = ov.trust.paused;
  // Repos known only from GitHub have no Quod numbers yet.
  const quod = ov.repos.some((r) => r.hasQuodData !== false);
  return (
    <div className="page">
      <div className="row-between" style={{ gap: 16 }}>
        <h1 className="h1">Overview</h1>
        <div style={{ display: "flex", gap: 10, alignItems: "center", flexWrap: "wrap" }}>
          <RangeToggle base={base} current={range} />
          <Link className="btn solid" href={`${base}/connect`} style={{ fontSize: 14 }}>
            Connect repository
          </Link>
        </div>
      </div>

      {github && github.state !== "linked" ? <GitHubStatusCard link={github} admin={admin} /> : null}
      <NowRunning team={team.slug} initial={activity} />
      <LiveRefresh scope={{ kind: "team" }} />

      {quod ? (
        <LiveTotals key={range} totals={t} days={ov.days} from={RANGE_FROM[range]} reseed={!activity.simulated} />
      ) : (
        <section className="card" style={{ padding: 20 }}>
          <h2 className="h2">Time saved and replays</h2>
          <p className="muted" style={{ fontSize: 14, marginTop: 6, lineHeight: 1.6 }}>
            Appears once Quod runs in one of these repositories. Until then, this page shows their CI from GitHub as it runs.
          </p>
        </section>
      )}

      <div className="split">
        <section className="card" aria-labelledby="repos-h" style={{ flex: "999 1 560px", overflow: "hidden" }}>
          <div className="card-head">
            <h2 id="repos-h" className="h2">
              Repositories
            </h2>
            <Link href={`${base}/repositories`} className="link-muted">
              View all
            </Link>
          </div>
          <RepoTable team={team.slug} repos={ov.repos} asOf={ov.asOf} />
        </section>

        {quod ? (
        <div style={{ flex: "1 1 300px", display: "flex", flexDirection: "column", gap: 16 }}>
          <section className="card" aria-labelledby="trust-h" style={{ padding: 18, display: "flex", flexDirection: "column", gap: 14 }}>
            <div className="row-between">
              <h2 id="trust-h" className="h2">
                Trust
              </h2>
              <Link href={`${base}/trust`} className="link-muted">
                Details
              </Link>
            </div>
            <dl style={{ display: "flex", flexDirection: "column", gap: 10, fontSize: 14, margin: 0 }}>
              {[
                ["Proven steps", ov.trust.proven],
                ["Still proving", ov.trust.proving],
                ["Spot checks passed", ov.trust.spotChecksPassed],
              ].map(([k, v]) => (
                <div key={k} style={{ display: "flex", justifyContent: "space-between" }}>
                  <dt className="muted">{k}</dt>
                  <dd className="mono" style={{ margin: 0 }}>
                    {formatCount(v as number)}
                  </dd>
                </div>
              ))}
            </dl>
            {paused.length > 0 ? (
              <Link href={`${base}/${paused[0].repoSlug}/steps/${paused[0].key}`} className="notice">
                <Dot tone="amber" />
                <span style={{ display: "flex", flexDirection: "column", gap: 2 }}>
                  <span style={{ fontWeight: 500 }}>{paused.length === 1 ? "1 step paused" : `${paused.length} steps paused`}</span>
                  <span className="soft">
                    {paused[0].repoName} · {paused[0].job} / {paused[0].command.replace(/^npx /, "")} gave a different result on a re-check. Quod runs it every time until it is consistent
                    again.
                  </span>
                </span>
              </Link>
            ) : (
              <p className="muted" style={{ fontSize: 13 }}>
                No paused steps. Every spot check matched.
              </p>
            )}
          </section>
          <section className="card" aria-labelledby="runners-h" style={{ padding: 18, display: "flex", flexDirection: "column", gap: 12 }}>
            <div className="row-between">
              <h2 id="runners-h" className="h2">
                Runners
              </h2>
              <Link href={`${base}/connect`} className="link-muted">
                Add
              </Link>
            </div>
            <ul style={{ listStyle: "none", margin: 0, padding: 0, display: "flex", flexDirection: "column", gap: 12 }}>
              {ov.runners.map((r) => (
                <li key={r.name} style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 10, fontSize: 13 }}>
                  <span style={{ display: "flex", alignItems: "center", gap: 8, minWidth: 0 }}>
                    <Dot tone={r.status === "running" ? "blue" : r.status === "offline" ? "dim" : "green"} />
                    <span className="mono ellipsis">{r.name}</span>
                  </span>
                  <span className="faint" style={{ whiteSpace: "nowrap" }}>
                    {r.kind === "github-hosted" ? "via quodci/setup" : `Self-hosted · ${r.status}`}
                  </span>
                </li>
              ))}
            </ul>
          </section>
        </div>
        ) : null}
      </div>
    </div>
  );
}

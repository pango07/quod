import Link from "next/link";
import { StepsTable } from "@/components/steps-table";
import { Dot, EmptyState } from "@/components/ui";
import { api } from "@/lib/api";
import { requireTeam } from "@/lib/dal";
import { parseStepFilter } from "@/lib/params";

export const metadata = { title: "Trust" };

const ORDER = { paused: 0, proving: 1, watching: 2, replaying: 3 } as const;

export default async function TrustPage({ params, searchParams }: PageProps<"/[team]/trust">) {
  const [{ team: slug }, sp] = await Promise.all([params, searchParams]);
  const { team } = await requireTeam(slug);
  const steps = (await api().listSteps(team.slug)).sort((a, b) => ORDER[a.trust.kind] - ORDER[b.trust.kind]);
  const paused = steps.filter((s) => s.trust.kind === "paused");
  return (
    <div className="page">
      <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
        <h1 className="h1">Trust</h1>
        <p className="muted" style={{ maxWidth: "70ch" }}>
          A step replays only after the same inputs gave the same result twice. A few replays are re-run at random and compared. If one ever differs, the step is paused and
          runs every time until it is consistent again.
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
      {steps.length ? (
        <StepsTable team={team.slug} steps={steps} initial={parseStepFilter(sp.state)} showRepo />
      ) : (
        <section className="card">
          <EmptyState title="Nothing to trust yet">Steps appear here after their first run with Quod.</EmptyState>
        </section>
      )}
    </div>
  );
}

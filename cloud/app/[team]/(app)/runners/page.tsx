import Link from "next/link";
import { Dot, EmptyState } from "@/components/ui";
import { api } from "@/lib/api";
import { requireTeam } from "@/lib/dal";

export const metadata = { title: "Runners" };

export default async function RunnersPage({ params }: PageProps<"/[team]/runners">) {
  const { team: slug } = await params;
  const { team } = await requireTeam(slug);
  const runners = await api().listRunners(team.slug);
  return (
    <div className="page">
      <div className="row-between">
        <h1 className="h1">Runners</h1>
        <Link className="btn solid" href={`/${team.slug}/connect?host=own`} style={{ fontSize: 14 }}>
          Add runner
        </Link>
      </div>
      <section className="card" aria-label="Runners" style={{ overflow: "hidden" }}>
        {runners.length ? (
          <ul style={{ listStyle: "none", margin: 0, padding: 0 }}>
            {runners.map((r, i) => (
              <li key={r.name} className="trow" style={{ gridTemplateColumns: "minmax(0, 1fr) auto", borderTop: i ? undefined : 0 }}>
                <span style={{ display: "flex", alignItems: "center", gap: 10, minWidth: 0 }}>
                  <Dot tone={r.status === "running" ? "blue" : r.status === "offline" ? "dim" : "green"} />
                  <span className="mono ellipsis">{r.name}</span>
                </span>
                <span className="faint" style={{ fontSize: 13 }}>
                  {r.kind === "github-hosted" ? "GitHub-hosted, via quodci/setup" : `Self-hosted · ${r.status}`}
                </span>
              </li>
            ))}
          </ul>
        ) : (
          <EmptyState
            title="No runners yet"
            action={
              <Link className="btn solid lg" href={`/${team.slug}/connect`}>
                Connect repository
              </Link>
            }
          >
            Use GitHub-hosted runners with one added step, or install Quod on your own runners.
          </EmptyState>
        )}
      </section>
    </div>
  );
}

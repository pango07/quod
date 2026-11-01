import Link from "next/link";
import { CiEmpty, RunnersSeen, type CiState } from "@/components/github-ci";
import { Dot, EmptyState } from "@/components/ui";
import type { Runner } from "@/lib/api/types";
import type { SeenRunner } from "@/lib/github/pages";

/**
 * Runners: Quod's own runners (once Quod runs; the sample team's are
 * simulated), and the runners seen in the team's CI from GitHub's jobs.
 */
export function RunnersView({ team, asOf, state, quod, seen }: { team: string; asOf: string; state: CiState; quod: Runner[]; seen: SeenRunner[] | null }) {
  return (
    <>
      {quod.length ? (
        <section className="card" aria-labelledby="quod-runners-h" style={{ overflow: "hidden" }}>
          <div className="card-head">
            <h2 id="quod-runners-h" className="h2">
              With Quod
            </h2>
          </div>
          <ul style={{ listStyle: "none", margin: 0, padding: 0 }}>
            {quod.map((r) => (
              <li key={r.name} className="trow" style={{ gridTemplateColumns: "minmax(0, 1fr) auto" }}>
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
        </section>
      ) : null}
      {state === "sample" && !quod.length ? (
        <section className="card">
          <EmptyState
            title="No runners yet"
            action={
              <Link className="btn solid lg" href={`/${team}/connect`}>
                Connect repository
              </Link>
            }
          >
            Use GitHub-hosted runners with one added step, or install Quod on your own runners.
          </EmptyState>
        </section>
      ) : null}
      {state !== "sample" ? (
        <section className="card" aria-labelledby="seen-h" style={{ overflow: "hidden" }}>
          <div className="card-head" style={{ flexDirection: "column", alignItems: "flex-start", gap: 4 }}>
            <h2 id="seen-h" className="h2">
              Seen in your CI
            </h2>
            <p className="muted" style={{ fontSize: 13 }}>
              From your GitHub jobs over the last 7 days. GitHub-hosted runners are fresh machines each time, so they are grouped by image.
            </p>
          </div>
          {seen && seen.length ? <RunnersSeen runners={seen} asOf={asOf} /> : <CiEmpty team={team} state={state === "data" ? "waiting" : state} what="runners" />}
        </section>
      ) : null}
    </>
  );
}

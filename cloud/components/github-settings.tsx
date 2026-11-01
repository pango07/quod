import { connection } from "next/server";
import { DisconnectGitHub } from "@/components/github-disconnect";
import { Chip } from "@/components/ui";
import type { TeamContext } from "@/lib/dal";
import { dbConfigured } from "@/lib/db";
import { githubAppConfig } from "@/lib/github/env";
import { teamInstallations } from "@/lib/github/live";
import { teamIdOf } from "@/lib/github/team";

/** Team settings, "GitHub": the installations linked to this team, Connect, and Disconnect for admins. */
export async function GitHubSettings({ ctx }: { ctx: TeamContext }) {
  await connection(); // live data, per request (Cache Components)
  const ready = githubAppConfig().ok && dbConfigured();
  let installs: Awaited<ReturnType<typeof teamInstallations>> = [];
  let failed = false;
  if (ready) {
    try {
      installs = await teamInstallations(teamIdOf(ctx));
    } catch {
      failed = true;
    }
  }
  return (
    <section className="card" aria-labelledby="github-h" style={{ overflow: "hidden" }}>
      <div className="card-head">
        <h2 id="github-h" className="h2">
          GitHub
        </h2>
        {ready && ctx.admin ? (
          // A plain GET form, not <Link>: /github/install sets a state cookie and leaves for github.com, so it must never be prefetched.
          <form action="/github/install" method="get">
            <button className="btn sm" type="submit">
              Connect GitHub
            </button>
          </form>
        ) : null}
      </div>
      {!ready ? (
        <p className="muted" style={{ padding: "0 20px 20px", fontSize: 13 }}>
          The Quod CI GitHub App isn&apos;t set up on this deploy yet.
        </p>
      ) : failed ? (
        <p className="muted" style={{ padding: "0 20px 20px", fontSize: 13 }}>
          Can&apos;t read the GitHub connections right now. Try again in a minute.
        </p>
      ) : installs.length === 0 ? (
        <p className="muted" style={{ padding: "0 20px 20px", fontSize: 13 }}>
          No GitHub account or organization is connected. {ctx.admin ? "Connect one to see CI jobs live." : "Ask a team admin to connect one."}
        </p>
      ) : (
        <ul style={{ listStyle: "none", margin: 0, padding: 0 }}>
          {installs.map((i) => (
            <li key={i.id} className="trow" style={{ gridTemplateColumns: "minmax(0, 1fr) auto" }}>
              <span className="cell-main">
                <span style={{ fontWeight: 500 }}>{i.account || `Installation ${i.id}`}</span>
                <span className="cell-sub">
                  {i.repos.length} {i.repos.length === 1 ? "repository" : "repositories"}
                  {i.repos.length ? `: ${i.repos.slice(0, 3).map((r) => r.fullName).join(", ")}${i.repos.length > 3 ? ", …" : ""}` : ""}
                </span>
              </span>
              <span style={{ display: "flex", gap: 8, alignItems: "center", flexWrap: "wrap", justifyContent: "flex-end" }}>
                {i.suspended ? <Chip tone="amber">Suspended on GitHub</Chip> : null}
                {ctx.admin ? <DisconnectGitHub team={ctx.team.slug} installationId={i.id} account={i.account || `installation ${i.id}`} /> : null}
              </span>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

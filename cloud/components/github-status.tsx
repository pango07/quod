import type { GitHubLink } from "@/lib/api/types";

/**
 * Overview, for a team whose CI does not reach Quod yet: connect the GitHub
 * App (admins), ask an admin (members), or why live CI is unavailable here.
 */
export function GitHubStatusCard({ link, admin }: { link: GitHubLink; admin: boolean }) {
  if (link.state === "linked") return null;
  const body =
    link.state === "unlinked"
      ? link.canInstall
        ? admin
          ? "Install the Quod CI GitHub App on your repositories. Every job then shows up here as it queues and runs, with or without Quod in it."
          : "An admin of this team can install the Quod CI GitHub App on your repositories. Every job then shows up here as it queues and runs."
        : "The Quod CI GitHub App isn't set up on this deploy yet, so live CI can't be connected here."
      : link.state === "not-configured"
        ? "Live CI needs a database, and this deploy has none (DATABASE_URL is not set). Everything else works."
        : "Live CI is unavailable right now: the database did not answer. Everything else works; try again in a minute.";
  const title = link.state === "unlinked" ? "Connect GitHub to see your CI live" : "Live CI is not available";
  return (
    <section className="card github-cta" aria-labelledby="gh-cta-h" role={link.state === "unavailable" ? "status" : undefined}>
      <div style={{ display: "flex", flexDirection: "column", gap: 6, flex: "1 1 320px", minWidth: 0 }}>
        <h2 id="gh-cta-h" className="h2">
          {title}
        </h2>
        <p className="muted" style={{ fontSize: 14, lineHeight: 1.6 }}>
          {body}
        </p>
      </div>
      {link.state === "unlinked" && link.canInstall ? (
        admin ? (
          // A plain GET form, not <Link>: /github/install sets a state cookie and leaves for github.com, so it must never be prefetched.
          <form action="/github/install" method="get">
            <button className="btn solid lg" type="submit">
              Connect GitHub
            </button>
          </form>
        ) : (
          <span className="chip">Ask an admin</span>
        )
      ) : null}
    </section>
  );
}

import Link from "next/link";
import type { GitHubLink } from "@/lib/api/types";

/**
 * Connect page, step 1: what works today. Connecting the Quod CI GitHub App
 * shows the team's CI live (every job, with Quod or without).
 * `link` is undefined for the sample team, whose CI is simulated.
 */
export function ConnectGitHubStep({ team, link, admin }: { team: string; link: GitHubLink | undefined; admin: boolean }) {
  const manage = `/${team}/settings#github-h`;
  let status: React.ReactNode;
  let action: React.ReactNode = null;

  if (!link) {
    status = "This is the sample team: its CI is simulated. Connect GitHub from your own team to see your real CI here.";
  } else if (link.state === "linked") {
    const accounts = link.accounts.length ? link.accounts.join(", ") : "GitHub";
    status = (
      <>
        <span className="chip green">Connected</span>{" "}
        <span>
          {accounts} · {link.repos === 1 ? "1 repository" : `${link.repos} repositories`}. Every job shows on the dashboard as it queues and runs.
        </span>
      </>
    );
    action = (
      <Link className="btn lg" href={manage}>
        Manage
      </Link>
    );
  } else if (link.state === "unlinked") {
    if (!link.canInstall) {
      status = "The Quod CI GitHub App isn't set up on this deploy yet.";
    } else if (admin) {
      status = "Install the Quod CI GitHub App on the repositories you choose. Every job then shows on the dashboard as it queues and runs, with or without Quod in it.";
      // A plain GET form, not <Link>: /github/install sets a state cookie and leaves for github.com, so it must never be prefetched.
      action = (
        <form action="/github/install" method="get">
          <button className="btn solid lg" type="submit">
            Connect GitHub
          </button>
        </form>
      );
    } else {
      status = "A team admin can install the Quod CI GitHub App on your repositories. Then every job shows on the dashboard as it queues and runs.";
      action = <span className="chip">Ask an admin</span>;
    }
  } else if (link.state === "not-configured") {
    status = "Live CI needs a database, and this deploy has none yet.";
  } else {
    status = "Couldn't check the GitHub connection right now: the database did not answer. Try again in a minute.";
  }

  return (
    <section aria-labelledby="gh-step" className="card" style={{ padding: 20, display: "flex", flexDirection: "column", gap: 12 }}>
      <h2 id="gh-step" className="h2">
        1. Connect GitHub: see your CI live
      </h2>
      <div className="row-between" style={{ alignItems: "center", gap: 16 }}>
        <p className="muted" style={{ fontSize: 14, lineHeight: 1.6, flex: "1 1 320px", minWidth: 0 }}>
          {status}
        </p>
        {action}
      </div>
    </section>
  );
}

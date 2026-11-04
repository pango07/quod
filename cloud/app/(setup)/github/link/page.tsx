import Link from "next/link";
import { connection } from "next/server";
import { AuthShell } from "@/components/auth/shell";
import { getViewer } from "@/lib/auth/session";
import { getTeams, requireTeam } from "@/lib/dal";
import { cancelLink, confirmLink } from "@/lib/github/actions";
import { githubAppConfig } from "@/lib/github/env";
import { REFUSAL_MESSAGE } from "@/lib/github/link";
import { teamInstallations } from "@/lib/github/live";
import { currentTeam, readClaim } from "@/lib/github/server";
import { teamIdOf } from "@/lib/github/team";
import { isSlug, one } from "@/lib/params";

export const metadata = { title: "Connect GitHub" };

const ERRORS: Record<string, string> = {
  // the refusals from lib/github/link.ts, then this page's own
  ...REFUSAL_MESSAGE,
  "pick-team": "You are in more than one Quod team. Open the team you want to connect, then install or configure the App again.",
  "no-installation": "GitHub didn't say which installation this is. Open the App's page on GitHub and choose Configure.",
  state: "That sign-in with GitHub didn't match this browser, or it expired. Try again.",
  expired: "That link step expired (10 minutes). Install or configure the App on GitHub again.",
  github: "GitHub didn't answer as expected. Try again in a minute.",
  db: "Quod Cloud can't reach its database right now. Try again in a minute.",
  "not-configured": "The GitHub App isn't set up on this deploy yet.",
  uninstalled: "This installation was removed on GitHub. Install the App again.",
  denied: "GitHub didn't authorize Quod, so nothing was connected. Try again and choose Authorize.",
};

type SP = Promise<Record<string, string | string[] | undefined>>;

// Where /github/installed lands: the confirm step for a verified
// installation, the result, or why it couldn't be linked.
export default async function GitHubLink({ searchParams }: { searchParams: SP }) {
  await connection();
  const sp = await searchParams;
  const error = one(sp.error);
  const status = one(sp.status);
  const cfg = githubAppConfig();
  // Installs start at Quod (/github/install), which seals a state GitHub hands back (M1).
  const installUrl = cfg.ok ? "/github/install" : null;
  const [viewer, teams] = await Promise.all([getViewer(), getTeams()]);
  const team = currentTeam(viewer, teams);
  // The current team, else the first one; with none, "/" offers "Create my team" (and any invitations).
  const home = team ? `/${team.slug}` : teams[0] ? `/${teams[0].slug}` : "/";

  if (error) {
    return (
      <AuthShell title="GitHub isn't connected" footer={<Link href={home}>Back to the dashboard</Link>}>
        <p className="setup-error" role="alert">
          {ERRORS[error] ?? ERRORS.github}
        </p>
        {installUrl ? (
          <a className="btn lg auth-btn" href={installUrl}>
            Install or configure on GitHub
          </a>
        ) : null}
      </AuthShell>
    );
  }

  if (status === "requested") {
    return (
      <AuthShell title="Request sent" lede="An owner of that GitHub organization has to approve the install. When they do, they come back here to connect it to their Quod team.">
        <Link className="btn solid lg auth-btn" href={home}>
          Back to the dashboard
        </Link>
      </AuthShell>
    );
  }

  if (status === "updated") {
    return (
      <AuthShell title="Changes saved" lede="Repository changes on GitHub reach Quod by webhook within a few seconds.">
        <Link className="btn solid lg auth-btn" href={home}>
          Back to the dashboard
        </Link>
      </AuthShell>
    );
  }

  if (status === "linked") {
    const slug = one(sp.team);
    if (!isSlug(slug)) return <Fallback home={home} installUrl={installUrl} />;
    const ctx = await requireTeam(slug);
    const installationId = Number(one(sp.installation));
    const linked = (await teamInstallations(teamIdOf(ctx))).filter((i) => !Number.isFinite(installationId) || i.id === installationId);
    return (
      <AuthShell title="GitHub is connected" lede={<>CI jobs in these repositories now show live in {ctx.team.name}.</>}>
        {linked.map((i) => (
          <div key={i.id} className="auth-field">
            <span className="muted">
              {i.account} ({i.accountType === "Organization" ? "organization" : "account"})
            </span>
            {i.repos.length ? (
              <ul className="setup-list">
                {i.repos.map((r) => (
                  <li key={r.id}>
                    <span className="mono">{r.fullName}</span>
                    <span className="chip small">{r.private ? "private" : "public"}</span>
                  </li>
                ))}
              </ul>
            ) : (
              <p className="muted">No repositories yet. Add some on GitHub (Configure), and they appear here.</p>
            )}
          </div>
        ))}
        <Link className="btn solid lg auth-btn" href={`/${ctx.team.slug}`}>
          Open {ctx.team.name}
        </Link>
      </AuthShell>
    );
  }

  const claim = await readClaim();
  const claimTeam = claim && claim.userId === viewer.id ? teams.find((t) => t.orgId === claim.teamOrgId) : undefined;
  if (claim && claimTeam) {
    return (
      <AuthShell
        title="Connect GitHub to Quod"
        lede={
          <>
            GitHub user <strong>{claim.githubLogin}</strong> will connect the {claim.account.type === "Organization" ? "organization" : "account"} <strong>{claim.account.login}</strong>. Not you? Cancel.
          </>
        }
      >
        <ul className="setup-list">
          <li>
            <span>Signed in to GitHub as</span>
            <span className="mono">{claim.githubLogin}</span>
          </li>
          <li>
            <span>Connects</span>
            <span className="mono">{claim.account.login}</span>
          </li>
          <li>
            <span>Repositories</span>
            <span className="mono">{claim.repositorySelection === "all" ? `All (${claim.repoCount})` : claim.repoCount}</span>
          </li>
          <li>
            <span>Quod team</span>
            <span className="mono">{claimTeam.name}</span>
          </li>
        </ul>
        <p className="auth-hint">
          Everyone in {claimTeam.name} will see these repositories&apos; CI jobs live. Quod reads workflow runs and jobs only.
        </p>
        <form action={confirmLink} className="auth-form">
          <button type="submit" className="btn solid lg auth-btn">
            Connect to {claimTeam.name}
          </button>
        </form>
        <form action={cancelLink} className="auth-form">
          <button type="submit" className="btn lg auth-btn">
            Cancel
          </button>
        </form>
      </AuthShell>
    );
  }

  return <Fallback home={home} installUrl={installUrl} />;
}

function Fallback({ home, installUrl }: { home: string; installUrl: string | null }) {
  return (
    <AuthShell title="Connect GitHub" lede="Install the Quod CI GitHub App on your account or organization to see your CI jobs live." footer={<Link href={home}>Back to the dashboard</Link>}>
      {installUrl ? (
        <a className="btn solid lg auth-btn" href={installUrl}>
          Install on GitHub
        </a>
      ) : (
        <p className="muted">{ERRORS["not-configured"]}</p>
      )}
    </AuthShell>
  );
}

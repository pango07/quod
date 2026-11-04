import "./(auth)/auth.css";
import { redirect } from "next/navigation";
import { connection } from "next/server";
import { CreateTeamForm, JoinTeamForm } from "@/components/auth/flows";
import { Logo } from "@/components/ui";
import { signOutAction } from "@/lib/actions";
import { getTeams } from "@/lib/dal";
import { pendingInvitationsFor } from "@/lib/auth/directory";
import { getViewer } from "@/lib/auth/session";

// "/" sends you to your team: the one the session is scoped to, else the first.
// With no team (sign-in normally makes one; it failed, or the session predates
// that, or there are invitations): join an invited team, or create your own.
export default async function Home() {
  await connection();
  const [viewer, teams] = await Promise.all([getViewer(), getTeams()]);
  const team = teams.find((t) => t.orgId === viewer.currentOrgId) ?? teams[0];
  if (team) redirect(`/${team.slug}`);
  const invitations = await pendingInvitationsFor(viewer.email);

  if (invitations.length) {
    return (
      <main id="main" className="focus-page" style={{ maxWidth: 520, paddingTop: 96 }}>
        <Logo size={32} />
        <h1 style={{ fontSize: 28, fontWeight: 600, letterSpacing: "-0.02em" }}>You have an invitation</h1>
        <p className="muted" style={{ fontSize: 15, lineHeight: 1.6 }}>
          Signed in as {viewer.email}. Join the team to see its CI.
        </p>
        {invitations.map((inv, i) => (
          <JoinTeamForm key={inv.id} invitationId={inv.id} teamName={inv.teamName} secondary={i > 0} />
        ))}
        <p className="muted" style={{ fontSize: 14, lineHeight: 1.6 }}>
          Not what you expected? Start your own free team instead.
        </p>
        <CreateTeamForm secondary />
      </main>
    );
  }

  return (
    <main id="main" className="focus-page" style={{ maxWidth: 520, paddingTop: 96 }}>
      <Logo size={32} />
      <h1 style={{ fontSize: 28, fontWeight: 600, letterSpacing: "-0.02em" }}>You are not in a team yet</h1>
      <p className="muted" style={{ fontSize: 15, lineHeight: 1.6 }}>
        Signed in as {viewer.email}. Create your own team (free for personal projects) to connect GitHub. To join an existing team instead, ask
        someone on it to invite you from Settings, then open the link in their email.
      </p>
      <CreateTeamForm />
      <form action={signOutAction}>
        <button className="btn" type="submit">
          Sign out
        </button>
      </form>
    </main>
  );
}

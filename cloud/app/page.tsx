import { redirect } from "next/navigation";
import { connection } from "next/server";
import { Logo } from "@/components/ui";
import { signOutAction } from "@/lib/actions";
import { getTeams } from "@/lib/dal";
import { getViewer } from "@/lib/auth/session";

// "/" sends you to your team: the one the session is scoped to, else the first.
export default async function Home() {
  await connection();
  const [viewer, teams] = await Promise.all([getViewer(), getTeams()]);
  const team = teams.find((t) => t.orgId === viewer.currentOrgId) ?? teams[0];
  if (team) redirect(`/${team.slug}`);

  return (
    <main id="main" className="focus-page" style={{ maxWidth: 520, paddingTop: 96 }}>
      <Logo size={32} />
      <h1 style={{ fontSize: 28, fontWeight: 600, letterSpacing: "-0.02em" }}>You are not in a team yet</h1>
      <p className="muted" style={{ fontSize: 15, lineHeight: 1.6 }}>
        Signed in as {viewer.email}. Ask someone on your team to invite you from Settings, then open the link in their email.
      </p>
      <form action={signOutAction}>
        <button className="btn" type="submit">
          Sign out
        </button>
      </form>
    </main>
  );
}

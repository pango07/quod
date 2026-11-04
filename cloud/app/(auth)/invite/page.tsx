import { getWorkOS, withAuth } from "@workos-inc/authkit-nextjs";
import Link from "next/link";
import { connection } from "next/server";
import { Suspense } from "react";
import { JoinTeamForm, SignInFlow, SignUpForm } from "@/components/auth/flows";
import { AuthShell, AuthSkeleton, DemoNote } from "@/components/auth/shell";
import { signOutAction } from "@/lib/actions";
import { isDemo } from "@/lib/auth/pages";
import { checkToken } from "@/lib/auth/validate";
import { one } from "@/lib/params";

export const metadata = { title: "Join a team" };

export default function InvitePage(props: PageProps<"/invite">) {
  return (
    <Suspense fallback={<AuthSkeleton />}>
      <Invite {...props} />
    </Suspense>
  );
}

interface InviteInfo {
  id: string;
  email: string;
  teamName: string;
  open: boolean;
}

async function lookUp(token: string): Promise<InviteInfo | null> {
  if (isDemo()) return { id: "invitation_demo0000000", email: "sam@example.com", teamName: "quodci", open: token !== "expired-preview" };
  try {
    const workos = getWorkOS();
    const inv = await workos.userManagement.findInvitationByToken(token);
    const org = inv.organizationId ? await workos.organizations.getOrganization(inv.organizationId) : null;
    return { id: inv.id, email: inv.email, teamName: org?.name ?? "Quod", open: inv.state === "pending" && new Date(inv.expiresAt).getTime() > Date.now() };
  } catch (e) {
    console.warn("[quod-cloud] invitation lookup failed", { status: (e as { status?: number })?.status });
    return null;
  }
}

// The WorkOS "User invitation URL" points here: /invite?invitation_token=...
async function Invite({ searchParams }: PageProps<"/invite">) {
  await connection(); // reads the session and the invitation per request
  const sp = await searchParams;
  const raw = one(sp.invitation_token) ?? (isDemo() ? "invite-preview-000" : "");
  const token = checkToken(raw);
  const info = token.ok ? await lookUp(token.value) : null;
  const demo = isDemo() ? (
    <DemoNote
      previews={[
        { href: "/invite", label: "new account" },
        { href: "/invite?mode=sign-in", label: "existing account" },
        { href: "/invite?invitation_token=expired-preview", label: "expired" },
      ]}
    />
  ) : null;

  if (!token.ok || !info || !info.open) {
    return (
      <AuthShell title="This invitation can't be used" lede="It may have expired or been used already. Ask the person who invited you to send a new one." aside={demo}>
        <Link className="btn solid lg auth-btn" href="/sign-in">
          Sign in
        </Link>
      </AuthShell>
    );
  }

  // Already signed in: a sign-up form here would make a second account. Join directly, or say whose invitation it is.
  const signedIn = isDemo() ? null : (await withAuth()).user;
  if (signedIn) {
    const mine = signedIn.email.toLowerCase() === info.email.toLowerCase();
    return mine ? (
      <AuthShell title={`Join ${info.teamName} on Quod`} lede={<>You are signed in as <strong style={{ color: "var(--text)" }}>{signedIn.email}</strong>.</>}>
        <JoinTeamForm invitationId={info.id} teamName={info.teamName} />
      </AuthShell>
    ) : (
      <AuthShell
        title="This invitation is for someone else"
        lede={
          <>
            You are signed in as <strong style={{ color: "var(--text)" }}>{signedIn.email}</strong>, and the invitation is for{" "}
            <strong style={{ color: "var(--text)" }}>{info.email}</strong>. Sign out, then open the invitation link again.
          </>
        }
      >
        <form action={signOutAction} className="auth-form">
          <button type="submit" className="btn solid lg auth-btn">
            Sign out
          </button>
        </form>
      </AuthShell>
    );
  }

  const tokenQuery = isDemo() ? "" : `invitation_token=${encodeURIComponent(token.value)}&`;
  const signIn = one(sp.mode) === "sign-in";
  return (
    <AuthShell
      title={`Join ${info.teamName} on Quod`}
      lede={
        <>
          The invitation is for <strong style={{ color: "var(--text)" }}>{info.email}</strong>.
        </>
      }
      footer={
        signIn ? (
          <span>
            New to Quod? <Link href={`/invite?${tokenQuery}`.replace(/[?&]$/, "")}>Create an account</Link>
          </span>
        ) : (
          <span>
            Already have an account? <Link href={`/invite?${tokenQuery}mode=sign-in`}>Sign in to accept</Link>
          </span>
        )
      }
      aside={demo}
    >
      {signIn ? (
        <SignInFlow returnTo="/" invitationToken={token.value} initial={{ step: "password", email: info.email }} lockEmail />
      ) : (
        <SignUpForm returnTo="/" invitationToken={token.value} email={info.email} />
      )}
    </AuthShell>
  );
}

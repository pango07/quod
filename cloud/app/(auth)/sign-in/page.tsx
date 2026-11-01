import Link from "next/link";
import { Suspense } from "react";
import { SignInFlow } from "@/components/auth/flows";
import { Notice } from "@/components/auth/fields";
import { AuthShell, AuthSkeleton, DemoNote } from "@/components/auth/shell";
import type { FormState } from "@/lib/auth/auth-actions";
import { DEMO_EMAIL, isDemo, previewOf, redirectIfSignedIn } from "@/lib/auth/pages";
import { sanitizeReturnTo } from "@/lib/auth/validate";
import { one } from "@/lib/params";

export const metadata = { title: "Sign in" };

export default function SignInPage(props: PageProps<"/sign-in">) {
  return (
    <Suspense fallback={<AuthSkeleton />}>
      <SignIn {...props} />
    </Suspense>
  );
}

const PREVIEWS: Record<string, FormState> = {
  password: { step: "password", email: DEMO_EMAIL },
  "wrong-password": { step: "password", email: DEMO_EMAIL, field: "password", error: "That email and password don't match. Try again, or sign in with a code." },
  code: { step: "code", email: DEMO_EMAIL, notice: `We emailed a 6-digit code to ${DEMO_EMAIL}. It works for 10 minutes.` },
  "rate-limited": { step: "password", email: DEMO_EMAIL, error: "Too many attempts. Wait a minute, then try again." },
};

async function SignIn({ searchParams }: PageProps<"/sign-in">) {
  const sp = await searchParams;
  const returnTo = sanitizeReturnTo(one(sp.returnTo));
  await redirectIfSignedIn(returnTo);
  const preview = previewOf(sp.preview);
  const initial = (preview && PREVIEWS[preview]) || {};
  const error = one(sp.error) === "provider" ? "That sign-in didn't finish. Try again, or use your email." : undefined;
  const signUpHref = returnTo === "/" ? "/sign-up" : `/sign-up?returnTo=${encodeURIComponent(returnTo)}`;
  return (
    <AuthShell
      title="Sign in to Quod"
      footer={
        <span>
          New to Quod? <Link href={signUpHref}>Create an account</Link>
        </span>
      }
      aside={
        isDemo() ? (
          <DemoNote
            previews={[
              { href: "/sign-in", label: "email" },
              ...Object.keys(PREVIEWS).map((k) => ({ href: `/sign-in?preview=${k}`, label: k.replace("-", " ") })),
              { href: "/sign-in/organization", label: "pick an org" },
              { href: "/sign-in/mfa?preview=enroll", label: "set up 2FA" },
              { href: "/sign-in/mfa", label: "2FA code" },
            ]}
          />
        ) : null
      }
    >
      {one(sp.reset) === "1" ? <Notice>Your password is changed. Sign in with it.</Notice> : null}
      {error ? (
        <p role="alert" className="toast err">
          {error}
        </p>
      ) : null}
      <SignInFlow key={preview ?? "live"} returnTo={returnTo} initial={initial} />
    </AuthShell>
  );
}

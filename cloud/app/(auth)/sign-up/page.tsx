import Link from "next/link";
import { Suspense } from "react";
import { SignUpForm } from "@/components/auth/flows";
import { AuthShell, AuthSkeleton, DemoNote } from "@/components/auth/shell";
import type { FormState } from "@/lib/auth/auth-actions";
import { isDemo, previewOf, redirectIfSignedIn } from "@/lib/auth/pages";
import { sanitizeReturnTo } from "@/lib/auth/validate";
import { one } from "@/lib/params";

export const metadata = { title: "Create an account" };

export default function SignUpPage(props: PageProps<"/sign-up">) {
  return (
    <Suspense fallback={<AuthSkeleton />}>
      <SignUp {...props} />
    </Suspense>
  );
}

const PREVIEWS: Record<string, FormState> = {
  "short-password": { field: "password", error: "Use at least 10 characters.", values: { firstName: "Jamie", lastName: "Rivera", email: "jamie@example.com" } },
};

async function SignUp({ searchParams }: PageProps<"/sign-up">) {
  const sp = await searchParams;
  const returnTo = sanitizeReturnTo(one(sp.returnTo));
  await redirectIfSignedIn(returnTo);
  const preview = previewOf(sp.preview);
  const signInHref = returnTo === "/" ? "/sign-in" : `/sign-in?returnTo=${encodeURIComponent(returnTo)}`;
  return (
    <AuthShell
      title="Create your Quod account"
      lede="Free for personal projects. Your CI keeps running as it does today."
      footer={
        <span>
          Already have an account? <Link href={signInHref}>Sign in</Link>
        </span>
      }
      aside={isDemo() ? <DemoNote previews={[{ href: "/sign-up", label: "form" }, { href: "/sign-up?preview=short-password", label: "short password" }, { href: "/verify-email", label: "verify email" }]} /> : null}
    >
      <SignUpForm key={preview ?? "live"} returnTo={returnTo} initial={(preview && PREVIEWS[preview]) || {}} />
    </AuthShell>
  );
}

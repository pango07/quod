import { connection } from "next/server";
import { Suspense } from "react";
import { StepExpired } from "@/components/auth/expired";
import { StartOver, VerifyEmailForm } from "@/components/auth/flows";
import { AuthShell, AuthSkeleton, DemoNote } from "@/components/auth/shell";
import { readPending } from "@/lib/auth/flow";
import { DEMO_EMAIL, isDemo, previewOf } from "@/lib/auth/pages";

export const metadata = { title: "Verify your email" };

export default function VerifyEmailPage(props: PageProps<"/verify-email">) {
  return (
    <Suspense fallback={<AuthSkeleton />}>
      <VerifyEmail {...props} />
    </Suspense>
  );
}

async function VerifyEmail({ searchParams }: PageProps<"/verify-email">) {
  // Per request, never prerendered: unsealing the pending cookie reads the clock (Date.now()),
  // which Next refuses inside a runtime prefetch (seen in production on /verify-email.rsc).
  await connection();
  const sp = await searchParams;
  const preview = previewOf(sp.preview);
  const pending = isDemo() ? { email: DEMO_EMAIL } : await readPending();
  if (!pending || (!isDemo() && "step" in pending && pending.step !== "verify-email")) return <StepExpired />;
  return (
    <AuthShell
      title="Check your email"
      lede={
        <>
          We sent a 6-digit code to <strong style={{ color: "var(--text)" }}>{pending.email ?? "your email"}</strong>. It works for 10 minutes. Enter it to finish.
        </>
      }
      footer={<StartOver />}
      aside={isDemo() ? <DemoNote previews={[{ href: "/verify-email", label: "form" }, { href: "/verify-email?preview=wrong-code", label: "wrong code" }]} /> : null}
    >
      <VerifyEmailForm key={preview ?? "live"} initial={preview === "wrong-code" ? { field: "code", error: "That code didn't work. Check it, or send a new one." } : {}} />
    </AuthShell>
  );
}

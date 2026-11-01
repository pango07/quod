import Link from "next/link";
import { Suspense } from "react";
import { NewPasswordForm, ResetRequestForm } from "@/components/auth/flows";
import { AuthShell, AuthSkeleton, DemoNote } from "@/components/auth/shell";
import { isDemo, previewOf } from "@/lib/auth/pages";
import { checkToken } from "@/lib/auth/validate";
import { one } from "@/lib/params";

export const metadata = { title: "Reset your password" };

export default function ResetPasswordPage(props: PageProps<"/reset-password">) {
  return (
    <Suspense fallback={<AuthSkeleton />}>
      <ResetPassword {...props} />
    </Suspense>
  );
}

// Without ?token=: ask for the email. With it (the link in the email): choose a new password.
async function ResetPassword({ searchParams }: PageProps<"/reset-password">) {
  const sp = await searchParams;
  const preview = previewOf(sp.preview);
  const raw = one(sp.token) ?? (preview === "new" ? "preview-token-000" : undefined);
  const token = raw ? checkToken(raw) : null;
  const demo = isDemo() ? (
    <DemoNote
      previews={[
        { href: "/reset-password", label: "request" },
        { href: "/reset-password?preview=sent", label: "sent" },
        { href: "/reset-password?preview=new", label: "new password" },
      ]}
    />
  ) : null;
  const back = (
    <span>
      Remembered it? <Link href="/sign-in">Sign in</Link>
    </span>
  );

  if (token && !token.ok) {
    return (
      <AuthShell title="This link is not valid" lede="Reset links work once, for 15 minutes." footer={back} aside={demo}>
        <Link className="btn solid lg auth-btn" href="/reset-password">
          Send a new link
        </Link>
      </AuthShell>
    );
  }
  if (token?.ok) {
    return (
      <AuthShell title="Choose a new password" footer={back} aside={demo}>
        <NewPasswordForm token={token.value} />
      </AuthShell>
    );
  }
  return (
    <AuthShell title="Reset your password" lede="Enter your email and we'll send a link to choose a new one." footer={back} aside={demo}>
      <ResetRequestForm
        key={preview ?? "live"}
        initial={preview === "sent" ? { notice: "If jamie@example.com has an account, a link to reset its password is on its way. It works for 15 minutes." } : {}}
      />
    </AuthShell>
  );
}

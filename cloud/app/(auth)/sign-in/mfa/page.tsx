import QRCode from "qrcode";
import { Suspense } from "react";
import { StepExpired } from "@/components/auth/expired";
import { StartOver, TotpForm } from "@/components/auth/flows";
import { AuthShell, AuthSkeleton, DemoNote } from "@/components/auth/shell";
import { readPending, type Pending } from "@/lib/auth/flow";
import { isDemo, previewOf } from "@/lib/auth/pages";

export const metadata = { title: "Two-step verification" };

export default function MfaPage(props: PageProps<"/sign-in/mfa">) {
  return (
    <Suspense fallback={<AuthSkeleton />}>
      <Mfa {...props} />
    </Suspense>
  );
}

// WorkOS answered "mfa_enrollment" (set up an authenticator app first) or "mfa_challenge".
async function Mfa({ searchParams }: PageProps<"/sign-in/mfa">) {
  const sp = await searchParams;
  const preview = previewOf(sp.preview);
  const pending: Pick<Pending, "step" | "totp"> | null = isDemo()
    ? preview === "enroll"
      ? { step: "mfa-enroll", totp: { uri: "otpauth://totp/Quod:jamie%40example.com?secret=JBSWY3DPEHPK3PXP&issuer=Quod", secret: "JBSWY3DPEHPK3PXP" } }
      : { step: "mfa-challenge" }
    : await readPending();
  if (!pending || (pending.step !== "mfa-enroll" && pending.step !== "mfa-challenge")) return <StepExpired />;
  const enroll = pending.step === "mfa-enroll";
  // The cookie holds only the otpauth URI; the QR code is drawn here (an SVG of the URI, no outside service).
  const qr = enroll && pending.totp ? await QRCode.toString(pending.totp.uri, { type: "svg", margin: 1, errorCorrectionLevel: "M" }) : null;
  const demo = isDemo() ? <DemoNote previews={[{ href: "/sign-in/mfa?preview=enroll", label: "set up" }, { href: "/sign-in/mfa", label: "code" }]} /> : null;

  return (
    <AuthShell
      title={enroll ? "Set up two-step verification" : "Enter your code"}
      lede={
        enroll
          ? "Your team asks for a second step. Scan this with an authenticator app (1Password, Google Authenticator, Authy), then enter the code it shows."
          : "Open your authenticator app and enter the 6-digit code for Quod."
      }
      footer={<StartOver />}
      aside={demo}
    >
      {enroll && pending.totp ? (
        <>
          {qr ? <div className="auth-qr" role="img" aria-label="QR code to add Quod to your authenticator app" dangerouslySetInnerHTML={{ __html: qr }} /> : null}
          <p className="auth-secret">
            <span className="sr-only">Or type this key into the app: </span>
            {pending.totp.secret}
          </p>
        </>
      ) : null}
      <TotpForm />
    </AuthShell>
  );
}

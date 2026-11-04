import Link from "next/link";
import { AuthShell } from "./shell";

/** A pending step whose sealed cookie is gone or expired (30 minutes, flow.ts PENDING_TTL). */
export function StepExpired() {
  return (
    <AuthShell
      title="This step expired"
      lede="For your security, sign-in steps last 30 minutes, and only in the browser where they started. Sign in again; if you were invited, open the invitation link from your email again instead."
    >
      <Link className="btn solid lg auth-btn" href="/sign-in">
        Sign in again
      </Link>
    </AuthShell>
  );
}

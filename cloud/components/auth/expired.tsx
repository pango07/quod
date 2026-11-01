import Link from "next/link";
import { AuthShell } from "./shell";

/** A pending step whose sealed cookie is gone or expired (10 minutes). */
export function StepExpired() {
  return (
    <AuthShell title="This step expired" lede="For your security, sign-in steps last 10 minutes.">
      <Link className="btn solid lg auth-btn" href="/sign-in">
        Sign in again
      </Link>
    </AuthShell>
  );
}

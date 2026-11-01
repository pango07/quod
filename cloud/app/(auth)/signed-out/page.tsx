import Link from "next/link";
import { AuthShell } from "@/components/auth/shell";

export const metadata = { title: "Signed out" };

// The WorkOS sign-out redirect lands here. Public: the proxy lets it through.
export default function SignedOut() {
  return (
    <AuthShell title="You are signed out" lede="Your CI keeps running with Quod either way.">
      <Link className="btn solid lg auth-btn" href="/sign-in">
        Sign in again
      </Link>
    </AuthShell>
  );
}

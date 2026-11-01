import { Logo } from "@/components/ui";

export const metadata = { title: "Signed out" };

// The WorkOS sign-out redirect lands here. Public: the proxy lets it through.
export default function SignedOut() {
  return (
    <main id="main" className="focus-page" style={{ maxWidth: 520, paddingTop: 96 }}>
      <Logo size={32} />
      <h1 style={{ fontSize: 28, fontWeight: 600, letterSpacing: "-0.02em" }}>You are signed out</h1>
      <p className="muted" style={{ fontSize: 15 }}>Your CI keeps running with Quod either way.</p>
      <div>
        {/* A plain link, not <Link>: prefetching must not start a sign-in. */}
        {/* eslint-disable-next-line @next/next/no-html-link-for-pages */}
        <a className="btn solid lg" href="/sign-in">
          Sign in again
        </a>
      </div>
    </main>
  );
}

import Link from "next/link";
import { Logo } from "@/components/ui";

export const metadata = { title: "Not found" };

export default function NotFound() {
  return (
    <main id="main" className="focus-page" style={{ maxWidth: 520, paddingTop: 96 }}>
      <Logo size={32} />
      <p className="mono faint">404</p>
      <h1 style={{ fontSize: 28, fontWeight: 600, letterSpacing: "-0.02em" }}>This page does not exist, or it is not yours</h1>
      <p className="muted" style={{ fontSize: 15, lineHeight: 1.6 }}>
        Check the address. If a teammate sent it, they may need to invite you to their team first.
      </p>
      <div>
        <Link className="btn solid lg" href="/">
          Go to your dashboard
        </Link>
      </div>
    </main>
  );
}

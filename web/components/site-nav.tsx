import Image from "next/image";
import Link from "next/link";

export function SiteNav() {
  return (
    <header className="nav-bar">
      <div className="nav">
        <Link href="/" aria-label="Quod home">
          <Image src="/quod-logo.png" alt="Quod" width={1280} height={340} priority className="logo" />
        </Link>
        <nav aria-label="Main">
          <Link href="/#how">How it works</Link>
          <Link href="/under-the-hood">Under the hood</Link>
          <Link href="/#faq">FAQ</Link>
          <Link className="btn small" href="/#join">
            Join the waitlist
          </Link>
        </nav>
      </div>
    </header>
  );
}

export function SiteFooter() {
  return (
    <footer className="footer">
      <Image src="/quod-logo.png" alt="Quod" width={1280} height={340} className="logo" />
      <span>Quod (kwod), from Q.E.D. &copy; 2026</span>
    </footer>
  );
}

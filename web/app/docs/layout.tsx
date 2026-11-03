import type { Metadata, Viewport } from "next";
import Image from "next/image";
import Link from "next/link";
import { MobileNav, Pager, Sidebar } from "@/components/docs/sidebar";
import { THEME_SCRIPT, ThemeToggle } from "@/components/docs/theme-toggle";
import { Toc } from "@/components/docs/toc";
import "./docs.css";

export const metadata: Metadata = {
  title: { template: "%s | Quod Docs", default: "Quod Docs" },
  description: "Quod replays the CI test results whose inputs did not change. Set it up, see how it stays correct, and tune it.",
};

// The docs follow the system theme (or the reader's choice); the marketing site stays dark.
export const viewport: Viewport = {
  colorScheme: "light dark",
  themeColor: [
    { media: "(prefers-color-scheme: light)", color: "#ffffff" },
    { media: "(prefers-color-scheme: dark)", color: "#000000" },
  ],
};

export default function DocsLayout({ children }: LayoutProps<"/docs">) {
  return (
    <div className="docs">
      <script dangerouslySetInnerHTML={{ __html: THEME_SCRIPT }} />
      <a href="#d-main" className="d-skip">
        Skip to content
      </a>
      <header className="d-top">
        <div className="d-top-in">
          <Link href="/" aria-label="Quod home" className="d-brand">
            <Image src="/quod-logo.png" alt="Quod" width={1280} height={340} priority className="d-logo" />
          </Link>
          <Link href="/docs" className="d-brand-docs">
            Docs
          </Link>
          <nav className="d-top-links" aria-label="Site">
            <Link href="/under-the-hood">Under the hood</Link>
            <a href="https://github.com/pango07/quod-action" target="_blank" rel="noreferrer">
              GitHub Action
            </a>
            <a href="https://app.quodci.com" target="_blank" rel="noreferrer">
              Dashboard
            </a>
          </nav>
          <ThemeToggle />
        </div>
        <MobileNav />
      </header>
      <div className="d-shell">
        <aside className="d-side-wrap">
          <Sidebar />
        </aside>
        <main id="d-main" className="d-main">
          <article className="d-article">{children}</article>
          <Pager />
        </main>
        <aside className="d-toc-wrap">
          <Toc />
        </aside>
      </div>
    </div>
  );
}

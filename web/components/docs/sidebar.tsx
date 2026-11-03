"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { useEffect, useRef } from "react";
import { DOCS_FLAT, DOCS_NAV } from "./nav";

function Groups({ path }: { path: string }) {
  return (
    <>
      {DOCS_NAV.map((g) => (
        <div key={g.title} className="d-nav-group">
          <p className="d-nav-title">{g.title}</p>
          <ul>
            {g.links.map((l) => (
              <li key={l.href}>
                <Link href={l.href} aria-current={path === l.href ? "page" : undefined}>
                  {l.title}
                </Link>
              </li>
            ))}
          </ul>
        </div>
      ))}
    </>
  );
}

/** The left sidebar on wide screens. */
export function Sidebar() {
  const path = usePathname();
  return (
    <nav className="d-side" aria-label="Docs">
      <Groups path={path} />
    </nav>
  );
}

/** The menu on phones: a disclosure under the top bar that closes on navigation. */
export function MobileNav() {
  const path = usePathname();
  const ref = useRef<HTMLDetailsElement>(null);
  useEffect(() => {
    if (ref.current) ref.current.open = false;
  }, [path]);
  const here = DOCS_FLAT.find((l) => l.href === path)?.title ?? "Menu";
  return (
    <details ref={ref} className="d-mnav">
      <summary>
        <span className="d-mnav-label">Menu</span>
        <span className="d-mnav-here">{here}</span>
      </summary>
      <nav aria-label="Docs">
        <Groups path={path} />
      </nav>
    </details>
  );
}

/** Previous and next page, in sidebar order. */
export function Pager() {
  const path = usePathname();
  const i = DOCS_FLAT.findIndex((l) => l.href === path);
  if (i < 0) return null;
  const prev = DOCS_FLAT[i - 1];
  const next = DOCS_FLAT[i + 1];
  return (
    <nav className="d-pager" aria-label="Previous and next">
      {prev ? (
        <Link href={prev.href}>
          <span>Previous</span>
          {prev.title}
        </Link>
      ) : (
        <span />
      )}
      {next ? (
        <Link href={next.href} className="next">
          <span>Next</span>
          {next.title}
        </Link>
      ) : null}
    </nav>
  );
}

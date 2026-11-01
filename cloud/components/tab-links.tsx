"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";

export function TabLinks({ label, tabs }: { label: string; tabs: { href: string; label: string }[] }) {
  const path = usePathname();
  return (
    <nav className="tabs" aria-label={label}>
      {tabs.map((t) => (
        <Link key={t.href} href={t.href} aria-current={path === t.href ? "page" : undefined}>
          {t.label}
        </Link>
      ))}
    </nav>
  );
}

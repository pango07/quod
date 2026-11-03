"use client";

import { usePathname } from "next/navigation";
import { useEffect, useState } from "react";

type Item = { id: string; text: string; depth: number };

/** "On this page": the article's h2 and h3 headings, with the one in view highlighted. */
export function Toc() {
  const path = usePathname();
  const [items, setItems] = useState<Item[]>([]);
  const [active, setActive] = useState<string>("");

  useEffect(() => {
    const hs = Array.from(document.querySelectorAll<HTMLElement>(".d-article h2[id], .d-article h3[id]"));
    setItems(hs.map((h) => ({ id: h.id, text: (h.textContent ?? "").replace(/#$/, "").trim(), depth: h.tagName === "H3" ? 3 : 2 })));
    setActive(hs[0]?.id ?? "");
    const io = new IntersectionObserver(
      (entries) => {
        const vis = entries.filter((e) => e.isIntersecting).sort((a, b) => a.boundingClientRect.top - b.boundingClientRect.top);
        if (vis[0]) setActive(vis[0].target.id);
      },
      { rootMargin: "-72px 0px -70% 0px" },
    );
    hs.forEach((h) => io.observe(h));
    return () => io.disconnect();
  }, [path]);

  if (items.length < 2) return null;
  return (
    <nav className="d-toc" aria-label="On this page">
      <p className="d-nav-title">On this page</p>
      <ul>
        {items.map((it) => (
          <li key={it.id} className={it.depth === 3 ? "sub" : undefined}>
            <a href={`#${it.id}`} aria-current={active === it.id ? "location" : undefined}>
              {it.text}
            </a>
          </li>
        ))}
      </ul>
    </nav>
  );
}

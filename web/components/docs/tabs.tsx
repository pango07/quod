"use client";

import { Children, isValidElement, useEffect, useId, useState, type ReactNode } from "react";

const EVENT = "quod-docs-tab";
const key = (group: string) => `quod-docs-tab:${group}`;

/**
 * Tabs over code blocks (or any content): one child per item, in order.
 * Tabs with the same `group` (e.g. "pm" for package managers) stay in step across the page
 * and are remembered for the next visit.
 */
export function Tabs({ items, group, children }: { items: string[]; group?: string; children: ReactNode }) {
  const panels = Children.toArray(children).filter(isValidElement);
  const [i, setI] = useState(0);
  const id = useId();

  useEffect(() => {
    if (!group) return;
    const apply = (label: string | null) => {
      const n = label ? items.indexOf(label) : -1;
      if (n >= 0) setI(n);
    };
    try {
      apply(localStorage.getItem(key(group)));
    } catch {}
    const on = (e: Event) => {
      const d = (e as CustomEvent<{ group: string; label: string }>).detail;
      if (d.group === group) apply(d.label);
    };
    window.addEventListener(EVENT, on);
    return () => window.removeEventListener(EVENT, on);
  }, [group, items]);

  const pick = (n: number) => {
    setI(n);
    if (!group) return;
    try {
      localStorage.setItem(key(group), items[n]);
    } catch {}
    window.dispatchEvent(new CustomEvent(EVENT, { detail: { group, label: items[n] } }));
  };

  return (
    <div className="d-tabs">
      <div className="d-tablist" role="tablist">
        {items.map((label, n) => (
          <button
            key={label}
            role="tab"
            type="button"
            id={`${id}-t${n}`}
            aria-selected={n === i}
            aria-controls={`${id}-p${n}`}
            tabIndex={n === i ? 0 : -1}
            onClick={() => pick(n)}
            onKeyDown={(e) => {
              if (e.key !== "ArrowRight" && e.key !== "ArrowLeft") return;
              const m = (n + (e.key === "ArrowRight" ? 1 : items.length - 1)) % items.length;
              pick(m);
              document.getElementById(`${id}-t${m}`)?.focus();
            }}
          >
            {label}
          </button>
        ))}
      </div>
      {panels.map((p, n) => (
        <div key={n} role="tabpanel" id={`${id}-p${n}`} aria-labelledby={`${id}-t${n}`} hidden={n !== i} className="d-tabpanel">
          {p}
        </div>
      ))}
    </div>
  );
}

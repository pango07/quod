"use client";

import { useEffect, useId, useRef, useState, type ReactNode } from "react";

/** A button and a menu below it: closes on Escape (focus back on the button) and on a click outside. */
export function Popover({
  label,
  button,
  buttonClass,
  align = "left",
  children,
}: {
  label: string;
  button: ReactNode;
  buttonClass?: string;
  align?: "left" | "right";
  children: (close: () => void) => ReactNode;
}) {
  const [open, setOpen] = useState(false);
  const wrap = useRef<HTMLDivElement>(null);
  const btn = useRef<HTMLButtonElement>(null);
  const id = useId();

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (!wrap.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setOpen(false);
        btn.current?.focus();
      }
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        const items = [...(wrap.current?.querySelectorAll<HTMLElement>(".menu-item") ?? [])];
        if (!items.length) return;
        e.preventDefault();
        const at = items.indexOf(document.activeElement as HTMLElement);
        const next = e.key === "ArrowDown" ? (at + 1) % items.length : (at - 1 + items.length) % items.length;
        items[next].focus();
      }
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    wrap.current?.querySelector<HTMLElement>(".menu-item")?.focus();
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  return (
    <div ref={wrap} style={{ position: "relative", display: "flex" }}>
      <button
        ref={btn}
        type="button"
        className={buttonClass}
        aria-label={label}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? id : undefined}
        onClick={() => setOpen((o) => !o)}
      >
        {button}
      </button>
      {open ? (
        <div id={id} role="menu" aria-label={label} className="menu" style={{ top: "calc(100% + 8px)", [align]: 0 }}>
          {children(() => setOpen(false))}
        </div>
      ) : null}
    </div>
  );
}

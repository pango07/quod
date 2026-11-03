"use client";

import { useEffect, useState } from "react";

type Mode = "system" | "light" | "dark";
const KEY = "quod-docs-theme";

/** Runs before paint (inline in the docs layout) so the chosen theme never flashes. */
export const THEME_SCRIPT = `try{var m=localStorage.getItem("${KEY}");if(m==="light"||m==="dark")document.documentElement.dataset.theme=m}catch(e){}`;

function apply(m: Mode) {
  const el = document.documentElement;
  if (m === "system") delete el.dataset.theme;
  else el.dataset.theme = m;
  try {
    if (m === "system") localStorage.removeItem(KEY);
    else localStorage.setItem(KEY, m);
  } catch {}
}

const LABEL: Record<Mode, string> = { system: "System", light: "Light", dark: "Dark" };

/** System, light or dark. */
export function ThemeToggle() {
  const [mode, setMode] = useState<Mode>("system");
  useEffect(() => {
    // Also covers a client-side navigation into the docs, where the inline script never ran.
    let t: string | null | undefined = document.documentElement.dataset.theme;
    try {
      t = localStorage.getItem(KEY) ?? t;
    } catch {}
    if (t === "light" || t === "dark") {
      setMode(t);
      document.documentElement.dataset.theme = t;
    }
    // Leaving the docs: the marketing site is dark only.
    return () => {
      delete document.documentElement.dataset.theme;
    };
  }, []);
  return (
    <div className="d-theme" role="radiogroup" aria-label="Theme">
      {(["system", "light", "dark"] as Mode[]).map((m) => (
        <button
          key={m}
          type="button"
          role="radio"
          aria-checked={mode === m}
          aria-label={LABEL[m]}
          title={LABEL[m]}
          onClick={() => {
            setMode(m);
            apply(m);
          }}
        >
          {m === "system" ? (
            <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
              <rect x="1.5" y="2.5" width="13" height="9" rx="1.5" fill="none" stroke="currentColor" strokeWidth="1.3" />
              <path d="M5.5 14h5" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" />
            </svg>
          ) : m === "light" ? (
            <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
              <circle cx="8" cy="8" r="3" fill="none" stroke="currentColor" strokeWidth="1.3" />
              <path d="M8 1v1.6M8 13.4V15M1 8h1.6M13.4 8H15M3 3l1.1 1.1M11.9 11.9 13 13M3 13l1.1-1.1M11.9 4.1 13 3" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" />
            </svg>
          ) : (
            <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
              <path d="M13.5 9.6A5.6 5.6 0 0 1 6.4 2.5a5.6 5.6 0 1 0 7.1 7.1Z" fill="none" stroke="currentColor" strokeWidth="1.3" strokeLinejoin="round" />
            </svg>
          )}
        </button>
      ))}
    </div>
  );
}

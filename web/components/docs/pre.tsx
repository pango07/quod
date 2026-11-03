"use client";

import { useRef, useState, type ComponentProps } from "react";

/** A highlighted code block (from rehype-pretty-code) with a copy button. */
export function Pre(props: ComponentProps<"pre">) {
  const ref = useRef<HTMLPreElement>(null);
  const [done, setDone] = useState(false);
  const copy = async () => {
    const text = ref.current?.innerText.replace(/\n$/, "") ?? "";
    try {
      await navigator.clipboard.writeText(text);
      setDone(true);
      setTimeout(() => setDone(false), 1600);
    } catch {}
  };
  return (
    <div className="d-pre">
      <pre ref={ref} {...props} />
      <button type="button" className="d-copy" onClick={copy} aria-label={done ? "Copied" : "Copy code"}>
        {done ? (
          <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
            <path d="M3 8.5l3 3 7-7" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" />
          </svg>
        ) : (
          <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
            <rect x="5.5" y="5.5" width="8" height="8" rx="1.5" fill="none" stroke="currentColor" strokeWidth="1.4" />
            <path d="M10.5 3.5v-.5A1.5 1.5 0 0 0 9 1.5H4A1.5 1.5 0 0 0 2.5 3v5A1.5 1.5 0 0 0 4 9.5h.5" fill="none" stroke="currentColor" strokeWidth="1.4" />
          </svg>
        )}
      </button>
    </div>
  );
}

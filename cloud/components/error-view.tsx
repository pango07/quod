"use client";

import Link from "next/link";
import { useEffect } from "react";

/** Shared by the error boundaries: says what happened in plain words and offers a retry. */
export function ErrorView({ error, retry, inShell }: { error: unknown; retry: () => void; inShell?: boolean }) {
  useEffect(() => {
    console.error(error);
  }, [error]);
  const digest = error instanceof Error && "digest" in error ? String((error as { digest?: string }).digest ?? "") : "";
  return (
    <div className={inShell ? "page" : "focus-page"} style={inShell ? undefined : { maxWidth: 520, paddingTop: 96 }} role="alert">
      <h1 style={{ fontSize: 24, fontWeight: 600, letterSpacing: "-0.02em" }}>Something went wrong loading this page</h1>
      <p className="muted" style={{ fontSize: 15, lineHeight: 1.6, maxWidth: "60ch" }}>
        Your CI is not affected: Quod Cloud only shows results. Try again; if it keeps failing, send us the reference below.
      </p>
      {digest ? <p className="mono faint">Reference: {digest}</p> : null}
      <div style={{ display: "flex", gap: 8 }}>
        <button className="btn solid" type="button" onClick={() => retry()}>
          Try again
        </button>
        <Link className="btn" href="/">
          Go to your dashboard
        </Link>
      </div>
    </div>
  );
}

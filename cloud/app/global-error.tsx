"use client";

// Replaces the root layout when it fails, so it brings its own document and styles.
export default function GlobalError({ error, retry }: { error: unknown; retry: () => void }) {
  const digest = error instanceof Error && "digest" in error ? String((error as { digest?: string }).digest ?? "") : "";
  return (
    <html lang="en">
      <body style={{ margin: 0, background: "#000", color: "#ededed", font: "15px/1.6 system-ui, sans-serif" }}>
        <title>Quod Cloud: something went wrong</title>
        <main style={{ maxWidth: 520, margin: "0 auto", padding: "96px 16px", display: "flex", flexDirection: "column", gap: 16 }} role="alert">
          <h1 style={{ fontSize: 24, margin: 0 }}>Quod Cloud could not load</h1>
          <p style={{ color: "#a1a1a1", margin: 0 }}>Your CI is not affected: Quod Cloud only shows results. Try again in a moment.</p>
          {digest ? <p style={{ color: "#8f8f8f", fontFamily: "ui-monospace, monospace", margin: 0 }}>Reference: {digest}</p> : null}
          <div>
            <button
              type="button"
              onClick={() => retry()}
              style={{ background: "#ededed", color: "#000", border: 0, borderRadius: 8, padding: "9px 14px", font: "500 14px system-ui, sans-serif", cursor: "pointer" }}
            >
              Try again
            </button>
          </div>
        </main>
      </body>
    </html>
  );
}

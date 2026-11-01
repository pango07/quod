"use client";

import { useEffect, useState } from "react";
import { sleep, useInView, useReducedMotion } from "@/lib/use-in-view";

type Row = { k: "f" | "d" | "a"; p: string; h: string; note: string };

// A trimmed manifest from `go test ./...` on the tiny demo module.
const ROWS: Row[] = [
  { k: "f", p: "go.mod", h: "b3:7c1e…", note: "read: the module path and Go version" },
  { k: "a", p: "go.work", h: "absent", note: "looked for, not found: no workspace file" },
  { k: "d", p: "calc", h: "b3:19af…", note: "listed: go test globs *.go here" },
  { k: "f", p: "calc/calc.go", h: "b3:4d02…", note: "read: the code under test" },
  { k: "f", p: "calc/calc_test.go", h: "b3:a9b8…", note: "read: the tests" },
  { k: "a", p: "vendor", h: "absent", note: "looked for, not found: no vendored modules" },
];

// Each change, what it does to the tree, and what `matches` says. Messages follow quod-manifest's Display.
const CHANGES: { label: string; row: number | null; result: string; replay: boolean }[] = [
  {
    label: "Edit README.md",
    row: null,
    result: "match: README.md isn't in the manifest, the step never read it",
    replay: true,
  },
  {
    label: "touch calc/calc.go",
    row: null,
    result: "match: same bytes, same hash, same served mtime",
    replay: true,
  },
  {
    label: "Change calc/calc.go",
    row: 3,
    result: "calc/calc.go: file content changed (file b3:4d02… mode 644 -> file b3:e310… mode 644)",
    replay: false,
  },
  {
    label: "Create go.work",
    row: 1,
    result: "go.work: was absent, now present (absent -> file b3:0b5c… mode 644)",
    replay: false,
  },
  {
    label: "Add calc/extra.go",
    row: 2,
    result: "calc: directory listing changed (directory listing b3:19af… -> directory listing b3:c4d1…)",
    replay: false,
  },
];

export function ManifestDemo() {
  const [ref, inView] = useInView<HTMLDivElement>(0.4);
  const reduce = useReducedMotion();
  const [on, setOn] = useState(-1);
  const [held, setHeld] = useState(false);

  useEffect(() => {
    if (!inView || held || reduce) return;
    const ctl = new AbortController();
    (async () => {
      for (;;) {
        for (let i = 0; i < CHANGES.length; i++) {
          setOn(i);
          await sleep(3400, ctl.signal);
        }
      }
    })().catch(() => {});
    return () => ctl.abort();
  }, [inView, held, reduce]);

  const c = on >= 0 ? CHANGES[on] : null;
  return (
    <div className="mf" ref={ref}>
      <div className="seg mf-changes" role="group" aria-label="Try a change">
        {CHANGES.map((ch, i) => (
          <button
            key={ch.label}
            type="button"
            aria-pressed={i === on}
            onClick={() => {
              setHeld(true);
              setOn(i);
            }}
          >
            {ch.label}
          </button>
        ))}
      </div>
      <table className="mf-table">
        <thead>
          <tr>
            <th scope="col">k</th>
            <th scope="col">path</th>
            <th scope="col">recorded</th>
            <th scope="col" className="mf-note-h">why it&rsquo;s here</th>
          </tr>
        </thead>
        <tbody>
          {ROWS.map((r, i) => (
            <tr key={r.p} className={c && c.row === i ? "bad" : ""}>
              <td className="mf-k">{r.k}</td>
              <td>{r.p}</td>
              <td className="faint">{r.h}</td>
              <td className="mf-note">{r.note}</td>
            </tr>
          ))}
        </tbody>
      </table>
      <p className={`mf-result ${c ? (c.replay ? "ok" : "no") : ""}`} aria-live="polite">
        {c ? (
          <>
            <b>{c.replay ? "replay" : "run"}</b> {c.result}
          </>
        ) : (
          "Pick a change to see what Quod decides."
        )}
      </p>
    </div>
  );
}

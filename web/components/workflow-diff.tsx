"use client";

import { useState } from "react";

const BEFORE = `jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7
      - uses: actions/setup-node@v7
        with:
          node-version: 24.18.0
      - run: npm ci
      - run: npm test`;
const AFTER = BEFORE.replace("    steps:", "    permissions:\n      contents: read\n      id-token: write\n    steps:")
  .replace("      - run: npm ci", "      - uses: pango07/quod-action@v2\n      - run: npm ci");

export function WorkflowDiff() {
  const [applied, setApplied] = useState(true);
  const [copied, setCopied] = useState("");
  return (
    <div className="workflow-demo">
      <div className="demo-toolbar">
        <div className="seg" role="group" aria-label="Compare workflow setup">
          <button type="button" aria-pressed={!applied} onClick={() => { setApplied(false); setCopied(""); }}>Before</button>
          <button type="button" aria-pressed={applied} onClick={() => { setApplied(true); setCopied(""); }}>With Quod</button>
        </div>
        <button type="button" className="demo-control" onClick={async () => {
          try { await navigator.clipboard.writeText(applied ? AFTER : BEFORE); setCopied("Copied"); }
          catch { setCopied("Select the YAML below to copy"); }
        }}>Copy YAML</button>
      </div>
      <div className={`diff ${applied ? "applied" : ""}`} aria-label="Workflow YAML" tabIndex={0}>
        {(applied ? AFTER : BEFORE).split("\n").map((line, i) => (
          <div key={i} className={applied && /quod-action|permissions:|contents:|id-token:/.test(line) ? "ad" : "ctx"}>{line}</div>
        ))}
      </div>
      <p className="foot-note" role="status">{copied || (applied ? "Added: Quod and the permissions it uses to connect to Quod Cloud." : "Your existing Node and Vitest workflow.")}</p>
    </div>
  );
}

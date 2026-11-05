"use client";

import { useState } from "react";

const ROWS = [
  { kind: "file", path: "package.json", note: "Shared configuration" },
  { kind: "file", path: "calc/calc.ts", note: "Imported source" },
  { kind: "file", path: "calc/calc.test.ts", note: "Test file" },
  { kind: "dir", path: "calc/testdata/cases", note: "Observed directory listing" },
  { kind: "absent", path: "calc/testdata/override.json", note: "Looked for, not found" },
];
const CHANGES = [
  { label: "Edit README.md", row: -1, replay: true, result: "The recorded inputs still match. This example did not read README.md." },
  { label: "Change source", row: 1, replay: false, result: "calc/calc.ts changed. The affected test file needs to run again." },
  { label: "Change a test", row: 2, replay: false, result: "The test file itself changed. Its previous result cannot be reused." },
  { label: "Add a fixture", row: 3, replay: false, result: "The directory listing changed. A newly discovered fixture can change the result." },
  { label: "Create missing config", row: 4, replay: false, result: "A file that was absent now exists. The previous observation no longer matches." },
];

export function ManifestDemo() {
  const [on, setOn] = useState(0);
  const change = CHANGES[on];
  return (
    <div className="mf">
      <div className="seg mf-changes" role="group" aria-label="Try an input change">
        {CHANGES.map((item, i) => <button key={item.label} type="button" aria-pressed={i === on} onClick={() => setOn(i)}>{item.label}</button>)}
      </div>
      <div className="mf-scroll" tabIndex={0} aria-label="Illustrative recorded inputs">
        <table className="mf-table">
          <thead><tr><th scope="col">kind</th><th scope="col">recorded input</th><th scope="col" className="mf-note-h">why it matters</th></tr></thead>
          <tbody>{ROWS.map((row, i) => <tr key={row.path} className={change.row === i ? "bad" : ""}><td className="mf-k">{row.kind}</td><td>{row.path}</td><td className="mf-note">{row.note}</td></tr>)}</tbody>
        </table>
      </div>
      <p className={`mf-result ${change.replay ? "ok" : "no"}`} aria-live="polite"><b>{change.replay ? "eligible for reuse" : "run again"}</b> {change.result}</p>
      <p className="demo-note">Illustrative inputs. Reuse also requires a verified result and no audit due.</p>
    </div>
  );
}

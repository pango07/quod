"use client";

import { useState } from "react";
import { ManifestDemo } from "@/components/manifest-demo";
import { Terminal } from "@/components/terminal";
import { TrustMeter } from "@/components/trust-meter";

const VIEWS = [
  { id: "terminal", label: "Try the terminal", description: "Run, edit, repeat. Type commands or use the shortcuts below." },
  { id: "inputs", label: "Change an input", description: "Pick a change to see why a recorded input makes a difference." },
  { id: "verification", label: "Test an audit", description: "Build trust, reuse a result, then see what happens when an audit disagrees." },
] as const;

export function Playground() {
  const [on, setOn] = useState(0);
  return (
    <div className="playground">
      <div className="seg playground-switch" role="group" aria-label="Explore Quod">
        {VIEWS.map((view, i) => <button type="button" key={view.id} aria-pressed={on === i} aria-controls={`playground-${view.id}`} onClick={() => setOn(i)}>{view.label}</button>)}
      </div>
      <p className="foot-note">{VIEWS[on].description}</p>
      <div id="playground-terminal" hidden={on !== 0}><Terminal id="term-try" /></div>
      <div id="playground-inputs" hidden={on !== 1}><ManifestDemo /></div>
      <div id="playground-verification" hidden={on !== 2}><TrustMeter /></div>
      <p className="foot-note">Interactive examples, running locally in your browser. They model selected behaviors; they do not execute Vitest or measure CI speed. The GitHub benchmark above contains the real measurements.</p>
    </div>
  );
}

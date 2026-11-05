"use client";

import { useReducer } from "react";

type State = { stage: "new" | "recorded" | "verified" | "quarantined"; audits: number; log: string };
const initial: State = { stage: "new", audits: 0, log: "Start with a fresh test file. Run it to record the example result." };
function update(state: State, action: "run" | "audit" | "disagree" | "reset"): State {
  if (action === "reset") return initial;
  if (action === "disagree") return { stage: "quarantined", audits: state.audits + 1, log: "The audit disagreed. Reuse stops for this file; it needs investigation. A lineage audit also revokes trust for its promotion batch." };
  if (action === "audit") return { ...state, audits: state.audits + 1, log: "Audit: executed again, and the per-test states agreed. The record remains eligible for reuse." };
  if (state.stage === "new") return { ...state, stage: "recorded", log: "Recorded. A fresh result waits for another agreeing execution." };
  if (state.stage === "recorded") return { ...state, stage: "verified", log: "Confirmed. The outcomes agree; unchanged inputs can now qualify for reuse." };
  if (state.stage === "quarantined") return { ...state, log: "Executed again. This file stays quarantined in the example; a passing run alone does not clear it." };
  return { ...state, log: "Reused the verified result. The recorded inputs matched and no audit was due." };
}

export function TrustMeter() {
  const [state, dispatch] = useReducer(update, initial);
  const verified = state.stage === "verified";
  const progress = state.stage === "recorded" ? 50 : verified ? 100 : 0;
  return (
    <div className="meter">
      <div className="meter-top"><span className="mono">calc/calc.test.ts</span><span className={`state ${verified ? "on" : ""}`}>{state.stage}</span></div>
      <div className="bar" aria-hidden><i style={{ width: `${progress}%` }} /></div>
      <div className="meter-row mono"><span>Illustrative verification cycle</span><span>{state.audits} audits</span></div>
      <p className="meter-log" aria-live="polite">{state.log}</p>
      <div className="chips">
        <button type="button" onClick={() => dispatch("run")}>Run next job</button>
        <button type="button" disabled={!verified} onClick={() => dispatch("audit")}>Audit: same result</button>
        <button type="button" disabled={!verified} onClick={() => dispatch("disagree")}>Audit: different result</button>
        <button type="button" onClick={() => dispatch("reset")}>Reset verification</button>
      </div>
      <p className="foot-note">A simplified example. Actual eligibility and audit policies depend on the configuration.</p>
    </div>
  );
}

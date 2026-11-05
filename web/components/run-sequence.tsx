"use client";

import { useState } from "react";
import { benchmark } from "@/lib/benchmark";

const STAGES = [
  { title: "Find the test file", body: "Quod tracks a test file within its Vitest project. Shared configuration, tool versions and recorder compatibility also affect reuse.", example: "Project + test file + compatible recording" },
  { title: "Check its inputs", body: "Compare recorded file, directory and environment observations with their current states. A changed input makes the affected file execute again.", example: "calc.ts changed → calc.test.ts runs" },
  { title: "Check eligibility", body: "Matching inputs alone are not enough. The result needs verification, must meet the configured replay policy and must not be quarantined.", example: "Matching inputs + verified result + eligible file" },
  { title: "Reuse or execute", body: "An eligible unchanged file can reuse its result. New, changed and ineligible files execute. Files selected for an audit also execute.", example: "An audit due → execute, even when the inputs match" },
  { title: "Compare outcomes", body: "Confirmation and audit runs compare per-test states. The benchmark also checks each job against plain CI, including test names, counts and the job conclusion.", example: "Compare each test, not just the final green check" },
  { title: "Handle disagreement", body: "An audit disagreement quarantines the file. A failed lineage audit also revokes trust for every record with the same promotion stamp.", example: "Disagreement → stop reuse and revoke related trust" },
];

export function RunSequence() {
  const [on, setOn] = useState(0);
  const stage = STAGES[on];
  return (
    <div className="seq">
      <ol className="seq-list">
        {STAGES.map((item, i) => <li key={item.title}><button type="button" aria-current={i === on ? "step" : undefined} className={i === on ? "on" : i < on ? "past" : ""} onClick={() => setOn(i)}><span className="n">{i + 1}</span>{item.title}</button></li>)}
      </ol>
      <div className="seq-detail" aria-live="polite">
        <h3>{stage.title}</h3><p className="seq-body">{stage.body}</p>
        <p className="sequence-example mono">{stage.example}</p>
        <a className="text-link" href={benchmark.support}>Read the implementation boundaries ↗</a>
      </div>
    </div>
  );
}

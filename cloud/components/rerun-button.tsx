"use client";

import { useState, useTransition } from "react";
import { rerunWithoutQuod, type ActionResult } from "@/lib/actions";

/** Asks for a fresh run with every step executed, to compare against. */
export function RerunButton({ team, repo, run }: { team: string; repo: string; run: number }) {
  const [pending, start] = useTransition();
  const [result, setResult] = useState<ActionResult>(null);
  return (
    <>
      <button
        type="button"
        className="btn"
        disabled={pending}
        aria-describedby={result ? `rerun-${run}` : undefined}
        onClick={() => start(async () => setResult(await rerunWithoutQuod(team, repo, run)))}
      >
        {pending ? "Requesting…" : "Re-run without Quod"}
      </button>
      {result ? (
        <p id={`rerun-${run}`} role="status" className={`toast ${result.ok ? "ok" : "err"}`} style={{ flexBasis: "100%" }}>
          {result.message}
        </p>
      ) : null}
    </>
  );
}

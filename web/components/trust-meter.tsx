"use client";

import { useEffect, useReducer } from "react";
import { sleep, useInView, useReducedMotion } from "@/lib/use-in-view";

type State = { proofs: number; audits: number; quarantined: boolean; runs: number; log: string };
type Action = { type: "run" } | { type: "flake" } | { type: "reset" };

const start: State = { proofs: 0, audits: 0, quarantined: false, runs: 0, log: "Every step starts in observe mode." };
const need = (s: State) => 2 + (s.quarantined ? 1 : 0);

function reduce(s: State, a: Action): State {
  if (a.type === "reset") return start;
  if (a.type === "flake")
    return {
      ...s,
      proofs: 0,
      audits: s.audits + 1,
      quarantined: true,
      log: "An audit re-ran a replay and got a different result. Quarantined until it proves itself again.",
    };
  const replaying = s.proofs >= need(s);
  const runs = s.runs + 1;
  if (replaying) {
    const audited = runs % 3 === 0;
    return {
      ...s,
      runs,
      audits: s.audits + (audited ? 1 : 0),
      log: audited ? "Audit: re-ran this replay in full. Identical." : "Replayed from a proven run.",
    };
  }
  const proofs = s.proofs + 1;
  return {
    ...s,
    runs,
    proofs,
    log: proofs >= need(s) ? "Proven. Identical results from identical inputs. Replays are on." : `Executed. Proof ${proofs} of ${need(s)}.`,
  };
}

// Prove, replay, get audited, hit a flaky run, recover.
const PLAY: Action["type"][] = ["run", "run", "run", "run", "run", "flake", "run", "run", "run", "run"];

export function TrustMeter() {
  const [ref, inView] = useInView<HTMLDivElement>(0.5);
  const reduceMotion = useReducedMotion();
  const [s, dispatch] = useReducer(reduce, start);
  const replaying = s.proofs >= need(s);

  useEffect(() => {
    if (!inView || reduceMotion) return;
    const ctl = new AbortController();
    (async () => {
      for (;;) {
        dispatch({ type: "reset" });
        await sleep(1200, ctl.signal);
        for (const a of PLAY) {
          dispatch({ type: a });
          await sleep(1500, ctl.signal);
        }
        await sleep(2500, ctl.signal);
      }
    })().catch(() => {});
    return () => ctl.abort();
  }, [inView, reduceMotion]);

  return (
    <div className="meter" ref={ref}>
      <div className="meter-top">
        <span className="mono">go test ./...</span>
        <span className={`state ${replaying ? "on" : ""}`}>
          {replaying ? "replaying" : s.quarantined ? "quarantined" : "observing"}
        </span>
      </div>
      <div className="bar" aria-hidden>
        <i style={{ width: `${Math.min(100, (s.proofs / need(s)) * 100)}%` }} />
      </div>
      <div className="meter-row mono">
        <span>{replaying ? `proven ${need(s)} of ${need(s)}` : `${s.proofs} of ${need(s)} proofs`}</span>
        <span>
          {s.audits} audit{s.audits === 1 ? "" : "s"}
        </span>
      </div>
      <p className="meter-log" aria-live="polite">
        {s.log}
      </p>
    </div>
  );
}

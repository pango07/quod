"use client";

import { useEffect, useState } from "react";
import { sleep, useInView, useReducedMotion } from "@/lib/use-in-view";

type Push = "first" | "next";

// A go job's steps, timed from a cli/cli run: first push executes, next push replays.
const STEPS: { name: string; first: number; next: number; tag?: string }[] = [
  { name: "Set up job", first: 3.1, next: 0.3 },
  { name: "Checkout", first: 2.4, next: 0.1, tag: "served from store" },
  { name: "Set up Go", first: 14.0, next: 0.2, tag: "replayed" },
  { name: "Download modules", first: 9.6, next: 0.2, tag: "replayed" },
  { name: "Build", first: 21.4, next: 0.3, tag: "replayed" },
  { name: "go test ./...", first: 17.0, next: 0.25, tag: "replayed" },
];

const fmt = (s: number) => (s < 1 ? s.toFixed(2) : s.toFixed(1)) + "s";

export function CiJob() {
  const [ref, inView] = useInView<HTMLDivElement>(0.4);
  const reduce = useReducedMotion();
  const [push, setPush] = useState<Push>("first");
  const [done, setDone] = useState(STEPS.length);
  const [pinned, setPinned] = useState(false);

  // While in view and not pinned by a click, loop: first push, then next push.
  useEffect(() => {
    if (!inView || pinned || reduce) return;
    const ctl = new AbortController();
    (async () => {
      for (;;) {
        for (const p of ["first", "next"] as Push[]) {
          setPush(p);
          for (let i = 0; i <= STEPS.length; i++) {
            setDone(i);
            await sleep(p === "first" ? 620 : 170, ctl.signal);
          }
          await sleep(p === "first" ? 1400 : 2800, ctl.signal);
        }
      }
    })().catch(() => {});
    return () => ctl.abort();
  }, [inView, pinned, reduce]);

  const choose = (p: Push) => {
    setPinned(true);
    setPush(p);
    setDone(STEPS.length);
  };

  const total = STEPS.slice(0, done).reduce((s, x) => s + (push === "first" ? x.first : x.next), 0);
  return (
    <div className="job" ref={ref}>
      <div className="job-head">
        <span className="job-title">
          test <span className="faint">/</span> go test ./...
        </span>
        <div className="seg" role="group" aria-label="Which push to show">
          {(["first", "next"] as Push[]).map((p) => (
            <button key={p} type="button" aria-pressed={push === p} onClick={() => choose(p)}>
              {p === "first" ? "First push" : "Next push"}
            </button>
          ))}
        </div>
      </div>
      <ol className="steps">
        {STEPS.map((s, i) => {
          const t = push === "first" ? s.first : s.next;
          const state = i < done ? (push === "next" && s.tag ? "rep" : "done") : i === done ? "run" : "";
          return (
            <li key={s.name}>
              <span className={`dot ${state}`} aria-hidden />
              <span>
                {s.name}
                {i < done && push === "next" && s.tag ? <span className="tag">{s.tag}</span> : null}
              </span>
              <span className="t">{i < done ? fmt(t) : ""}</span>
            </li>
          );
        })}
      </ol>
      <div className="job-foot">
        <span>
          total <b>{fmt(total)}</b>
        </span>
        <span>{push === "first" ? "every step runs and is recorded" : "unchanged inputs, proven results"}</span>
      </div>
    </div>
  );
}

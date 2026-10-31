"use client";

import { useEffect, useState } from "react";
import { sleep, useInView, useReducedMotion } from "@/lib/use-in-view";

const STEPS = [
  {
    title: "Observe",
    body: "Your repo is served through a filesystem that records every file a step opens, every directory it lists and every path it looks for and doesn't find. That record is the step's real input list. Nothing to declare.",
  },
  {
    title: "Prove",
    body: "The result is stored against those exact inputs. A step becomes replayable only after it reproduces the same result from the same inputs.",
  },
  {
    title: "Replay",
    body: "On the next push, if every recorded input is byte-identical, Quod returns the proven result in a fraction of a second. If anything changed, the step runs and the receipt names what moved.",
  },
];

/** The three stages, lit one after another while on screen. */
export function Flow() {
  const [ref, inView] = useInView<HTMLOListElement>(0.4);
  const reduce = useReducedMotion();
  const [on, setOn] = useState(-1);

  useEffect(() => {
    if (!inView || reduce) return;
    const ctl = new AbortController();
    (async () => {
      for (let i = 0; ; i = (i + 1) % STEPS.length) {
        setOn(i);
        await sleep(3200, ctl.signal);
      }
    })().catch(() => {});
    return () => ctl.abort();
  }, [inView, reduce]);

  return (
    <ol className="flow" ref={ref}>
      {STEPS.map((s, i) => (
        <li key={s.title} className={on === -1 || on === i ? "lit" : ""} onMouseEnter={() => setOn(i)}>
          <span className="n">{i + 1}</span>
          <h3>{s.title}</h3>
          <p>{s.body}</p>
        </li>
      ))}
    </ol>
  );
}

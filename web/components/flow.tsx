"use client";

import { useEffect, useState } from "react";
import { sleep, useInView, useReducedMotion } from "@/lib/use-in-view";

const STEPS = [
  {
    title: "Record",
    body: "Quod runs your tests and records the inputs used by each file, including shared configuration and loaded dependencies.",
  },
  {
    title: "Verify",
    body: "A fresh result needs confirmation before reuse. Quod compares test outcomes and checks the recorded inputs on later runs.",
  },
  {
    title: "Reuse",
    body: "Eligible files with unchanged inputs reuse their results. Changed and ineligible files run. Audits keep checking reused results.",
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

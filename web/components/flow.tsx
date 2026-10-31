"use client";

import { useEffect, useState } from "react";
import { sleep, useInView, useReducedMotion } from "@/lib/use-in-view";

const STEPS = [
  {
    title: "Watch",
    body: "Quod runs your CI step and notes every file it reads.",
  },
  {
    title: "Compare",
    body: "On your next push, it checks whether any of those files changed.",
  },
  {
    title: "Reuse",
    body: "Nothing changed? You get the last result in under a second. Something changed? The step runs as usual.",
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

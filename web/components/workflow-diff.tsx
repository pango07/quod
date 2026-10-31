"use client";

import { useInView } from "@/lib/use-in-view";

/** The one-line change, applied once it scrolls into view. */
export function WorkflowDiff() {
  const [ref, inView] = useInView<HTMLDivElement>(0.6);
  return (
    <div
      className={`diff ${inView ? "applied" : ""}`}
      ref={ref}
      role="img"
      aria-label="Workflow change: runs-on ubuntu-latest becomes runs-on self-hosted, quod"
    >
      <div className="ctx">jobs:</div>
      <div className="ctx">  test:</div>
      <div className="rm">-   runs-on: ubuntu-latest</div>
      <div className="ad">+   runs-on: [self-hosted, quod]</div>
      <div className="ctx">    steps:</div>
      <div className="ctx">      - uses: actions/checkout@v4</div>
      <div className="ctx">      - uses: actions/setup-go@v5</div>
      <div className="ctx">      - run: go test ./...</div>
    </div>
  );
}

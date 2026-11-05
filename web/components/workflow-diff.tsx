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
      aria-label="Add pango07/quod-action at v2 after setup-node. Keep ubuntu-latest, npm ci, and npm test."
    >
      <div className="ctx">jobs:</div>
      <div className="ctx">  test:</div>
      <div className="ctx">    runs-on: ubuntu-latest</div>
      <div className="ctx">    steps:</div>
      <div className="ctx">      - uses: actions/checkout@v7</div>
      <div className="ctx">      - uses: actions/setup-node@v7</div>
      <div className="ctx">        with:</div>
      <div className="ctx">          node-version: 24.18.0</div>
      <div className="ad">+     - uses: pango07/quod-action@v2</div>
      <div className="ctx">      - run: npm ci</div>
      <div className="ctx">      - run: npm test</div>
    </div>
  );
}

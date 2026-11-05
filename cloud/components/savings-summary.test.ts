import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import fs from "node:fs";
import path from "node:path";
import type { TeamTotals } from "@/lib/api/types";
import { SavingsSummary } from "./savings-summary";

const totals: TeamTotals = {range:"7d", savedMinutes:100, stepsReplayed:60, stepsTotal:100,
  costSavedUsd:0.8, usdPerMinute:0.008, wrongReplays:0, spotChecks:0, spotChecksIdentical:0};
const ci = {medianSeconds:20, measuredRuns:12, sample:false, runs:[]};
function preview(name:string, html:string) {
  const dir = process.env.QUOD_OVERVIEW_PREVIEW;
  if (!dir) return;
  fs.mkdirSync(dir, {recursive:true});
  const css = fs.readFileSync(path.join(process.cwd(), "app/globals.css"), "utf8").replace('@import "tailwindcss";', "");
  fs.writeFileSync(path.join(dir, name+".html"), `<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Quod overview preview</title><style>${css}:root{--sans:system-ui,sans-serif;--mono:monospace}.page{max-width:1100px;margin:60px auto;padding:24px}h1{margin-bottom:24px}</style><body><main class="page"><h1>Overview</h1><p class="muted" style="margin-bottom:24px">Preview · illustrative data · last 7 days</p>${html}<details class="card" style="padding:20px;margin-top:20px"><summary>Savings details</summary><p>Execution estimates and whole-job savings stay separate. Dollar estimates include runner billing rules.</p></details></main></body></html>`);
}
describe("overview savings evidence", () => {
  it("does not present legacy skipped-step estimates as net money or workflow time saved", () => {
    const html = renderToStaticMarkup(createElement(SavingsSummary, {totals, ci}));
    expect(html).toContain("Execution avoided");
    expect(html).toContain("before overhead");
    expect(html).toContain("20s");
    expect(html).toContain("Measuring");
    expect(html).not.toContain("$0.80");
    expect(html).not.toContain("minutes you did not wait for");
    preview("measuring", html);
  });
  it("shows zero dollars for known free runners and preserves the time saving", () => {
    const html = renderToStaticMarkup(createElement(SavingsSummary, {ci, totals:{...totals,
      jobSavings:{jobs:12,baseline:"observed",runnerSeconds:1173,baselineRunnerSeconds:1905,billedMinutes:25,baselineBilledMinutes:36,costUsd:0,baselineCostUsd:0}}}));
    expect(html).toContain("12m 12s");
    expect(html).toContain("38% less");
    expect(html).toContain("$0.00");
    expect(html).not.toContain("Measuring");
    preview("known-free-runners", html);
  });
  it("keeps additional time/cost visible instead of clamping it to zero", () => {
    const html = renderToStaticMarkup(createElement(SavingsSummary, {ci, totals:{...totals,
      jobSavings:{jobs:1,baseline:"observed",runnerSeconds:70,baselineRunnerSeconds:50,billedMinutes:2,baselineBilledMinutes:1,costUsd:0.02,baselineCostUsd:0.01}}}));
    expect(html).toContain("Extra runner time");
    expect(html).toContain("40% more");
    expect(html).toContain("Est. extra runner cost");
  });
});

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
const valuationRate = {usdPerMinute:0.016, source:"team" as const};
function preview(name:string, html:string) {
  const dir = process.env.QUOD_OVERVIEW_PREVIEW;
  if (!dir) return;
  fs.mkdirSync(dir, {recursive:true});
  const css = fs.readFileSync(path.join(process.cwd(), "app/globals.css"), "utf8").replace('@import "tailwindcss";', "");
  fs.writeFileSync(path.join(dir, name+".html"), `<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Quod overview preview</title><style>${css}:root{--sans:system-ui,sans-serif;--mono:monospace}.page{max-width:1100px;margin:60px auto;padding:24px}h1{margin-bottom:24px}</style><body><main class="page"><h1>Overview</h1><p class="muted" style="margin-bottom:24px">Preview · illustrative data · last 7 days</p>${html}<details class="card" style="padding:20px;margin-top:20px"><summary>Savings details</summary><p>Execution estimates and whole-job savings stay separate. Modeled time value uses configured rates; compute charges stay separate.</p></details></main></body></html>`);
}
describe("overview savings evidence", () => {
  it("uses the team's test history when step telemetry is absent", () => {
    const html = renderToStaticMarkup(createElement(SavingsSummary, {totals:{...totals, savedMinutes:0}, ci, valuationRate,
      execution:{minutes:2, note:"Estimated skipped work, before overhead · 2/3 test runs"}}));
    expect(html).toContain("Execution avoided");
    expect(html).toContain("2m");
    expect(html).toContain("2/3 test runs");
    expect(html).toContain("$0.0320");
    expect(html).toContain("Est. execution value avoided");
  });
  it("values legacy skipped work at the configured rate while retaining its before-overhead basis", () => {
    const html = renderToStaticMarkup(createElement(SavingsSummary, {totals, ci, valuationRate}));
    expect(html).toContain("Execution avoided");
    expect(html).toContain("before overhead");
    expect(html).toContain("20s");
    expect(html).toContain("$1.60");
    expect(html).toContain("$0.016 per minute");
    expect(html).toContain("Est. execution value avoided");
    expect(html).not.toContain("$0.80");
    expect(html).not.toContain("minutes you did not wait for");
    preview("measuring", html);
  });
  it("values time on known-free runners and keeps their zero compute cost separate", () => {
    const html = renderToStaticMarkup(createElement(SavingsSummary, {ci, valuationRate, totals:{...totals,
      jobSavings:{jobs:12,baseline:"observed",runnerSeconds:1173,baselineRunnerSeconds:1905,billedMinutes:25,baselineBilledMinutes:36,costUsd:0,baselineCostUsd:0}}}));
    expect(html).toContain("12m 12s");
    expect(html).toContain("38% less");
    expect(html).toContain("$0.00");
    expect(html).toContain("$0.1952");
    expect(html).toContain("Modeled runner value saved");
    expect(html).toContain("Reported compute cost");
    expect(html).not.toContain("Measuring");
    preview("known-free-runners", html);
  });
  it("keeps additional time/cost visible instead of clamping it to zero", () => {
    const html = renderToStaticMarkup(createElement(SavingsSummary, {ci, valuationRate, totals:{...totals,
      jobSavings:{jobs:1,baseline:"observed",runnerSeconds:70,baselineRunnerSeconds:50,billedMinutes:2,baselineBilledMinutes:1,costUsd:0.02,baselineCostUsd:0.01}}}));
    expect(html).toContain("Extra runner time");
    expect(html).toContain("40% more");
    expect(html).toContain("Est. extra runner cost");
    expect(html).toContain("Modeled extra runner value");
    expect(html).toContain("$0.0053");
  });

  it("revalues the same observed difference at the current rate without using provider-rounded minutes", () => {
    const data = { ...totals, jobSavings:{jobs:1,baseline:"observed" as const,runnerSeconds:175,baselineRunnerSeconds:100,billedMinutes:3,baselineBilledMinutes:2,costUsd:0,baselineCostUsd:0} };
    const render = (rate:typeof valuationRate) => renderToStaticMarkup(createElement(SavingsSummary, {ci, totals:data, valuationRate:rate}));
    expect(render({...valuationRate, usdPerMinute:0.006})).toContain("$0.0075");
    expect(render(valuationRate)).toContain("$0.0200");
    expect(render(valuationRate)).toContain("1m 15s");
  });

  it("keeps missing observations and unavailable rates unknown instead of fabricating zero", () => {
    const unavailable = renderToStaticMarkup(createElement(SavingsSummary, {totals, ci}));
    expect(unavailable).toContain("Measuring");
    expect(unavailable).toContain("Waiting for your runner price");
    const partial = renderToStaticMarkup(createElement(SavingsSummary, {totals, ci, valuationRate, execution:{minutes:null, note:"No duration reported"}}));
    expect(partial).toContain("Measuring");
    expect(partial).not.toContain("$1.60");
    const noRuns = renderToStaticMarkup(createElement(SavingsSummary, {totals, ci, valuationRate, hasQuodData:false}));
    expect(noRuns).toContain("Measuring");
    expect(noRuns).not.toContain("$1.60");
  });
});

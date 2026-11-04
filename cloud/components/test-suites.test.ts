import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { sampleTestRuns, sampleTestStats, sampleTestSuites, summaryOnly } from "@/lib/api/fixture-tests";
import { formatAgo } from "@/lib/format";
import { trustModel } from "@/lib/test-suites";
import { TrustCompact, TrustStart } from "./test-suites";

// The compact Trust page, rendered from the sample data (as on first load: everything collapsed).

const NOW = Date.parse("2026-11-02T12:00:00Z");
const ASOF = new Date(NOW).toISOString();
const text = (html: string) => html.replace(/<[^>]+>/g, " ").replace(/\s+/g, " ");

const model = (suites = sampleTestSuites(NOW)) =>
  trustModel({ team: "acme", suites, runs: sampleTestRuns(NOW).map(summaryOnly), stats: sampleTestStats(NOW), now: NOW, ago: (iso) => formatAgo(iso, ASOF) });

describe("Trust, compact", () => {
  const html = renderToStaticMarkup(createElement(TrustCompact, { model: model(), asOf: ASOF }));
  const t = text(html);

  it("a summary strip: wrong results, time saved, % skipped, one state bar for every file", () => {
    expect(t).toContain("Wrong results 0");
    expect(t).toMatch(/\d+ spot-checks passed/);
    expect(t).toContain("Time saved, 7 days");
    expect(t).toContain("of test files skipped, last 20 checks");
    expect(html).toMatch(/title="[0-9,]+ of [0-9,]+ files across 20 checks"/);
    expect(t).toContain("296 test files");
    expect(t).toContain("10 learning");
    expect(t).toContain("265 skipping");
    expect(html.match(/class="statebar "/g)).toHaveLength(1);
  });

  it("needs a look lists only paused and always-runs files, paused first, one line each", () => {
    const look = html.slice(html.indexOf('id="look-h"'), html.indexOf('class="card repo-suites"'));
    const files = [...look.matchAll(/class="file">([^<]+)</g)].map((m) => m[1]);
    expect(files).toEqual(["src/billing/discounts.test.ts", "src/api/routes/login.test.ts", "src/db/migrations.test.ts", "src/components/date-picker.test.tsx"]);
    expect(look).toContain("Latest run");
    expect(look).not.toContain("retry.test.ts");
  });

  it("suites grouped under their repository, one collapsed row each: package, mini bar, counts; no file lists until opened", () => {
    const heads = [...html.matchAll(/class="repo-suites-name">([^<]+)</g)].map((m) => m[1]);
    expect(heads).toEqual(["web", "quod"]);
    const web = text(html.slice(html.indexOf('class="repo-suites-name">web<'), html.indexOf('class="repo-suites-name">quod<')));
    expect(web).toContain("quodci/web");
    expect(web).toContain("packages/backend");
    expect(t).toContain("repo root");
    expect(t).toContain("184 files · 170 skipping · 3 need a look");
    expect(t).toContain("7 files · 7 skipping");
    expect(html.match(/aria-expanded="false"/g)).toHaveLength(4);
    expect(html).not.toContain("module-01.test.ts");
    expect(t).toContain("No recording yet");
  });

  it("filter pills with counts", () => {
    expect(t).toContain("All 296");
    expect(t).toContain("Needs a look 4");
    expect(t).toContain("Skipping 265");
    expect(t).toContain("Ready 17");
    expect(t).toContain("Learning 10");
  });

  it("never says default branch", () => {
    expect(html).not.toMatch(/default branch/i);
  });
});

describe("Trust, nothing needs a look", () => {
  it("the section is replaced by one line", () => {
    const calm = sampleTestSuites(NOW).map((s) => ({ ...s, files: s.files.filter((f) => f.state !== "paused" && f.state !== "always") }));
    const t = text(renderToStaticMarkup(createElement(TrustCompact, { model: model(calm), asOf: ASOF })));
    expect(t).toContain("Nothing needs a look.");
    expect(t).not.toContain("Needs a look ·");
  });
});

describe("Trust, empty", () => {
  it("says how to start in plain words and names the action as code; no other setup steps", () => {
    const html = renderToStaticMarkup(createElement(TrustStart));
    expect(html).toContain("<code");
    expect(html).toContain("pango07/quod-action</code>");
    expect(text(html)).toContain("add pango07/quod-action to your CI");
    expect(html).not.toMatch(/<table|Step/);
  });
});

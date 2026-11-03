import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { sampleTestSuites } from "@/lib/api/fixture-tests";
import { TestSuitesList, TrustStart } from "./test-suites";

// The Trust page's test files and its empty state, rendered.

const NOW = Date.parse("2026-11-02T12:00:00Z");
const ASOF = new Date(NOW).toISOString();
const text = (html: string) => html.replace(/<[^>]+>/g, " ").replace(/\s+/g, " ");

describe("Trust, test-first", () => {
  const html = renderToStaticMarkup(createElement(TestSuitesList, { team: "acme", suites: sampleTestSuites(NOW), asOf: ASOF }));
  const t = text(html);

  it("one card per suite, headed repo · package, with totals and the recording it reads", () => {
    expect(t).toContain("web · packages/backend");
    expect(t).toContain("web · packages/web");
    expect(t).toContain("184 test files: 170 skipping, 10 ready, 1 learning, 2 always run, 1 paused.");
    expect(t).toContain("Reads the recording of main at 1d7e5b2, 2h ago.");
  });

  it("files that need a look are listed; skipping and ready fold under a count", () => {
    expect(t).toContain("Paused: a spot-check differed");
    expect(t).toContain("Always runs: it failed last time");
    expect(t).toContain("Always runs: recording changed its behaviour");
    expect(t).toContain("Learning");
    expect(t).toContain("Show 180 more: 170 skipping, 10 ready");
    expect(t).toContain("Show 95 more: 88 skipping, 7 ready");
    // Folded: no skipping row is rendered until opened.
    expect(t).not.toMatch(/Skipping: skipped \d+ time/);
  });

  it("links each suite to its repository's test runs", () => {
    expect(html).toContain('href="/acme/web/tests"');
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

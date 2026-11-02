import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import type { GitHubLink } from "@/lib/api/types";

// The Connect page must only offer what works today (review of the "quodci/setup@v1" copy).
vi.mock("@/lib/actions", () => ({ saveConnectChoice: vi.fn() }));
const { ConnectGitHubStep } = await import("./connect-github");
const { ConnectFlow, EARLY_ACCESS_URL, SETUP_ACTION_TODAY } = await import("./connect-flow");

const step = (link: GitHubLink | undefined, admin: boolean) => renderToStaticMarkup(createElement(ConnectGitHubStep, { team: "acme", link, admin }));

describe("Connect, step 1: GitHub", () => {
  it("linked: says Connected with the account and repo count, and links to Settings, GitHub", () => {
    const html = step({ state: "linked", repos: 2, accounts: ["acme"] }, true);
    expect(html).toContain("Connected");
    expect(html).toContain("acme · 2 repositories");
    expect(html).toContain('href="/acme/settings#github-h"');
    expect(html).toContain("Manage");
    expect(html).not.toContain("/github/install");
  });

  it("unlinked, admin: a Connect GitHub button to /github/install (a GET form, never prefetched)", () => {
    const html = step({ state: "unlinked", canInstall: true }, true);
    expect(html).toMatch(/<form action="\/github\/install" method="get">/);
    expect(html).toContain("Connect GitHub");
    expect(html).not.toContain("Ask an admin");
  });

  it("unlinked, member: asks an admin, and offers no install", () => {
    const html = step({ state: "unlinked", canInstall: true }, false);
    expect(html).toContain("Ask an admin");
    expect(html).not.toContain("/github/install");
  });

  it("says so when the App or the database is not set up, and on the sample team", () => {
    expect(step({ state: "unlinked", canInstall: false }, true)).toContain("isn&#x27;t set up on this deploy");
    expect(step({ state: "not-configured" }, true)).toContain("needs a database");
    expect(step(undefined, true)).toContain("sample team");
  });
});

describe("Connect, the Quod setup", () => {
  const setup = {
    setupAction: "quodci/setup@v1",
    runnerLabels: ["self-hosted", "quod"],
    installCommand: "curl -fsSL https://quodci.com/install.sh | sh",
    selfHostedExample: "https://quod.internal.example",
    firstRepo: null,
    saved: null,
  };
  const render = (store: "cloud" | "self", host: "github" | "own") =>
    renderToStaticMarkup(createElement(ConnectFlow, { team: "acme", setup, initial: { store, host }, canSave: true }));

  describe("GitHub-hosted + Quod Cloud: available", () => {
    const html = render("cloud", "github");

    it("is marked available, not coming soon", () => {
      expect(html).toContain("Available: GitHub-hosted + Quod Cloud");
      expect(html).not.toContain("Coming soon: early access");
      expect(html).not.toContain("doesn&#x27;t work yet");
    });

    it("shows the working workflow change, copyable: id-token, the setup action that exists, quod run", () => {
      expect(html).toMatch(/<pre class="wf" aria-label="Workflow change"/);
      expect(html).toContain("id-token: write");
      expect(html).toContain(SETUP_ACTION_TODAY);
      expect(html).toContain("quod run -- npm test");
      expect(html).not.toContain("quodci/setup@v1");
      expect(html).not.toContain("(preview)");
    });
  });

  for (const [store, host] of [["self", "github"], ["cloud", "own"], ["self", "own"]] as const) {
    describe(`${store} + ${host}: coming soon`, () => {
      const html = render(store, host);

      it("is marked coming soon, says it doesn't work yet, and links to early access", () => {
        expect(html).toContain("Coming soon: early access");
        expect(html).toContain("doesn&#x27;t work yet");
        expect(EARLY_ACCESS_URL).toBe("https://www.quodci.com/#join");
        expect(html).toContain(`href="${EARLY_ACCESS_URL}"`);
      });

      it("keeps the workflow diff as a non-copyable preview, and 'Push, and watch it learn' inside the group", () => {
        expect(html).toMatch(/<pre class="wf no-copy" aria-label="Workflow change \(preview; not released yet\)"/);
        const group = html.slice(html.indexOf('class="soon"'));
        expect(group).toContain("Push, and watch it learn");
        expect(html).not.toContain("Done, back to the dashboard");
      });
    });
  }
});

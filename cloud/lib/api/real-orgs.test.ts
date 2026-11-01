import { describe, expect, it } from "vitest";
import { githubRepoSlugs } from "@/lib/api/composite";
import { whileAllowed } from "@/lib/live/guard";
import { isTeamSlug } from "@/lib/params";

describe("team slugs", () => {
  it("are never a file name, so /favicon.ico and friends are not teams", () => {
    for (const ok of ["quodci", "quodci-2", "acme", "a"]) expect(isTeamSlug(ok), ok).toBe(true);
    for (const bad of ["favicon.ico", "apple-touch-icon.png", "icon.svg", "Quodci", "-x", "", "a".repeat(64)]) expect(isTeamSlug(bad), bad).toBe(false);
  });
});

describe("GitHub repo slugs", () => {
  it("use the bare name, or owner-name when two owners share it; keyed by id, never by bare name", () => {
    expect(
      githubRepoSlugs([
        { id: 1, fullName: "acme/api" },
        { id: 2, fullName: "other/API" },
        { id: 3, fullName: "acme/web" },
        { id: 3, fullName: "acme/web" },
        { id: 4, fullName: "acme/settings" }, // a team page owns /settings
      ]),
    ).toEqual([
      { id: 1, fullName: "acme/api", slug: "acme-api" },
      { id: 2, fullName: "other/API", slug: "other-API" },
      { id: 3, fullName: "acme/web", slug: "web" },
    ]);
  });
});

describe("whileAllowed", () => {
  async function* ticks(signal: AbortSignal) {
    for (let i = 0; !signal.aborted; i++) {
      yield i;
      await new Promise((r) => setTimeout(r, 10));
    }
  }

  it("ends the stream once the check says no", async () => {
    let allowed = true;
    const got: number[] = [];
    setTimeout(() => (allowed = false), 60);
    for await (const i of whileAllowed(ticks, { signal: new AbortController().signal, everyMs: 20, check: async () => allowed })) got.push(i);
    expect(got.length).toBeGreaterThan(2);
  });

  it("ends the stream when the check fails", async () => {
    const got: number[] = [];
    for await (const i of whileAllowed(ticks, { signal: new AbortController().signal, everyMs: 20, check: () => Promise.reject(new Error("down")) })) got.push(i);
    expect(got.length).toBeGreaterThan(0);
  });

  it("keeps going while allowed, until the caller aborts", async () => {
    const ac = new AbortController();
    const got: number[] = [];
    setTimeout(() => ac.abort(), 100);
    for await (const i of whileAllowed(ticks, { signal: ac.signal, everyMs: 20, check: async () => true })) got.push(i);
    expect(got.length).toBeGreaterThan(4);
  });
});

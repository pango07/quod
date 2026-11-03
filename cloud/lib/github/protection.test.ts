import { afterAll, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { migratedDb, truncateAll } from "@/lib/db/testing";
import type { Db } from "@/lib/db/types";
import { protectionLabel } from "@/components/section";
import { isPlanLimit, PLAN_REASON, PROTECTION_TIMEOUT_MS, readDefaultBranchProtection, type ProtectionRead } from "./api";
import { delivery, SECRET } from "./fixtures";
import { claimForRefresh, markProtectionStale, PENDING_S, protectionState, readCached, recordRead, refreshProtection } from "./protection";
import { linkInstallation } from "./store";
import type { TeamId } from "./team";
import { handleWebhook } from "./webhook";

let db: Db;
const TEAM = "org_acme" as TeamId;
const OWNER = { id: 500, login: "acme", type: "Organization" };
const APP = { id: 7, name: "app", full_name: "acme/app", private: true, owner: OWNER };
const WEB = { id: 8, name: "web", full_name: "acme/web", private: true, owner: OWNER };
const NO_RATE = { remaining: 4000, resetAt: null, retryAfterMs: null };
const ok = (classic: boolean | null, rulesets: boolean | null): ProtectionRead => ({ kind: "ok", defaultBranch: "main", classic, rulesets, reason: null, rate: NO_RATE });

beforeAll(async () => {
  db = await migratedDb();
});
afterAll(async () => db.close());
beforeEach(async () => {
  await truncateAll(db);
  await db.tx((q) => linkInstallation(q, { installation: { id: 77, account: OWNER }, teamId: TEAM, userId: "user_1", githubUserId: 1, repos: [APP, WEB] }));
});

describe("default-branch protection", () => {
  it("protected when classic protection or a ruleset applies; unprotected only when both were read and neither does", () => {
    expect(protectionState({ classic: true, rulesets: null })).toBe("protected");
    expect(protectionState({ classic: null, rulesets: true })).toBe("protected");
    expect(protectionState({ classic: false, rulesets: false })).toBe("unprotected");
    expect(protectionState({ classic: null, rulesets: false })).toBe("unknown");
    expect(protectionState({ classic: false, rulesets: null })).toBe("unknown");
  });

  it("reads GitHub: the repo's default branch, its classic protection, the rules that apply to it", async () => {
    const calls: string[] = [];
    const gh = (answers: Record<string, [number, unknown]>) =>
      (async (url: string) => {
        const path = new URL(url).pathname + new URL(url).search;
        calls.push(path);
        const [status, body] = answers[path] ?? [500, {}];
        return new Response(JSON.stringify(body), { status });
      }) as unknown as typeof fetch;
    const unprotected = await readDefaultBranchProtection("t", "acme/app", gh({
      "/repos/acme/app": [200, { default_branch: "trunk" }],
      "/repos/acme/app/branches/trunk/protection": [404, { message: "Branch not protected" }],
      "/repos/acme/app/rules/branches/trunk?per_page=1": [200, []],
    }));
    expect(unprotected).toMatchObject({ kind: "ok", defaultBranch: "trunk", classic: false, rulesets: false });
    expect(calls).toContain("/repos/acme/app/branches/trunk/protection");

    const noPermission = await readDefaultBranchProtection("t", "acme/app", gh({
      "/repos/acme/app": [200, { default_branch: "main" }],
      "/repos/acme/app/branches/main/protection": [403, { message: "Resource not accessible by integration" }],
      "/repos/acme/app/rules/branches/main?per_page=1": [200, []],
    }));
    expect(noPermission).toMatchObject({ kind: "ok", classic: null, rulesets: false, reason: "no-permission" });

    const ruleset = await readDefaultBranchProtection("t", "acme/app", gh({
      "/repos/acme/app": [200, { default_branch: "main" }],
      "/repos/acme/app/branches/main/protection": [404, { message: "Not Found" }],
      "/repos/acme/app/rules/branches/main?per_page=1": [200, [{ type: "pull_request", ruleset_id: 1 }]],
    }));
    // Rulesets are read first; one that applies is enough, classic protection is not read.
    expect(ruleset).toMatchObject({ kind: "ok", classic: null, rulesets: true, reason: null });
    expect(calls.filter((c) => c === "/repos/acme/app/branches/main/protection")).toHaveLength(1);
    expect(await readDefaultBranchProtection("t", "acme/gone", gh({ "/repos/acme/gone": [404, {}] }))).toMatchObject({ kind: "gone" });
  });

  it("a private repo on GitHub's free plan (403 'Upgrade to GitHub Pro'): can't check on this plan", async () => {
    const seen: string[] = [];
    const f = (async (url: string) => {
      const path = new URL(url).pathname + new URL(url).search;
      seen.push(path);
      if (path === "/repos/acme/app") return new Response(JSON.stringify({ default_branch: "main" }), { status: 200 });
      if (path.startsWith("/repos/acme/app/rules/")) return new Response("[]", { status: 200 });
      return new Response(JSON.stringify({ message: "Upgrade to GitHub Pro or make this repository public to enable this feature." }), { status: 403 });
    }) as unknown as typeof fetch;
    const read = await readDefaultBranchProtection("t", "acme/app", f);
    expect(read).toMatchObject({ kind: "ok", classic: null, rulesets: false, reason: PLAN_REASON });
    expect(seen.indexOf("/repos/acme/app/rules/branches/main?per_page=1")).toBeLessThan(seen.indexOf("/repos/acme/app/branches/main/protection"));
    expect(protectionState(read as Extract<ProtectionRead, { kind: "ok" }>)).toBe("unknown");
    expect(isPlanLimit("Resource not accessible by integration")).toBe(false);

    // Cached and shown as such.
    await claimForRefresh(db, TEAM, [7]);
    await recordRead(db, 7, read);
    const p = (await readCached(db, TEAM, [7])).byRepo!.get(7)!;
    expect(p).toMatchObject({ state: "unknown", detail: "plan" });
    expect(protectionLabel(p)).toBe("Protection: can't check on this plan");
  });

  it("gives up after the timeout: an error, never a read that hangs", async () => {
    const hang = ((_: string, init?: RequestInit) =>
      new Promise<Response>((_, reject) => init?.signal?.addEventListener("abort", () => reject(init.signal!.reason)))) as unknown as typeof fetch;
    const deaf = (() => new Promise<Response>(() => undefined)) as unknown as typeof fetch;
    expect(PROTECTION_TIMEOUT_MS).toBe(5_000);
    expect(await readDefaultBranchProtection("t", "acme/app", hang, 20)).toMatchObject({ kind: "error", status: 0, reason: "timeout" });
    // Even a fetch that ignores the signal.
    expect(await readDefaultBranchProtection("t", "acme/app", deaf, 20)).toMatchObject({ kind: "error", reason: "timeout" });
    const boom = (async () => {
      throw new TypeError("fetch failed");
    }) as unknown as typeof fetch;
    expect(await readDefaultBranchProtection("t", "acme/app", boom)).toMatchObject({ kind: "error", status: 0 });
  });

  it("no spinner forever: 'checking' only while a first read is under way, then unknown", async () => {
    const now = new Date("2026-11-02T12:00:00Z");
    // No row yet: a read starts with this view.
    expect(protectionLabel((await readCached(db, TEAM, [7], now)).byRepo!.get(7)!)).toBe("checking protection");
    await claimForRefresh(db, TEAM, [7], { now });
    expect((await readCached(db, TEAM, [7], new Date(now.getTime() + 3_000))).byRepo!.get(7)!.pending).toBe(true);
    // The claim is older than PENDING_S and nothing was recorded (the function died): unknown.
    const later = (await readCached(db, TEAM, [7], new Date(now.getTime() + (PENDING_S + 1) * 1000))).byRepo!.get(7)!;
    expect(later.pending).toBeUndefined();
    expect(protectionLabel(later)).toBe("Protection: unknown");
    // A timed-out read is recorded: unknown at once, and read again on the next view.
    await recordRead(db, 7, { kind: "error", status: 0, reason: "timeout", rate: NO_RATE }, now);
    const after = (await readCached(db, TEAM, [7], now)).byRepo!.get(7)!;
    expect(after.checkedAt).not.toBeNull();
    expect(protectionLabel(after)).toBe("Protection: unknown");
    expect((await readCached(db, TEAM, [7], now)).due).toContain(7);
    // A badge without a cache row (the composite's default) is unknown too.
    expect(protectionLabel({ state: "unknown", branch: null, checkedAt: null })).toBe("Protection: unknown");
  });

  it("caches per repo, refreshes what is missing, stale or 6 h old, and only for the team's repos", async () => {
    let cached = await readCached(db, TEAM, [7, 8, 999]);
    expect(cached.due.sort()).toEqual([7, 8]); // 999 is not the team's
    expect(cached.byRepo?.get(7)).toEqual({ state: "unknown", branch: null, checkedAt: null, pending: true });

    const reads: string[] = [];
    const r = await refreshProtection(db, TEAM, [7, 8], { read: async (_i, name) => (reads.push(name), name === "acme/app" ? ok(false, false) : ok(true, false)) });
    expect(r).toEqual({ read: 2, limited: false });
    expect(reads.sort()).toEqual(["acme/app", "acme/web"]);
    cached = await readCached(db, TEAM, [7, 8]);
    expect(cached.due).toEqual([]);
    expect(cached.byRepo?.get(7)).toMatchObject({ state: "unprotected", branch: "main" });
    expect(cached.byRepo?.get(8)).toMatchObject({ state: "protected" });

    // Fresh: nothing is read again.
    expect(await refreshProtection(db, TEAM, [7, 8], { read: async () => ok(true, true) })).toEqual({ read: 0, limited: false });
    // Six hours later it is due again.
    const later = new Date(Date.now() + 6 * 3600_000 + 60_000);
    expect((await readCached(db, TEAM, [7, 8], later)).due.sort()).toEqual([7, 8]);
  });

  it("two instances never read the same repo at once", async () => {
    const now = new Date();
    const [a, b] = await Promise.all([claimForRefresh(db, TEAM, [7, 8], { now }), claimForRefresh(db, TEAM, [7, 8], { now })]);
    expect([...a, ...b].map((c) => c.id).sort()).toEqual([7, 8]);
    // An abandoned claim is taken over after 2 minutes.
    expect(await claimForRefresh(db, TEAM, [7, 8], { now })).toEqual([]);
    expect((await claimForRefresh(db, TEAM, [7, 8], { now: new Date(now.getTime() + 3 * 60_000) })).length).toBe(2);
  });

  it("a rate limit stops the pass and keeps what was known; a gone repo becomes unknown", async () => {
    const r = await refreshProtection(db, TEAM, [7, 8], { read: async () => ({ kind: "limited", rate: NO_RATE }) });
    expect(r.limited).toBe(true);
    const [claimed] = await claimForRefresh(db, TEAM, [7], { now: new Date(Date.now() + 3 * 60_000) });
    await recordRead(db, claimed.id, { kind: "gone", rate: NO_RATE });
    expect((await readCached(db, TEAM, [7])).byRepo?.get(7)).toMatchObject({ state: "unknown" });
  });

  it("webhooks mark it stale: the repo's, or every repo of the installation for an organization ruleset", async () => {
    await refreshProtection(db, TEAM, [7, 8], { read: async () => ok(true, false) });
    expect((await readCached(db, TEAM, [7, 8])).due).toEqual([]);

    const send = (event: string, payload: unknown) => handleWebhook({ db, secret: SECRET, ...delivery(event, payload) });
    expect((await send("branch_protection_rule", { action: "deleted", installation: { id: 77 }, repository: { id: 7 } })).body).toEqual({ ok: true });
    expect((await readCached(db, TEAM, [7, 8])).due).toEqual([7]);

    await refreshProtection(db, TEAM, [7], { read: async () => ok(false, false) });
    expect((await send("repository_ruleset", { action: "created", installation: { id: 77 } })).body).toEqual({ ok: true });
    expect((await readCached(db, TEAM, [7, 8])).due.sort()).toEqual([7, 8]);

    // A repository edit that is not about the default branch changes nothing.
    await refreshProtection(db, TEAM, [7, 8], { read: async () => ok(true, false) });
    expect((await send("repository", { action: "edited", installation: { id: 77 }, repository: { id: 7 }, changes: { description: {} } })).body).toEqual({ ok: true, ignored: true });
    expect((await send("repository", { action: "edited", installation: { id: 77 }, repository: { id: 7 }, changes: { default_branch: { from: "master" } } })).body).toEqual({ ok: true });
    expect((await readCached(db, TEAM, [7, 8])).due).toEqual([7]);
  });

  it("before the migration is applied: unknown, nothing written, webhooks still succeed", async () => {
    await db.exec("ALTER TABLE repo_protection RENAME TO repo_protection_later");
    try {
      expect(await readCached(db, TEAM, [7])).toEqual({ byRepo: null, due: [] });
      expect(await refreshProtection(db, TEAM, [7], { read: async () => ok(true, true) })).toEqual({ read: 0, limited: false });
      expect(await markProtectionStale(db, { repoId: 7, installationId: null })).toBe(0);
    } finally {
      await db.exec("ALTER TABLE repo_protection_later RENAME TO repo_protection");
    }
  });
});

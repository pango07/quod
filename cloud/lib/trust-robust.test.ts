import { describe, expect, it } from "vitest";
import { sampleTestRuns, sampleTestSuites, summaryOnly } from "@/lib/api/fixture-tests";
import { credentialsFor } from "@/lib/api/credentials";
import { HttpCloudApi, type ServerCredentials } from "@/lib/api/http";
import { type Section, section, sectionOrUnavailable } from "@/lib/api/section";
import { ApiUnavailableError, type TestRunSummary, type TestSuite } from "@/lib/api/types";
import { testsView } from "./test-suites";

// Trust must never say "Nothing here yet" because a read failed: the empty
// state needs both /test-suites and /test-runs to have answered 200 with
// nothing. A failure is "Couldn't load test files" with Retry.

const NOW = Date.parse("2026-11-02T12:00:00Z");
const CREDS: ServerCredentials = { team: "acme", token: "acme-viewer-token", kind: "static" };
const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });

function server(answer: (path: string) => Response | Promise<Response>, timeoutMs?: number) {
  const f = (async (input: URL | RequestInfo, init?: RequestInit) => {
    const url = new URL(String(input));
    const r = answer(url.pathname + url.search);
    // Honour the timeout's abort, as fetch does.
    return init?.signal ? Promise.race([r, new Promise<never>((_, reject) => init.signal!.addEventListener("abort", () => reject(init.signal!.reason)))]) : r;
  }) as typeof fetch;
  return new HttpCloudApi({ baseUrl: "https://quod.example.com", credentials: async () => CREDS, fetch: f, sleep: async () => undefined, ...(timeoutMs ? { timeoutMs } : {}) });
}

/** The Trust page's test view for a server that answers like `answer`. */
async function view(answer: (path: string) => Response | Promise<Response>, timeoutMs?: number) {
  const api = server(answer, timeoutMs);
  const [s, r] = await Promise.all([sectionOrUnavailable(api.listTestSuites("acme")), sectionOrUnavailable(api.listTestRuns("acme", { limit: 200 }))]);
  return testsView(s, r);
}

const ok = <T>(value: T): Section<T> => ({ ok: true, value });
const down: Section<never> = { ok: false, kind: "unavailable", message: "quod-server is unreachable" };
const old: Section<never> = { ok: false, kind: "not-available", message: "" };
const suites = sampleTestSuites(NOW);
const runs: TestRunSummary[] = sampleTestRuns(NOW).map(summaryOnly);
const none: TestSuite[] = [];

describe("Trust: fetch failed is an error, never the empty state", () => {
  it("a fetch that fails (network) → error", async () => {
    expect(await view(() => Promise.reject(new TypeError("fetch failed")))).toMatchObject({ kind: "error" });
  });

  it("a timeout → error", async () => {
    expect(await view(() => new Promise<Response>(() => undefined), 20)).toMatchObject({ kind: "error" });
  });

  it("5xx → error", async () => {
    expect(await view(() => json({ error: "internal", message: "boom" }, 500))).toMatchObject({ kind: "error" });
    expect(await view(() => new Response("Bad gateway", { status: 502 }))).toMatchObject({ kind: "error" });
  });

  it("a bare 404 (a proxy in front of a server that is down or redeploying) → error, not 'no data'", async () => {
    expect(await view(() => new Response("Application not found", { status: 404 }))).toMatchObject({ kind: "error" });
    expect(await view(() => json({ status: "error", code: 404, message: "Application not found" }, 404))).toMatchObject({ kind: "error" });
  });

  it("one of the two failing is enough: no empty state from half an answer", async () => {
    const half = await view((p) => (p.includes("test-suites") ? json({ suites: [] }) : json({ error: "internal", message: "x" }, 500)));
    expect(half).toMatchObject({ kind: "error" });
  });

  it("an older server (quod.v1 404, or 501) stays hidden, as before: not the empty state, not an error", async () => {
    expect(await view(() => json({ error: "not_found", message: "no such route" }, 404))).toEqual({ kind: "not-available" });
    expect(await view(() => json({ error: "not_implemented", message: "later" }, 501))).toEqual({ kind: "not-available" });
  });

  it("the empty state only when both answered 200 with nothing", async () => {
    expect(await view((p) => (p.includes("test-suites") ? json({ suites: [] }) : json({ runs: [] })))).toEqual({ kind: "empty" });
    expect(testsView(ok(none), ok([]))).toEqual({ kind: "empty" });
    expect(testsView(ok(none), ok(null))).toEqual({ kind: "not-available" });
    expect(testsView(ok(null), ok([]))).toEqual({ kind: "not-available" });
    expect(testsView(old, ok([]))).toEqual({ kind: "not-available" });
  });

  it("data wins when it is there; a failure on either still says so", () => {
    expect(testsView(ok(suites), ok(runs))).toEqual({ kind: "suites" });
    expect(testsView(ok(none), ok(runs))).toEqual({ kind: "runs" });
    expect(testsView(ok(suites), down)).toMatchObject({ kind: "error" });
    expect(testsView(down, ok(runs))).toMatchObject({ kind: "error" });
  });

  it("any other failure (a bug, a dropped stream) is unavailable too, never thrown into an empty page", async () => {
    expect(await sectionOrUnavailable(Promise.reject(new TypeError("terminated")))).toMatchObject({ ok: false, kind: "unavailable" });
    // `section` keeps rethrowing what it does not know (other pages), as before.
    await expect(section(Promise.reject(new TypeError("terminated")))).rejects.toThrow("terminated");
  });
});

describe("viewer sessions (mobile Safari resuming a tab)", () => {
  const deps = (token: string | undefined) => ({ teams: new Map(), viewerAuth: true, accessToken: async () => token });

  it("with viewer auth on, a missing access token is 'unavailable', not 'no Quod data for this team'", async () => {
    await expect(credentialsFor("org_01ABC", deps(undefined))).rejects.toBeInstanceOf(ApiUnavailableError);
    await expect(credentialsFor("org_01ABC", { ...deps(undefined), accessToken: async () => Promise.reject(new Error("refresh failed")) })).rejects.toBeInstanceOf(ApiUnavailableError);
    expect(await credentialsFor("org_01ABC", deps("tok"))).toEqual({ team: "org_01ABC", token: "tok", kind: "viewer" });
  });

  it("a team quod-server is not set up for is still null (no Quod data)", async () => {
    expect(await credentialsFor("org_01ABC", { teams: new Map(), viewerAuth: false, accessToken: async () => undefined })).toBeNull();
    expect(await credentialsFor(null, deps("tok"))).toBeNull();
  });
});

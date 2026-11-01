import { sealData } from "iron-session";
import { afterEach, describe, expect, it, vi } from "vitest";
import { appOrigin, githubAppConfig, isOperator, normalizePem } from "./env";
import { buildManifest, checkAppName, envFromConversion, MANIFEST_TTL, newState, sealState, stateMatches } from "./manifest";

const PASSWORD = "p".repeat(32);

afterEach(() => vi.useRealTimers());

describe("manifest state check", () => {
  it("accepts the state sealed in this browser's cookie", async () => {
    const state = newState();
    expect(await stateMatches(await sealState(state, PASSWORD), state, PASSWORD)).toBe(true);
  });

  it("refuses a different state, a missing cookie, a missing state", async () => {
    const cookie = await sealState(newState(), PASSWORD);
    expect(await stateMatches(cookie, newState(), PASSWORD)).toBe(false);
    expect(await stateMatches(undefined, newState(), PASSWORD)).toBe(false);
    expect(await stateMatches(cookie, null, PASSWORD)).toBe(false);
    expect(await stateMatches(cookie, "", PASSWORD)).toBe(false);
  });

  it("refuses a tampered cookie, another password, or a blob sealed for another purpose", async () => {
    const state = newState();
    const cookie = await sealState(state, PASSWORD);
    expect(await stateMatches(cookie.slice(0, -4) + "AAAA", state, PASSWORD)).toBe(false);
    expect(await stateMatches(cookie, state, "q".repeat(32))).toBe(false);
    const other = await sealData({ purpose: "quod-oauth", state }, { password: PASSWORD, ttl: 600 });
    expect(await stateMatches(other, state, PASSWORD)).toBe(false);
  });

  it("refuses an expired cookie", async () => {
    vi.useFakeTimers();
    const state = newState();
    const cookie = await sealState(state, PASSWORD);
    vi.setSystemTime(Date.now() + (MANIFEST_TTL + 120) * 1000);
    expect(await stateMatches(cookie, state, PASSWORD)).toBe(false);
  });

  it("makes long random states", () => {
    const a = newState();
    expect(a).toMatch(/^[A-Za-z0-9_-]{32}$/);
    expect(newState()).not.toBe(a);
  });
});

describe("buildManifest", () => {
  it("has the permissions, events and URLs the product owner asked for", () => {
    const m = buildManifest("https://app.quodci.com");
    expect(m).toMatchObject({
      name: "Quod CI",
      url: "https://quodci.com",
      hook_attributes: { url: "https://app.quodci.com/api/github/webhook" },
      redirect_url: "https://app.quodci.com/setup/github-app/callback",
      setup_url: "https://app.quodci.com/github/installed",
      setup_on_update: true,
      callback_urls: ["https://app.quodci.com/github/installed"],
      request_oauth_on_install: true,
      public: true,
      default_permissions: { actions: "read", metadata: "read" },
      default_events: ["workflow_job", "workflow_run"],
    });
    expect(Object.keys(m.default_permissions).sort()).toEqual(["actions", "metadata"]);
  });

  it("derives the hosts from APP_URL, else the request origin (localhost works)", () => {
    expect(buildManifest(appOrigin("http://localhost:3000", {})).redirect_url).toBe("http://localhost:3000/setup/github-app/callback");
    expect(appOrigin("https://quod-cloud-git-x.vercel.app", { APP_URL: "https://app.quodci.com/" })).toBe("https://app.quodci.com");
    expect(buildManifest("https://app.quodci.com", "Quod CI 2").name).toBe("Quod CI 2");
  });

  it("checks the editable App name", () => {
    expect(checkAppName("  Quod   CI ")).toEqual({ ok: true, value: "Quod CI" });
    expect(checkAppName("").ok).toBe(false);
    expect(checkAppName("x".repeat(35)).ok).toBe(false);
    expect(checkAppName("<script>").ok).toBe(false);
  });
});

describe("operators and App config", () => {
  it("allows only the listed operator emails, case-insensitively", () => {
    const env = { QUOD_OPERATOR_EMAILS: "Owner@Quodci.com, ops@quodci.com" };
    expect(isOperator("owner@quodci.com", env)).toBe(true);
    expect(isOperator("OPS@quodci.com", env)).toBe(true);
    expect(isOperator("someone@quodci.com", env)).toBe(false);
    expect(isOperator("owner@quodci.com", {})).toBe(false);
    expect(isOperator("", env)).toBe(false);
  });

  it("needs all six GITHUB_APP_* values, and restores newlines in a pasted key", () => {
    expect(githubAppConfig({ GITHUB_APP_ID: "1" })).toMatchObject({ ok: false });
    expect(normalizePem("-----BEGIN RSA PRIVATE KEY-----\\nabc\\n-----END RSA PRIVATE KEY-----")).toBe("-----BEGIN RSA PRIVATE KEY-----\nabc\n-----END RSA PRIVATE KEY-----");
  });

  it("lists the six values to paste, marking the secrets", () => {
    const vars = envFromConversion({ id: 1, slug: "quod-ci", client_id: "Iv1.x", client_secret: "cs", webhook_secret: "ws", pem: "PEM" });
    expect(vars.map(([k, , s]) => `${k}${s ? "*" : ""}`)).toEqual(["GITHUB_APP_ID", "GITHUB_APP_SLUG", "GITHUB_APP_CLIENT_ID", "GITHUB_APP_CLIENT_SECRET*", "GITHUB_APP_WEBHOOK_SECRET*", "GITHUB_APP_PRIVATE_KEY*"]);
  });
});

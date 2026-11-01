import { describe, expect, it } from "vitest";
import { AuthConfigError, authMode, resolveAuthMode } from "./config";

const full = {
  WORKOS_API_KEY: "sk_test_x",
  WORKOS_CLIENT_ID: "client_x",
  WORKOS_COOKIE_PASSWORD: "x".repeat(32),
  NEXT_PUBLIC_WORKOS_REDIRECT_URI: "https://app.quodci.com/callback",
};

describe("auth mode: production fails closed", () => {
  it("refuses to serve in production with no WorkOS config", () => {
    const mode = resolveAuthMode({ NODE_ENV: "production" });
    expect(mode.kind).toBe("misconfigured");
    expect(() => authMode({ NODE_ENV: "production" })).toThrow(AuthConfigError);
  });

  it("treats blank values as missing in production", () => {
    const blank = { NODE_ENV: "production", WORKOS_API_KEY: " ", WORKOS_CLIENT_ID: "", WORKOS_COOKIE_PASSWORD: "", NEXT_PUBLIC_WORKOS_REDIRECT_URI: "" };
    expect(resolveAuthMode(blank).kind).toBe("misconfigured");
  });

  it("refuses a partial config in production", () => {
    const { WORKOS_API_KEY: _omit, ...partial } = full;
    void _omit;
    const mode = resolveAuthMode({ NODE_ENV: "production", ...partial });
    expect(mode).toEqual({ kind: "misconfigured", problems: ["Missing WORKOS_API_KEY."] });
  });

  it("never picks the demo session in production, whatever else is set", () => {
    for (const extra of [{}, { QUOD_DEMO: "1" }, { VERCEL_ENV: "preview" }]) {
      expect(resolveAuthMode({ NODE_ENV: "production", ...extra }).kind).not.toBe("demo");
    }
  });

  it("refuses a short cookie password and a plain-http redirect in production", () => {
    expect(resolveAuthMode({ NODE_ENV: "production", ...full, WORKOS_COOKIE_PASSWORD: "short" }).kind).toBe("misconfigured");
    expect(
      resolveAuthMode({ NODE_ENV: "production", ...full, NEXT_PUBLIC_WORKOS_REDIRECT_URI: "http://app.quodci.com/callback" }).kind,
    ).toBe("misconfigured");
  });

  it("uses WorkOS in production when fully configured", () => {
    expect(authMode({ NODE_ENV: "production", ...full })).toBe("workos");
  });
});

describe("auth mode: local development", () => {
  it("uses the demo session only when no WorkOS variable is set", () => {
    expect(authMode({ NODE_ENV: "development" })).toBe("demo");
    expect(authMode({ NODE_ENV: "test" })).toBe("demo");
    expect(authMode({})).toBe("demo");
  });

  it("refuses a partial config in development instead of falling back to the demo", () => {
    expect(resolveAuthMode({ NODE_ENV: "development", WORKOS_CLIENT_ID: "client_x" }).kind).toBe("misconfigured");
  });

  it("uses WorkOS in development when configured, with a localhost redirect", () => {
    expect(authMode({ NODE_ENV: "development", ...full, NEXT_PUBLIC_WORKOS_REDIRECT_URI: "http://localhost:3000/callback" })).toBe(
      "workos",
    );
  });
});

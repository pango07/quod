import { describe, expect, it } from "vitest";
import { checkCode, checkEmail, checkId, checkName, checkNewPassword, checkPassword, checkToken, sanitizeReturnTo } from "./validate";

describe("sanitizeReturnTo: same-origin paths only", () => {
  it("keeps ordinary in-app paths with query and hash", () => {
    expect(sanitizeReturnTo("/quodci")).toBe("/quodci");
    expect(sanitizeReturnTo("/quodci/quod/runs/1842?step=3#why")).toBe("/quodci/quod/runs/1842?step=3#why");
  });

  it("refuses anything that could leave the site", () => {
    for (const bad of [
      "https://evil.example",
      "http://evil.example/quodci",
      "//evil.example",
      "//evil.example/path",
      "/\\evil.example",
      "\\\\evil.example",
      "/\t/evil.example",
      "javascript:alert(1)",
      "data:text/html,hi",
      "quodci",
      "",
      " /quodci",
    ]) {
      expect(sanitizeReturnTo(bad), bad).toBe("/");
    }
  });

  it("refuses paths that only become protocol-relative after dot-segment normalization (review B1)", () => {
    for (const bad of ["/..//evil.com", "/.//evil.com", "/a/..//evil.com", "/%2e%2e//evil.com", "/.%2e//evil.com", "/..//evil.com/.."]) {
      expect(sanitizeReturnTo(bad), bad).toBe("/");
    }
    // Whatever comes out is a path on this site: one leading slash, never two.
    for (const p of ["/x/..//y", "/./x", "/x/./..//./z", "/%2E%2E//evil.com", "/..%2f/evil.com"]) {
      const out = sanitizeReturnTo(p);
      expect(out.startsWith("/") && !out.startsWith("//") && !out.startsWith("/\\"), `${p} -> ${out}`).toBe(true);
    }
  });

  it("keeps an encoded newline encoded (it stays a path on this site)", () => {
    expect(sanitizeReturnTo("/%0a/evil")).toBe("/%0a/evil");
  });

  it("refuses non-strings, huge values and backslashes anywhere", () => {
    expect(sanitizeReturnTo(undefined)).toBe("/");
    expect(sanitizeReturnTo(["/x"])).toBe("/");
    expect(sanitizeReturnTo(`/${"a".repeat(1100)}`)).toBe("/");
    expect(sanitizeReturnTo("/a\\b")).toBe("/");
  });

  it("never sends you back to a sign-in page", () => {
    expect(sanitizeReturnTo("/sign-in")).toBe("/");
    expect(sanitizeReturnTo("/sign-in?returnTo=/x")).toBe("/");
    expect(sanitizeReturnTo("/callback?code=1")).toBe("/");
    expect(sanitizeReturnTo("/sign-insights")).toBe("/sign-insights");
  });

  it("normalizes dot segments without escaping the origin", () => {
    expect(sanitizeReturnTo("/a/../b")).toBe("/b");
    expect(sanitizeReturnTo("/../../etc")).toBe("/etc");
  });
});

describe("form inputs", () => {
  it("checks emails, lowercasing and trimming them", () => {
    expect(checkEmail("  Dev@Example.COM ")).toEqual({ ok: true, value: "dev@example.com" });
    for (const bad of ["", "dev", "dev@", "@x.com", "dev@x", "a b@x.com", 42]) expect(checkEmail(bad).ok, String(bad)).toBe(false);
  });

  it("asks for a long enough new password but accepts any given password to sign in", () => {
    expect(checkNewPassword("short").ok).toBe(false);
    expect(checkNewPassword("a long passphrase").ok).toBe(true);
    expect(checkNewPassword("x".repeat(257)).ok).toBe(false);
    expect(checkPassword("short").ok).toBe(true);
    expect(checkPassword("").ok).toBe(false);
  });

  it("accepts six-digit codes, including pasted ones with spaces or dashes", () => {
    expect(checkCode("123456")).toEqual({ ok: true, value: "123456" });
    expect(checkCode(" 123 456 ")).toEqual({ ok: true, value: "123456" });
    expect(checkCode("123-456")).toEqual({ ok: true, value: "123456" });
    for (const bad of ["12345", "1234567", "abcdef", undefined]) expect(checkCode(bad).ok).toBe(false);
  });

  it("checks names, tokens and ids", () => {
    expect(checkName("  Ada   Lovelace ", "name")).toEqual({ ok: true, value: "Ada Lovelace" });
    expect(checkName("<script>", "name").ok).toBe(false);
    expect(checkToken("Abc123._~-xyz").ok).toBe(true);
    expect(checkToken("short").ok).toBe(false);
    expect(checkToken("has space in it").ok).toBe(false);
    expect(checkId("org_01HXYZABCDEF1234", "org").ok).toBe(true);
    expect(checkId("org_../../x", "org").ok).toBe(false);
    expect(checkId("user_01HXYZABCDEF1234", "org").ok).toBe(false);
  });
});

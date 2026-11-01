import { createVerify, generateKeyPairSync } from "node:crypto";
import { describe, expect, it, vi } from "vitest";
import { appJwt, exchangeUserCode, GitHubApiError, listUserInstallations } from "./api";

// No request leaves the process: fetch is a mock.

describe("GitHub REST helpers", () => {
  it("signs an App JWT GitHub can verify (RS256, iss = app id, under 10 minutes)", () => {
    const { privateKey, publicKey } = generateKeyPairSync("rsa", { modulusLength: 2048 });
    const pem = privateKey.export({ type: "pkcs1", format: "pem" }).toString();
    const jwt = appJwt("12345", pem, 1_000_000);
    const [h, p, s] = jwt.split(".");
    expect(JSON.parse(Buffer.from(h, "base64url").toString())).toEqual({ alg: "RS256", typ: "JWT" });
    expect(JSON.parse(Buffer.from(p, "base64url").toString())).toEqual({ iat: 999_940, exp: 1_000_540, iss: "12345" });
    expect(createVerify("RSA-SHA256").update(`${h}.${p}`).verify(publicKey, Buffer.from(s, "base64url"))).toBe(true);
  });

  it("treats GitHub's 200-with-error answer to a bad code as a failure", async () => {
    const f = vi.fn(async () => new Response(JSON.stringify({ error: "bad_verification_code" }), { status: 200 }));
    await expect(exchangeUserCode({ clientId: "c", clientSecret: "s", code: "abc" }, f as unknown as typeof fetch)).rejects.toBeInstanceOf(GitHubApiError);
    await expect(exchangeUserCode({ clientId: "c", clientSecret: "s", code: "../x" }, f as unknown as typeof fetch)).rejects.toBeInstanceOf(GitHubApiError);
  });

  it("pages through /user/installations", async () => {
    const page = (n: number, count: number) => ({ total_count: 150, installations: Array.from({ length: count }, (_, i) => ({ id: n * 1000 + i, account: null })) });
    const f = vi.fn(async (url: string) => new Response(JSON.stringify(/[&?]page=1$/.test(url) ? page(1, 100) : page(2, 50)), { status: 200 }));
    const all = await listUserInstallations("ghu", f as unknown as typeof fetch);
    expect(all).toHaveLength(150);
    expect(f).toHaveBeenCalledTimes(2);
    const init = (f.mock.calls[0] as unknown as [string, RequestInit])[1];
    expect((init.headers as Record<string, string>).authorization).toBe("Bearer ghu");
  });
});

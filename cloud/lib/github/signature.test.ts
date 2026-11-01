import { createHmac } from "node:crypto";
import { describe, expect, it } from "vitest";
import { signBody, verifySignature } from "./signature";

// GitHub's published test vector (docs: "Validating webhook deliveries").
const SECRET = "It's a Secret to Everybody";
const BODY = "Hello, World!";
const GOOD = "sha256=757107ea0eb2509fc211221cce984b8a37570b6d7586c22c46f4379c8b043e17";

describe("verifySignature", () => {
  it("accepts GitHub's test vector, as bytes or string", () => {
    expect(signBody(SECRET, BODY)).toBe(GOOD);
    expect(verifySignature(SECRET, BODY, GOOD)).toBe(true);
    expect(verifySignature(SECRET, new TextEncoder().encode(BODY), GOOD)).toBe(true);
  });

  it("rejects a bad signature, another body, or another secret", () => {
    expect(verifySignature(SECRET, BODY, GOOD.replace(/.$/, "f"))).toBe(false);
    expect(verifySignature(SECRET, `${BODY} `, GOOD)).toBe(false);
    expect(verifySignature("another secret", BODY, GOOD)).toBe(false);
  });

  it("rejects a missing or empty header, and a missing secret", () => {
    expect(verifySignature(SECRET, BODY, null)).toBe(false);
    expect(verifySignature(SECRET, BODY, undefined)).toBe(false);
    expect(verifySignature(SECRET, BODY, "")).toBe(false);
    expect(verifySignature("", BODY, GOOD)).toBe(false);
  });

  it("rejects the wrong algorithm: SHA-1 (X-Hub-Signature), a bare digest, other prefixes", () => {
    const sha1 = `sha1=${createHmac("sha1", SECRET).update(BODY).digest("hex")}`;
    expect(verifySignature(SECRET, BODY, sha1)).toBe(false);
    expect(verifySignature(SECRET, BODY, GOOD.slice("sha256=".length))).toBe(false);
    expect(verifySignature(SECRET, BODY, GOOD.replace("sha256=", "sha512="))).toBe(false);
    expect(verifySignature(SECRET, BODY, GOOD.replace("sha256=", "SHA256="))).toBe(false);
  });

  it("rejects malformed digests without throwing (wrong length, not hex)", () => {
    expect(verifySignature(SECRET, BODY, GOOD.slice(0, -2))).toBe(false);
    expect(verifySignature(SECRET, BODY, `${GOOD}00`)).toBe(false);
    expect(verifySignature(SECRET, BODY, `sha256=${"z".repeat(64)}`)).toBe(false);
  });
});

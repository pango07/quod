import { createHmac, timingSafeEqual } from "node:crypto";

// GitHub signs each webhook delivery: X-Hub-Signature-256 is "sha256=" and
// the hex HMAC-SHA256 of the raw request body with the App's webhook secret.
// Only that header and that algorithm are accepted (never the SHA-1
// X-Hub-Signature), and the compare is constant-time.

const FORMAT = /^sha256=([0-9a-f]{64})$/;

export function signBody(secret: string, body: Uint8Array | string): string {
  return `sha256=${createHmac("sha256", secret).update(body).digest("hex")}`;
}

/** True only for a well-formed sha256 signature of exactly these bytes. */
export function verifySignature(secret: string, body: Uint8Array | string, header: string | null | undefined): boolean {
  if (!secret || typeof header !== "string") return false;
  const m = FORMAT.exec(header.trim());
  if (!m) return false;
  const expected = createHmac("sha256", secret).update(body).digest();
  const given = Buffer.from(m[1], "hex");
  return given.length === expected.length && timingSafeEqual(given, expected);
}

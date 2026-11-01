import { sealData } from "iron-session";
import { describe, expect, it } from "vitest";
import { MAX_COOKIE_VALUE } from "./limits";

// Review M2: the pending cookie must fit in a browser cookie in its largest shape.
// Every field is at its cap: returnTo 1024 (sanitizeReturnTo), invitation token 512
// (checkToken), email 254 (checkEmail); the org list and QR code are not stored.
describe("sealed pending cookie size", () => {
  it("stays under the cookie limit with every field at its maximum", async () => {
    const worst = {
      purpose: "quod-pending",
      step: "mfa-enroll",
      token: "x".repeat(128),
      returnTo: `/${"a".repeat(1023)}`,
      email: `${"e".repeat(64)}@${"d".repeat(185)}.com`,
      userId: "user_01HXYZABCDEFGHJKMNPQRSTVWX",
      via: "verification",
      invitationToken: "t".repeat(512),
      challengeId: "auth_challenge_01HXYZABCDEFGHJKMNPQRSTVWX",
      totp: { uri: `otpauth://totp/Quod:${"e".repeat(254)}?secret=${"S".repeat(32)}&issuer=Quod`, secret: "S".repeat(32) },
    };
    const sealed = await sealData(worst, { password: "p".repeat(32), ttl: 600 });
    expect(sealed.length).toBeLessThan(MAX_COOKIE_VALUE);
  });
});

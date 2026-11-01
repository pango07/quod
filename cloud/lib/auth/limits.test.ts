import { describe, expect, it } from "vitest";
import { assertCookieFits, MAX_COOKIE_VALUE, RateLimiter, TtlCache } from "./limits";

describe("TtlCache", () => {
  it("forgets entries after their time and caps its size", () => {
    let t = 0;
    const c = new TtlCache<string>(1000, 2, () => t);
    c.set("a", "1");
    expect(c.get("a")).toBe("1");
    t = 1000;
    expect(c.get("a")).toBeUndefined();
    c.set("x", "1");
    c.set("y", "2");
    c.set("z", "3");
    expect(c.get("x")).toBeUndefined();
    expect(c.get("z")).toBe("3");
  });
});

describe("RateLimiter", () => {
  it("allows `limit` hits per window per key, then refuses until the window passes", () => {
    let t = 0;
    const r = new RateLimiter(3, 60_000, () => t);
    expect([r.take("ip"), r.take("ip"), r.take("ip"), r.take("ip")]).toEqual([true, true, true, false]);
    expect(r.take("other")).toBe(true);
    t = 60_000;
    expect(r.take("ip")).toBe(true);
  });
});

describe("cookie size (review M2)", () => {
  it("refuses a value a browser would silently drop", () => {
    expect(() => assertCookieFits("c", "x".repeat(MAX_COOKIE_VALUE - 1))).not.toThrow();
    expect(() => assertCookieFits("c", "x".repeat(MAX_COOKIE_VALUE))).toThrow(/limit/);
  });
});

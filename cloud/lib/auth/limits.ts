// Small in-memory guards for the anonymous sign-in steps, so traffic to
// /sign-in cannot burn the WorkOS API rate limit. Per server instance: on
// serverless they are best effort, which is what they are for (WorkOS and
// Radar still rate-limit and score the real attempts).

/** A map whose entries expire, with a size cap so it cannot grow without bound. */
export class TtlCache<V> {
  private readonly entries = new Map<string, { value: V; until: number }>();
  constructor(
    private readonly ttlMs: number,
    private readonly max = 5000,
    private readonly now: () => number = Date.now,
  ) {}

  get(key: string): V | undefined {
    const e = this.entries.get(key);
    if (!e) return undefined;
    if (e.until <= this.now()) {
      this.entries.delete(key);
      return undefined;
    }
    return e.value;
  }

  set(key: string, value: V): void {
    if (this.entries.size >= this.max) {
      const oldest = this.entries.keys().next().value;
      if (oldest !== undefined) this.entries.delete(oldest);
    }
    this.entries.set(key, { value, until: this.now() + this.ttlMs });
  }
}

/** A fixed-window counter per key: allow `limit` hits per `windowMs`. */
export class RateLimiter {
  private readonly hits: TtlCache<{ count: number; start: number }>;
  constructor(
    private readonly limit: number,
    private readonly windowMs: number,
    private readonly now: () => number = Date.now,
  ) {
    this.hits = new TtlCache(windowMs, 20000, now);
  }

  /** True if this hit is allowed (and counts it). */
  take(key: string): boolean {
    const t = this.now();
    const cur = this.hits.get(key);
    if (!cur || t - cur.start >= this.windowMs) {
      this.hits.set(key, { count: 1, start: t });
      return true;
    }
    if (cur.count >= this.limit) return false;
    cur.count += 1;
    return true;
  }
}

/** Browsers drop a cookie over 4096 bytes without a word; stay well under, and fail loudly instead. */
export const MAX_COOKIE_VALUE = 3800;

export function assertCookieFits(name: string, value: string): void {
  if (value.length >= MAX_COOKIE_VALUE) throw new Error(`Cookie ${name} would be ${value.length} bytes (limit ${MAX_COOKIE_VALUE}); not set.`);
}

/** WorkOS pending-authentication codes our pages do not handle yet. */
export const UNSUPPORTED_STEPS = new Set(["mfa_verification", "radar_email_challenge", "radar_sms_challenge"]);
export const UNSUPPORTED_MESSAGE = "This sign-in needs an extra step Quod doesn't support yet. Contact support@quodci.com and we'll sort it out.";

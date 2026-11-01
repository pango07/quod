import { RANGES, type Range, type TrustState } from "@/lib/api/types";

type Param = string | string[] | undefined;

export const one = (v: Param) => (Array.isArray(v) ? v[0] : v);

export function parseRange(v: Param): Range {
  const r = one(v);
  return RANGES.includes(r as Range) ? (r as Range) : "7d";
}

export const RANGE_LABEL: Record<Range, string> = { "7d": "Last 7 days", "30d": "30 days", "90d": "90 days" };
export const RANGE_FROM: Record<Range, string> = { "7d": "7 days ago", "30d": "30 days ago", "90d": "90 days ago" };

/** A run number from a URL segment or a client: a whole number from 1 to 999,999,999, else null. */
export function parseRunNumber(v: unknown): number | null {
  const s = typeof v === "number" ? String(v) : v;
  if (typeof s !== "string" || !/^\d{1,9}$/.test(s)) return null;
  const n = Number(s);
  return n >= 1 ? n : null;
}

/** A team, repo or step-key segment: letters, digits, dot, dash, underscore; never "." or "..". */
export function isSlug(v: unknown): v is string {
  return typeof v === "string" && /^[A-Za-z0-9._-]{1,100}$/.test(v) && v !== "." && v !== "..";
}

export type StepFilter = "all" | TrustState["kind"];

export function parseStepFilter(v: Param): StepFilter {
  const f = one(v);
  return f === "replaying" || f === "proving" || f === "watching" || f === "paused" ? f : "all";
}

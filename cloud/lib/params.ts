import { RANGES, type Range, type TrustState } from "@/lib/api/types";

type Param = string | string[] | undefined;

export const one = (v: Param) => (Array.isArray(v) ? v[0] : v);

export function parseRange(v: Param): Range {
  const r = one(v);
  return RANGES.includes(r as Range) ? (r as Range) : "7d";
}

export const RANGE_LABEL: Record<Range, string> = { "7d": "Last 7 days", "30d": "30 days", "90d": "90 days" };
export const RANGE_FROM: Record<Range, string> = { "7d": "7 days ago", "30d": "30 days ago", "90d": "90 days ago" };

/** A positive whole number from a URL segment, or null. */
export function parseRunNumber(v: string): number | null {
  return /^\d{1,9}$/.test(v) ? Number(v) : null;
}

export type StepFilter = "all" | TrustState["kind"];

export function parseStepFilter(v: Param): StepFilter {
  const f = one(v);
  return f === "replaying" || f === "proving" || f === "watching" || f === "paused" ? f : "all";
}

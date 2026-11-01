// Formatting for every number the dashboard shows. One place, so "2m 09s"
// means the same thing on every screen.

const pad2 = (n: number) => String(n).padStart(2, "0");

/**
 * A step or run duration, given in seconds.
 * Under a second: two decimals ("0.21s"). Under a minute: one decimal, a
 * trailing ".0" dropped ("2.9s", "46s"). Then "2m 09s", then "1h 02m".
 */
export function formatDuration(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return "n/a";
  if (seconds < 1) return `${seconds.toFixed(2)}s`;
  if (seconds < 59.95) {
    const s = seconds.toFixed(1);
    return `${s.endsWith(".0") ? s.slice(0, -2) : s}s`;
  }
  const total = Math.round(seconds);
  if (total < 3600) return `${Math.floor(total / 60)}m ${pad2(total % 60)}s`;
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  return `${h}h ${pad2(m)}m`;
}

/**
 * Saved or spent time, given in whole minutes: "0m", "28m", "2h 41m", "241h".
 * Zero minutes past the hour are dropped.
 */
export function formatMinutes(minutes: number): string {
  if (!Number.isFinite(minutes) || minutes < 0) return "n/a";
  const total = Math.round(minutes);
  if (total < 60) return `${total}m`;
  const h = Math.floor(total / 60);
  const m = total % 60;
  return m === 0 ? `${h}h` : `${h}h ${pad2(m)}m`;
}

/** "3,412" */
export function formatCount(n: number): string {
  return new Intl.NumberFormat("en-US").format(Math.round(n));
}

/** A share between 0 and 1 as a whole percent: 0.6104 -> "61%". */
export function formatPercent(share: number): string {
  if (!Number.isFinite(share)) return "n/a";
  return `${Math.round(share * 100)}%`;
}

/** "$10.27" */
export function formatUsd(amount: number): string {
  return new Intl.NumberFormat("en-US", { style: "currency", currency: "USD" }).format(amount);
}

/** "4m ago", "2h ago", "1d ago", "just now". `now` is passed in so renders stay deterministic. */
export function formatAgo(iso: string, now: string | Date): string {
  const then = new Date(iso).getTime();
  const at = (typeof now === "string" ? new Date(now) : now).getTime();
  const s = Math.max(0, Math.round((at - then) / 1000));
  if (s < 45) return "just now";
  const m = Math.round(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h}h ago`;
  const d = Math.round(h / 24);
  if (d < 30) return `${d}d ago`;
  return new Date(iso).toLocaleDateString("en-US", { month: "short", day: "numeric", timeZone: "UTC" });
}

/** "Oct 26" for a YYYY-MM-DD day. */
export function formatDay(day: string): string {
  return new Date(`${day}T00:00:00Z`).toLocaleDateString("en-US", { month: "short", day: "numeric", timeZone: "UTC" });
}

/** "7c8e644" */
export function shortSha(sha: string): string {
  return sha.slice(0, 7);
}

/** The median of a list of numbers (0 for an empty list). */
export function median(values: number[]): number {
  if (values.length === 0) return 0;
  const s = [...values].sort((a, b) => a - b);
  const mid = Math.floor(s.length / 2);
  return s.length % 2 ? s[mid] : (s[mid - 1] + s[mid]) / 2;
}

/** Initials for an avatar: "Julio Vaught" -> "JV", "jv@x.com" -> "J". */
export function initials(name: string): string {
  const words = name.replace(/@.*/, "").split(/[\s._-]+/).filter(Boolean);
  if (words.length === 0) return "?";
  return (words.length === 1 ? words[0][0] : words[0][0] + words[words.length - 1][0]).toUpperCase();
}

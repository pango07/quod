/** The team's valuation rate; independent of a runner provider's actual charges. */
export type RunnerValueRate = {
  usdPerMinute: number;
  source: "team" | "default";
};

/** Fractional runner minutes at a rate. Signed differences retain regressions. */
export function modeledRunnerValue(seconds: number | null, rate: number | null | undefined): number | null {
  if (typeof seconds !== "number" || !Number.isFinite(seconds) ||
      typeof rate !== "number" || !Number.isFinite(rate) || rate < 0) return null;
  const value = (seconds / 60) * rate;
  return Number.isFinite(value) ? value : null;
}

/** Keep sub-cent values visible; only exact zero is displayed as $0.00. */
export function formatRunnerValue(usd: number): string {
  if (!Number.isFinite(usd)) return "n/a";
  const amount = Math.abs(usd);
  const sign = usd < 0 ? "−" : "";
  if (amount === 0) return "$0.00";
  if (amount < 0.0001) return `${sign}<$0.0001`;
  const digits = amount < 1 ? 4 : 2;
  return sign + new Intl.NumberFormat("en-US", {
    style: "currency", currency: "USD", minimumFractionDigits: digits, maximumFractionDigits: digits,
  }).format(amount);
}

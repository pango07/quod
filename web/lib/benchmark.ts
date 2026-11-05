// One pinned, cold-start 12-commit window. Warm-hit probes are separate.
export const benchmark = {
  project: "convex-helpers",
  commits: 12,
  plainSeconds: 1766,
  quodSeconds: 968,
  plainMinutes: 36,
  quodMinutes: 21,
  bestHitSeconds: 14,
  bestFloorSeconds: 11,
  medianHitSeconds: 20,
  report: "https://github.com/pango07/quod/blob/ae52750/docs/bench/config-names-proof-window.md",
  run: "https://github.com/pango07/ci-bench/actions/runs/42103710173",
  support: "https://github.com/pango07/quod/blob/8fe1016/docs/bench/support-boundaries.md",
} as const;

export const timeSaved = ((1 - benchmark.quodSeconds / benchmark.plainSeconds) * 100).toFixed(1);
export const costSaved = ((1 - benchmark.quodMinutes / benchmark.plainMinutes) * 100).toFixed(1);
export const duration = (seconds: number) => `${Math.floor(seconds / 60)}m ${seconds % 60}s`;

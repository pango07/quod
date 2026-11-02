import "server-only";
import { api } from "@/lib/api";
import type { RunDetail } from "@/lib/api/types";

/** How many recent runs the plain Trust list reads for "Last result" and "Skipped". */
export const RECENT_RUNS = 12;

/**
 * The newest runs' details (their steps and receipts), for the plain Trust
 * list. Best effort: a run that cannot be read is left out, and a failure
 * gives none (the list then shows the steps' typical times instead).
 */
export async function recentRunDetails(team: string, opts: { repo?: string; limit?: number } = {}): Promise<RunDetail[]> {
  try {
    const { runs } = await api().listRuns(team, { repo: opts.repo, limit: opts.limit ?? RECENT_RUNS });
    const picked = runs.slice(0, opts.limit ?? RECENT_RUNS);
    const got = await Promise.allSettled(picked.map((r) => api().getRun(team, r.repoSlug, r.number)));
    return got.flatMap((g) => (g.status === "fulfilled" && g.value ? [g.value] : []));
  } catch {
    return [];
  }
}

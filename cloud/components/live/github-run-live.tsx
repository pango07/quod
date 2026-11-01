"use client";

import { useRouter } from "next/navigation";
import { useEffect, useRef } from "react";
import { useLiveListener } from "./provider";

/**
 * Keeps a GitHub run's page current: any job event of this run (queued,
 * started, a new step, finished) re-renders the page from the database,
 * debounced, so the jobs and steps lists follow the run.
 */
export function GitHubRunLive({ runId }: { runId: number }) {
  const router = useRouter();
  const pending = useRef<ReturnType<typeof setTimeout> | null>(null);
  useLiveListener((e) => {
    if (e.type !== "job.queued" && e.type !== "job.started" && e.type !== "job.finished") return;
    if (e.job.runId !== runId) return;
    if (pending.current) return; // one refresh per burst
    pending.current = setTimeout(() => {
      pending.current = null;
      router.refresh();
    }, 1200);
  });
  useEffect(() => () => void (pending.current && clearTimeout(pending.current)), []);
  return null;
}

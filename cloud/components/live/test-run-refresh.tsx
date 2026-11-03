"use client";

import { useRouter } from "next/navigation";
import { useEffect, useRef } from "react";
import type { TestRunSummary } from "@/lib/api/types";
import { concernsTestRun } from "@/lib/test-runs";
import { useLiveListener } from "./provider";

/** A test run's page: re-read it when its report is sent again, or when the background recording of its commit arrives (with the spot-checks). */
export function TestRunRefresh({ run }: { run: Pick<TestRunSummary, "id" | "repoSlug" | "sha" | "suite"> }) {
  const router = useRouter();
  const pending = useRef<ReturnType<typeof setTimeout> | null>(null);
  useLiveListener((e) => {
    if (e.type !== "test.run" || !concernsTestRun(e.run, run)) return;
    if (pending.current) clearTimeout(pending.current);
    pending.current = setTimeout(() => {
      pending.current = null;
      router.refresh();
    }, 1_000);
  });
  useEffect(() => () => void (pending.current && clearTimeout(pending.current)), []);
  return null;
}

"use client";

import { useRouter } from "next/navigation";
import { createContext, use, useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import type { Activity, LiveEvent, LiveEventType, LiveJob } from "@/lib/api/types";

/** connecting: first connect. live: events flowing. reconnecting: lost it, retrying. paused: tab hidden. */
export type LiveStatus = "connecting" | "live" | "reconnecting" | "paused";

interface LiveState {
  status: LiveStatus;
  /** Jobs queued or running, from the stream's snapshot plus its events; null until the snapshot arrives. */
  jobs: LiveJob[] | null;
  simulated: boolean;
  subscribe: (fn: (e: LiveEvent) => void) => () => void;
}

const Ctx = createContext<LiveState | null>(null);

const TYPES: LiveEventType[] = ["job.queued", "job.started", "job.finished", "run.started", "run.finished", "step.started", "step.finished", "trust.changed", "stats.updated"];
const MAX_BACKOFF = 30_000;

function applyToJobs(jobs: LiveJob[], e: LiveEvent): LiveJob[] {
  switch (e.type) {
    case "job.queued":
    case "job.started":
      return [...jobs.filter((j) => j.id !== e.job.id), e.job].sort((a, b) => a.queuedAt.localeCompare(b.queuedAt));
    case "job.finished":
      return jobs.filter((j) => j.id !== e.job.id);
    case "step.started":
      return jobs.map((j) => (j.id === e.jobId ? { ...j, currentStep: { index: e.index, name: e.name } } : j));
    default:
      return jobs;
  }
}

/**
 * useLiveEvents: one EventSource per tab on /[team]/events. Reconnects with
 * backoff (EventSource resumes with Last-Event-ID on its own; after a manual
 * reconnect we pass the last id and also refresh the page's data), and closes
 * while the tab is hidden.
 */
function useLiveEvents(): LiveState {
  const router = useRouter();
  const [status, setStatus] = useState<LiveStatus>("connecting");
  const [jobs, setJobs] = useState<LiveJob[] | null>(null);
  const [simulated, setSimulated] = useState(false);
  const listeners = useRef(new Set<(e: LiveEvent) => void>());
  const lastId = useRef<string | null>(null);

  useEffect(() => {
    const team = window.location.pathname.split("/").filter(Boolean)[0];
    if (!team) return;
    let es: EventSource | null = null;
    let backoff = 1000;
    let retry: ReturnType<typeof setTimeout> | undefined;
    let stopped = false;
    let opened = false;

    const open = (resume: boolean) => {
      clearTimeout(retry);
      const q = new URLSearchParams({ scope: "team" });
      if (resume && lastId.current) q.set("lastEventId", lastId.current);
      es = new EventSource(`/${encodeURIComponent(team)}/events?${q}`);
      es.onopen = () => {
        setStatus("live");
        backoff = 1000;
        // After a gap we may have missed more than a resume covers: refetch the server-rendered parts too.
        if (opened) router.refresh();
        opened = true;
      };
      es.addEventListener("snapshot", (m) => {
        const a = JSON.parse((m as MessageEvent<string>).data) as Activity;
        setJobs(a.jobs);
        setSimulated(a.simulated);
      });
      for (const t of TYPES) {
        es.addEventListener(t, (m) => {
          const msg = m as MessageEvent<string>;
          const e = JSON.parse(msg.data) as LiveEvent;
          lastId.current = msg.lastEventId || e.id;
          setJobs((cur) => (cur ? applyToJobs(cur, e) : cur));
          for (const fn of listeners.current) fn(e);
        });
      }
      es.onerror = () => {
        if (stopped) return;
        setStatus("reconnecting");
        if (es?.readyState === EventSource.CLOSED) {
          // The browser gave up (an HTTP error, or the server is down): retry ourselves, slower each time.
          es = null;
          retry = setTimeout(() => open(true), backoff);
          backoff = Math.min(backoff * 2, MAX_BACKOFF);
        }
      };
    };

    const close = () => {
      clearTimeout(retry);
      es?.close();
      es = null;
    };

    const onVisibility = () => {
      if (document.hidden) {
        close();
        setStatus("paused");
      } else if (!es) {
        setStatus("reconnecting");
        open(true);
      }
    };

    open(false);
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      stopped = true;
      document.removeEventListener("visibilitychange", onVisibility);
      close();
    };
  }, [router]);

  const subscribe = useCallback((fn: (e: LiveEvent) => void) => {
    listeners.current.add(fn);
    return () => void listeners.current.delete(fn);
  }, []);

  return useMemo(() => ({ status, jobs, simulated, subscribe }), [status, jobs, simulated, subscribe]);
}

export function LiveProvider({ children }: { children: ReactNode }) {
  const live = useLiveEvents();
  return (
    <Ctx value={live}>
      <TabTitle running={live.jobs?.filter((j) => j.status === "in_progress").length ?? 0} />
      {children}
    </Ctx>
  );
}

export function useLive(): LiveState {
  const v = use(Ctx);
  if (!v) throw new Error("useLive outside LiveProvider");
  return v;
}

/** Calls `fn` for each live event. */
export function useLiveListener(fn: (e: LiveEvent) => void) {
  const { subscribe } = useLive();
  const ref = useRef(fn);
  useEffect(() => {
    ref.current = fn;
  });
  useEffect(() => subscribe((e) => ref.current(e)), [subscribe]);
}

const PREFIX = /^● \d+ running · /;

/** "● 2 running · Overview · Quod Cloud": kept in front of whatever title the page sets. */
function TabTitle({ running }: { running: number }) {
  useEffect(() => {
    const apply = () => {
      const base = document.title.replace(PREFIX, "");
      const next = running > 0 ? `● ${running} running · ${base}` : base;
      if (document.title !== next) document.title = next;
    };
    apply();
    const obs = new MutationObserver(apply);
    obs.observe(document.head, { subtree: true, childList: true, characterData: true });
    return () => {
      obs.disconnect();
      document.title = document.title.replace(PREFIX, "");
    };
  }, [running]);
  return null;
}

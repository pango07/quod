"use client";

import { useRouter } from "next/navigation";
import { createContext, use, useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import type { Activity, LiveEvent, LiveEventType, LiveJob } from "@/lib/api/types";
import { applyToJobs } from "@/lib/live/jobs";

/**
 * connecting: first connect. live: events flowing. reconnecting: lost it for
 * 3 s or more, retrying. paused: tab hidden. signed-out / no-access: the
 * server said 401 / 404, so we stop retrying.
 */
export type LiveStatus = "connecting" | "live" | "reconnecting" | "paused" | "signed-out" | "no-access";

interface LiveState {
  status: LiveStatus;
  /** Jobs queued or running, from the stream's snapshot plus its events; null until the snapshot arrives. */
  jobs: LiveJob[] | null;
  simulated: boolean;
  subscribe: (fn: (e: LiveEvent) => void) => () => void;
}

const Ctx = createContext<LiveState | null>(null);

const TYPES: LiveEventType[] = ["job.queued", "job.started", "job.finished", "run.started", "run.finished", "step.started", "step.finished", "trust.changed", "stats.updated", "test.run"];
const MAX_BACKOFF = 30_000;
/** Don't show or announce "Reconnecting" for blips shorter than this. */
const RECONNECTING_AFTER = 3_000;

/**
 * useLiveEvents: one EventSource per tab on /[team]/events.
 * - A planned close (the server says "bye" before its ~4 minute limit) reconnects silently.
 * - Other drops show "Reconnecting" only after 3 s; EventSource resumes with Last-Event-ID.
 * - If the browser gives up, we probe once (HEAD) and stop on 401/404, else retry with backoff.
 * - Hidden tabs close the stream; showing the tab resumes from the last id.
 * - Only an unplanned gap refreshes the page's server-rendered data.
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
    const url = `/${encodeURIComponent(team)}/events`;
    let es: EventSource | null = null;
    let backoff = 1000;
    let retry: ReturnType<typeof setTimeout> | undefined;
    let slow: ReturnType<typeof setTimeout> | undefined;
    let stopped = false;
    let planned = false;
    let gap = false;

    const lost = () => {
      clearTimeout(slow);
      slow = setTimeout(() => setStatus((s) => (s === "live" || s === "connecting" ? "reconnecting" : s)), RECONNECTING_AFTER);
    };

    const open = (resume: boolean) => {
      clearTimeout(retry);
      const q = new URLSearchParams({ scope: "team" });
      if (resume && lastId.current) q.set("lastEventId", lastId.current);
      es = new EventSource(`${url}?${q}`);
      es.onopen = () => {
        clearTimeout(slow);
        setStatus("live");
        backoff = 1000;
        // Only a real gap (not our planned ~4 minute rotation) may have missed more than a resume covers.
        if (gap) router.refresh();
        gap = false;
        planned = false;
      };
      es.addEventListener("bye", () => {
        planned = true;
      });
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
        if (!planned) {
          gap = true;
          lost();
        }
        if (es?.readyState === EventSource.CLOSED) {
          es = null;
          void probe();
        }
      };
    };

    // The browser gave up on the stream: find out why before retrying.
    const probe = async () => {
      let code = 0;
      try {
        code = (await fetch(url, { method: "HEAD", headers: { accept: "text/event-stream" }, cache: "no-store" })).status;
      } catch {
        // Offline: keep retrying.
      }
      if (stopped) return;
      if (code === 401) return void (clearTimeout(slow), setStatus("signed-out"));
      if (code === 404) return void (clearTimeout(slow), setStatus("no-access"));
      gap = true;
      lost();
      retry = setTimeout(() => open(true), backoff);
      backoff = Math.min(backoff * 2, MAX_BACKOFF);
    };

    const close = () => {
      clearTimeout(retry);
      clearTimeout(slow);
      es?.close();
      es = null;
    };

    const onVisibility = () => {
      if (document.hidden) {
        close();
        setStatus((s) => (s === "signed-out" || s === "no-access" ? s : "paused"));
      } else if (!es) {
        setStatus((s) => (s === "signed-out" || s === "no-access" ? s : "connecting"));
        gap = true;
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

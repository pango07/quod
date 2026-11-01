import type { Activity, LiveEvent } from "@/lib/api/types";
import { encodeComment, encodeMessage } from "./sse";

export interface StreamOptions {
  /** The events to send; must end soon after `signal` aborts. */
  events: (signal: AbortSignal) => AsyncIterable<LiveEvent>;
  /** Sent first, as an unnumbered "snapshot" message: jobs queued or running now. */
  snapshot?: Activity;
  /** A comment line this often keeps proxies and HTTP/1.1 clients from dropping the idle connection. */
  heartbeatMs?: number;
  /**
   * Close cleanly before the platform's function limit (Vercel: 300 s by default
   * on every plan), so the browser reconnects with Last-Event-ID instead of seeing a 504.
   */
  maxMs?: number;
  /** Aborts when the client goes away. */
  signal?: AbortSignal;
}

export const HEARTBEAT_MS = 15_000;
export const MAX_STREAM_MS = 240_000;
/** Reconnect delay we ask EventSource to use. */
export const RETRY_MS = 2_000;

export const SSE_HEADERS = {
  "content-type": "text/event-stream; charset=utf-8",
  "cache-control": "no-cache, no-store, no-transform, private",
  connection: "keep-alive",
  // Tells nginx-style proxies not to buffer the stream.
  "x-accel-buffering": "no",
} as const;

/** A byte stream of SSE messages: retry hint, snapshot, events, heartbeats; closes after `maxMs`. */
export function liveStream(opts: StreamOptions): ReadableStream<Uint8Array> {
  const enc = new TextEncoder();
  const stop = new AbortController();
  opts.signal?.addEventListener("abort", () => stop.abort(), { once: true });
  let heartbeat: ReturnType<typeof setInterval> | undefined;
  let deadline: ReturnType<typeof setTimeout> | undefined;

  return new ReadableStream<Uint8Array>({
    async start(controller) {
      const send = (text: string) => {
        if (!stop.signal.aborted || text.startsWith(":")) {
          try {
            controller.enqueue(enc.encode(text));
          } catch {
            stop.abort();
          }
        }
      };
      send(encodeMessage({ retry: RETRY_MS, data: "", event: "hello" }));
      if (opts.snapshot) send(encodeMessage({ event: "snapshot", data: JSON.stringify(opts.snapshot) }));
      heartbeat = setInterval(() => send(encodeComment("ping")), opts.heartbeatMs ?? HEARTBEAT_MS);
      deadline = setTimeout(() => stop.abort(), opts.maxMs ?? MAX_STREAM_MS);
      try {
        for await (const e of opts.events(stop.signal)) {
          if (stop.signal.aborted) break;
          send(encodeMessage({ id: e.id, event: e.type, data: JSON.stringify(e) }));
        }
      } catch (err) {
        // Stopping (deadline or client gone) aborts the source; that is not a failure.
        if (!stop.signal.aborted) console.warn("[quod-cloud] event stream failed", { message: err instanceof Error ? err.message : String(err) });
      } finally {
        clearInterval(heartbeat);
        clearTimeout(deadline);
        try {
          // "bye": a planned close. The browser reconnects with Last-Event-ID without showing "Reconnecting".
          controller.enqueue(enc.encode(encodeMessage({ event: "bye", data: "{}" })));
          controller.enqueue(enc.encode(encodeComment("closing; reconnect with Last-Event-ID")));
          controller.close();
        } catch {
          // The client already went away.
        }
      }
    },
    cancel() {
      stop.abort();
      clearInterval(heartbeat);
      clearTimeout(deadline);
    },
  });
}

// Ends a long-lived stream when the viewer is no longer allowed to see it.

/**
 * Yields `source`'s items while `check()` keeps answering true. It is asked
 * every `everyMs`; false, or a failure, aborts the source's signal and ends
 * the stream (the browser reconnects, and the reconnect re-checks access).
 */
export async function* whileAllowed<T>(
  source: (signal: AbortSignal) => AsyncIterable<T>,
  opts: { signal: AbortSignal; everyMs: number; check: () => Promise<boolean> },
): AsyncGenerator<T> {
  const ac = new AbortController();
  const stop = () => ac.abort();
  if (opts.signal.aborted) return;
  opts.signal.addEventListener("abort", stop, { once: true });
  let checking = false;
  const timer = setInterval(() => {
    if (checking || ac.signal.aborted) return;
    checking = true;
    opts
      .check()
      .then((ok) => (ok ? undefined : stop()), stop)
      .finally(() => (checking = false));
  }, opts.everyMs);
  try {
    for await (const item of source(ac.signal)) {
      if (ac.signal.aborted) return;
      yield item;
    }
  } finally {
    clearInterval(timer);
    opts.signal.removeEventListener("abort", stop);
    stop();
  }
}

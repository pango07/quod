/**
 * Interleaves async iterables in arrival order, until all end or `signal`
 * aborts. Each source is pulled one item at a time, so a slow consumer
 * slows every source rather than buffering without bound.
 */
export async function* mergeAsync<T>(sources: AsyncIterable<T>[], signal: AbortSignal): AsyncGenerator<{ index: number; value: T }> {
  const its = sources.map((s) => s[Symbol.asyncIterator]());
  const pending = new Map<number, Promise<{ index: number; r: IteratorResult<T> }>>();
  const pull = (i: number) => pending.set(i, its[i].next().then((r) => ({ index: i, r })));
  its.forEach((_, i) => pull(i));
  const aborted = new Promise<null>((resolve) => {
    if (signal.aborted) resolve(null);
    signal.addEventListener("abort", () => resolve(null), { once: true });
  });
  try {
    while (pending.size && !signal.aborted) {
      const next = await Promise.race([...pending.values(), aborted]);
      if (!next) break;
      if (next.r.done) {
        pending.delete(next.index);
        continue;
      }
      pull(next.index);
      yield { index: next.index, value: next.r.value };
    }
  } finally {
    await Promise.allSettled(its.map((it) => it.return?.()));
  }
}

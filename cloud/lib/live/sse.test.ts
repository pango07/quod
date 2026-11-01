import { describe, expect, it } from "vitest";
import { encodeComment, encodeMessage, SseDecoder } from "./sse";
import { liveStream } from "./stream";

describe("SSE encoder", () => {
  it("frames id, event and data, ending with a blank line", () => {
    expect(encodeMessage({ id: "1-2-3", event: "run.started", data: '{"a":1}' })).toBe('id: 1-2-3\nevent: run.started\ndata: {"a":1}\n\n');
  });

  it("splits multi-line data and cannot be broken by newlines in id or event", () => {
    expect(encodeMessage({ data: "a\nb\r\nc" })).toBe("data: a\ndata: b\ndata: c\n\n");
    expect(encodeMessage({ id: "x\ny", event: "e\r\nvil", data: "d" })).toBe("id: xy\nevent: evil\ndata: d\n\n");
  });

  it("writes retry hints and comments", () => {
    expect(encodeMessage({ retry: 2000.7, data: "" })).toBe("retry: 2000\ndata: \n\n");
    expect(encodeComment("ping\n")).toBe(": ping\n\n");
  });

  it("round-trips through the decoder, across arbitrary chunk boundaries", () => {
    const text = encodeComment("hi") + encodeMessage({ id: "7", event: "step.finished", data: "line1\nline2" }) + encodeMessage({ event: "x", data: "{}" });
    for (let cut = 1; cut < text.length; cut += 7) {
      const d = new SseDecoder();
      const got = [...d.push(text.slice(0, cut)), ...d.push(text.slice(cut))];
      expect(got).toEqual([
        { id: "7", event: "step.finished", data: "line1\nline2", retry: undefined },
        { id: "7", event: "x", data: "{}", retry: undefined },
      ]);
    }
  });
});

describe("liveStream", () => {
  it("sends the retry hint and snapshot, then events as id/event/data, and closes at its deadline", async () => {
    const ev = { type: "run.started", id: "1-0-0", at: "2026-11-01T00:00:00.000Z" } as never;
    const body = liveStream({
      snapshot: { asOf: "2026-11-01T00:00:00.000Z", jobs: [], simulated: true },
      maxMs: 50,
      heartbeatMs: 10,
      async *events(signal) {
        yield ev;
        while (!signal.aborted) await new Promise((r) => setTimeout(r, 5));
      },
    });
    const text = await new Response(body).text();
    expect(text).toContain("retry: 2000\n");
    expect(text).toContain('event: snapshot\ndata: {"asOf"');
    expect(text).toContain('id: 1-0-0\nevent: run.started\ndata: {"type":"run.started"');
    expect(text).toContain(": ping\n\n");
    expect(text.trimEnd().endsWith(": closing; reconnect with Last-Event-ID")).toBe(true);
  });
});

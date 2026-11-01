// Server-Sent Events framing (https://html.spec.whatwg.org/multipage/server-sent-events.html).
// The encoder is used by our route to the browser; the decoder by the HTTP
// adapter reading quod-server's stream. Both are pure and unit-tested.

export interface SseMessage {
  id?: string;
  event?: string;
  data: string;
  /** Reconnection delay hint, milliseconds. */
  retry?: number;
}

const clean = (v: string) => v.replace(/[\r\n]/g, "");

/** One message. Multi-line data becomes several data: lines; ids and event names cannot break the framing. */
export function encodeMessage(m: SseMessage): string {
  let out = "";
  if (m.retry !== undefined) out += `retry: ${Math.max(0, Math.floor(m.retry))}\n`;
  if (m.id !== undefined) out += `id: ${clean(m.id)}\n`;
  if (m.event !== undefined) out += `event: ${clean(m.event)}\n`;
  for (const line of m.data.split(/\r\n|\r|\n/)) out += `data: ${line}\n`;
  return `${out}\n`;
}

/** A comment line: ignored by EventSource, keeps proxies from closing an idle connection. */
export function encodeComment(text: string): string {
  return `: ${clean(text)}\n\n`;
}

/** Incremental decoder: feed text chunks, get whole messages. Comments and unknown fields are skipped. */
export class SseDecoder {
  private buffer = "";
  private data: string[] = [];
  private id: string | undefined;
  private event: string | undefined;
  private retry: number | undefined;

  push(chunk: string): SseMessage[] {
    this.buffer += chunk;
    const out: SseMessage[] = [];
    let nl: number;
    while ((nl = this.buffer.search(/\r\n|\r|\n/)) >= 0) {
      const line = this.buffer.slice(0, nl);
      const width = this.buffer.startsWith("\r\n", nl) ? 2 : 1;
      this.buffer = this.buffer.slice(nl + width);
      if (line === "") {
        if (this.data.length) out.push({ data: this.data.join("\n"), id: this.id, event: this.event, retry: this.retry });
        this.data = [];
        this.event = undefined;
        this.retry = undefined;
        continue;
      }
      if (line.startsWith(":")) continue;
      const colon = line.indexOf(":");
      const field = colon < 0 ? line : line.slice(0, colon);
      let value = colon < 0 ? "" : line.slice(colon + 1);
      if (value.startsWith(" ")) value = value.slice(1);
      if (field === "data") this.data.push(value);
      else if (field === "id") this.id = value;
      else if (field === "event") this.event = value;
      else if (field === "retry" && /^\d+$/.test(value)) this.retry = Number(value);
    }
    return out;
  }
}

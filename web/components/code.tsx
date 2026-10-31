import { Fragment } from "react";

const KEYWORDS = new Set(
  "pub fn let mut match if else return Some None Ok Err Self self struct enum impl use mod const for in while loop as ref where true false move async await type".split(
    " ",
  ),
);
const TOKEN = /(\/\/[^\n]*)|("(?:[^"\\]|\\.)*")|\b([A-Za-z_][A-Za-z0-9_]*)\b|(\b\d[\d_.]*\b)/g;

/** Rust-ish highlighting in grey scale: comments faint, strings mid, keywords bright. */
function paint(line: string) {
  const out: React.ReactNode[] = [];
  let last = 0;
  for (const m of line.matchAll(TOKEN)) {
    const i = m.index ?? 0;
    if (i > last) out.push(line.slice(last, i));
    const [tok, comment, str, word, num] = m;
    const cls = comment ? "c-com" : str ? "c-str" : num ? "c-num" : word && KEYWORDS.has(word) ? "c-kw" : word && /^[A-Z]/.test(word) ? "c-ty" : "";
    out.push(cls ? <span key={i} className={cls}>{tok}</span> : tok);
    last = i + tok.length;
  }
  if (last < line.length) out.push(line.slice(last));
  return out;
}

/** A code panel. `hl` lists 1-based lines to light up; the rest dim while any are lit. */
export function Code({ src, file, hl = [] }: { src: string; file?: string; hl?: number[] }) {
  const lines = src.replace(/\n$/, "").split("\n");
  const lit = new Set(hl);
  return (
    <figure className="code">
      {file ? <figcaption className="code-file">{file}</figcaption> : null}
      <pre>
        <code>
          {lines.map((l, i) => (
            <Fragment key={i}>
              <span className={`code-ln ${lit.size === 0 ? "" : lit.has(i + 1) ? "lit" : "dim"}`}>{paint(l)}</span>
              {"\n"}
            </Fragment>
          ))}
        </code>
      </pre>
    </figure>
  );
}

import Link from "next/link";
import type { ReactNode } from "react";

type Kind = "note" | "tip" | "warning" | "soon";
const TITLE: Record<Kind, string> = { note: "Note", tip: "Tip", warning: "Warning", soon: "Coming soon" };

/** Note, Tip, Warning or Coming soon. The label leads the first line, as in a sentence. */
export function Callout({ type = "note", title, children }: { type?: Kind; title?: string; children: ReactNode }) {
  return (
    <aside className={`d-callout ${type}`}>
      <span className="d-callout-icon" aria-hidden="true">
        {type === "warning" ? "!" : type === "tip" ? "→" : type === "soon" ? "○" : "i"}
      </span>
      <div>
        <strong>{title ?? TITLE[type]}:</strong> {children}
      </div>
    </aside>
  );
}

export const Note = (p: { title?: string; children: ReactNode }) => <Callout type="note" {...p} />;
export const Tip = (p: { title?: string; children: ReactNode }) => <Callout type="tip" {...p} />;
export const Warning = (p: { title?: string; children: ReactNode }) => <Callout type="warning" {...p} />;
export const Soon = (p: { title?: string; children: ReactNode }) => <Callout type="soon" {...p} />;

/** An inline "Coming soon" or "Experimental" label. */
export function Badge({ children }: { children: ReactNode }) {
  return <span className="d-badge">{children}</span>;
}

/** Numbered steps: every h3 inside is a step. */
export function Steps({ children }: { children: ReactNode }) {
  return <div className="d-steps">{children}</div>;
}

/** "Next steps" link cards. */
export function Cards({ children }: { children: ReactNode }) {
  return <div className="d-cards">{children}</div>;
}

export function Card({ href, title, children }: { href: string; title: string; children: ReactNode }) {
  return (
    <Link href={href} className="d-card">
      <span className="d-card-title">
        {title}
        <span aria-hidden="true"> &rarr;</span>
      </span>
      <span className="d-card-body">{children}</span>
    </Link>
  );
}

/** Headline numbers. */
export function Stats({ children }: { children: ReactNode }) {
  return <div className="d-stats">{children}</div>;
}

export function Stat({ value, label }: { value: string; label: ReactNode }) {
  return (
    <div>
      <b>{value}</b>
      <span>{label}</span>
    </div>
  );
}

/** Your test job reads the ledger and, as it runs, records into it. Stacks vertically on phones. */
export function FlowDiagram() {
  return (
    <figure className="d-flow" aria-label="How your test job reads the ledger in Quod Cloud and records into it">
      <div className="d-flow-row">
        <div className="d-flow-box">
          <span className="d-flow-k">Your test job</span>
          <b>check</b>
          <ol>
            <li>Read the newest ledger</li>
            <li>Unchanged, witnessed files: replay</li>
            <li>The rest: run</li>
            <li>The job passes if plain vitest would</li>
          </ol>
        </div>
        <div className="d-flow-arrow" aria-hidden="true">
          <span>reads</span>
        </div>
        <div className="d-flow-box mid">
          <span className="d-flow-k">Quod Cloud</span>
          <b>ledger</b>
          <ol>
            <li>Per test file: its inputs and result</li>
            <li>Signed answers</li>
            <li>Writer branches write trusted entries</li>
          </ol>
        </div>
        <div className="d-flow-arrow" aria-hidden="true">
          <span>saves</span>
        </div>
        <div className="d-flow-box">
          <span className="d-flow-k">In the same job</span>
          <b>record</b>
          <ol>
            <li>Files that ran: recorded</li>
            <li>A second run agrees: replays from then on</li>
            <li>A run disagrees: quarantined</li>
            <li>Failed while recording: run again plain</li>
          </ol>
        </div>
      </div>
      <figcaption>One job replays and records. A record replays only once a second run agrees with it, and recording never turns a green job red.</figcaption>
    </figure>
  );
}

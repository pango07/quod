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

/** The two jobs and the ledger between them. Stacks vertically on phones. */
export function FlowDiagram() {
  return (
    <figure className="d-flow" aria-label="How a check job, Quod Cloud and the shadow job fit together">
      <div className="d-flow-row">
        <div className="d-flow-box">
          <span className="d-flow-k">Your test job</span>
          <b>check</b>
          <ol>
            <li>Read the newest ledger</li>
            <li>Unchanged files: replay</li>
            <li>The rest: run as plain vitest</li>
            <li>Upload per-test results</li>
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
        <div className="d-flow-arrow back" aria-hidden="true">
          <span>saves</span>
        </div>
        <div className="d-flow-box">
          <span className="d-flow-k">After your test job</span>
          <b>shadow</b>
          <ol>
            <li>Same command, with recording</li>
            <li>Keep only files whose results match</li>
            <li>Audit a few replays</li>
            <li>Save the ledger</li>
          </ol>
        </div>
      </div>
      <figcaption>The test job never writes. The shadow job records, and it is never your check.</figcaption>
    </figure>
  );
}

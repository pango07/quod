import type { ReactNode } from "react";
import type { TrustState } from "@/lib/api/types";
import { trustLabel, trustTone, type Tone } from "@/lib/trust";

export function Logo({ size = 22 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" aria-hidden="true">
      <path d="M12 3 21 8l-9 5-9-5 9-5Z" fill="#ededed" />
      <path d="m3 12 9 5 9-5" fill="none" stroke="#ededed" strokeWidth="1.8" />
      <path d="m3 16 9 5 9-5" fill="none" stroke="#ededed" strokeWidth="1.8" />
    </svg>
  );
}

const DOT: Record<Tone, string> = { blue: "blue", amber: "amber", green: "green", gray: "gray", dim: "" };

export function Dot({ tone, size }: { tone: Tone | "white" | "red"; size?: "sm" | "lg" }) {
  const cls = tone === "white" || tone === "red" ? tone : DOT[tone];
  return <span className={`dot ${cls} ${size ?? ""}`} aria-hidden="true" />;
}

export function Chip({ tone = "gray", children, dot }: { tone?: Tone; children: ReactNode; dot?: boolean }) {
  return (
    <span className={`chip ${tone === "blue" || tone === "amber" || tone === "green" ? tone : ""}`}>
      {dot ? <Dot tone={tone === "gray" ? "white" : tone} size="sm" /> : null}
      {children}
    </span>
  );
}

export function TrustChip({ trust, dot = true }: { trust: TrustState; dot?: boolean }) {
  const tone = trustTone(trust.kind);
  return (
    <span className={`chip ${tone === "blue" || tone === "amber" ? tone : ""}`}>
      {dot ? <Dot tone={tone === "gray" ? "white" : tone} size="sm" /> : null}
      {trustLabel(trust)}
    </span>
  );
}

export function OutcomeChip({ outcome }: { outcome: "replayed" | "ran" }) {
  return <span className={`chip ${outcome === "replayed" ? "blue" : ""}`}>{outcome === "replayed" ? "Replayed" : "Ran"}</span>;
}

export function Stat({ label, value, note, muted }: { label: string; value: ReactNode; note?: ReactNode; muted?: boolean }) {
  return (
    <div className="card stat">
      <span className="label">{label}</span>
      <span className="value" style={muted ? { color: "var(--muted)" } : undefined}>
        {value}
      </span>
      {note ? <span className="note">{note}</span> : null}
    </div>
  );
}

export function Sk({ w = "100%", h = 14, r }: { w?: number | string; h?: number | string; r?: number }) {
  return <span className="sk" style={{ display: "block", width: w, height: h, borderRadius: r }} aria-hidden="true" />;
}

export function EmptyState({ title, children, action }: { title: string; children?: ReactNode; action?: ReactNode }) {
  return (
    <div className="empty">
      <h2 className="h2" style={{ fontSize: 17 }}>
        {title}
      </h2>
      {children ? <p>{children}</p> : null}
      {action}
    </div>
  );
}

const ICONS: Record<string, ReactNode> = {
  overview: (
    <>
      <rect x="3" y="3" width="7" height="7" rx="1" />
      <rect x="14" y="3" width="7" height="7" rx="1" />
      <rect x="3" y="14" width="7" height="7" rx="1" />
      <rect x="14" y="14" width="7" height="7" rx="1" />
    </>
  ),
  repos: (
    <>
      <path d="M6 3v12" />
      <circle cx="18" cy="6" r="3" />
      <circle cx="6" cy="18" r="3" />
      <path d="M18 9a9 9 0 0 1-9 9" />
    </>
  ),
  runs: <path d="M5 3l14 9-14 9V3Z" />,
  trust: <path d="M12 3l8 4v5c0 5-3.5 8-8 9-4.5-1-8-4-8-9V7l8-4Z" />,
  runners: (
    <>
      <rect x="3" y="4" width="18" height="6" rx="1" />
      <rect x="3" y="14" width="18" height="6" rx="1" />
      <path d="M7 7h.01M7 17h.01" />
    </>
  ),
  usage: (
    <>
      <path d="M3 3v18h18" />
      <path d="M7 15l4-4 3 3 5-6" />
    </>
  ),
  settings: (
    <>
      <circle cx="12" cy="12" r="3" />
      <path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1A2 2 0 1 1 4.3 17l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1A2 2 0 1 1 7 4.3l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1A2 2 0 1 1 19.7 7l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1Z" />
    </>
  ),
  search: (
    <>
      <circle cx="11" cy="11" r="7" />
      <path d="m20 20-3.5-3.5" />
    </>
  ),
  chevron: <path d="m9 6 6 6-6 6" />,
  branch: (
    <>
      <path d="M6 3v12" />
      <circle cx="18" cy="6" r="3" />
      <circle cx="6" cy="18" r="3" />
      <path d="M18 9a9 9 0 0 1-9 9" />
    </>
  ),
  updown: <path d="m7 15 5 5 5-5M7 9l5-5 5 5" />,
  check: <path d="M20 6 9 17l-5-5" />,
  plus: <path d="M12 5v14M5 12h14" />,
};

export function Icon({ name, size = 16, stroke = "currentColor" }: { name: keyof typeof ICONS | string; size?: number; stroke?: string }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke={stroke} strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      {ICONS[name]}
    </svg>
  );
}

export function Avatar({ name, url }: { name: string; url: string | null }) {
  const letter = (name.trim()[0] ?? "?").toUpperCase();
  // eslint-disable-next-line @next/next/no-img-element
  return url ? <img src={url} alt="" /> : <>{letter}</>;
}

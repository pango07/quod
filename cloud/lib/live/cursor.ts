// Stream positions when a team's live events come from two sources: the
// GitHub App (job events, ids "gh-<version>") and Quod (run, step, trust and
// stats events, opaque ids from quod-server or the simulator).
//
// A merged stream's event ids carry both positions: "gh-<version>" alone, or
// "gh-<version>,<quod id>" once Quod has a position too. A client sends the
// last one back as Last-Event-ID and each source resumes from its own part.
// Streams with a single source keep that source's ids unchanged.

export interface SplitCursor {
  gh: string | null;
  quod: string | null;
}

const GH = /^gh-\d{1,15}$/;

export function joinCursor(gh: string | null, quod: string | null): string | null {
  if (!gh) return quod;
  return quod ? `${gh},${quod}` : gh;
}

export function splitCursor(c: string | null | undefined): SplitCursor {
  if (!c) return { gh: null, quod: null };
  if (!c.startsWith("gh-")) return { gh: null, quod: c };
  const comma = c.indexOf(",");
  const gh = comma < 0 ? c : c.slice(0, comma);
  const quod = comma < 0 ? null : c.slice(comma + 1) || null;
  return { gh: GH.test(gh) ? gh : null, quod };
}

// Teams for the viewer's WorkOS organizations: the dashboard owns team slugs.
//
// A team is one WorkOS Organization, addressed by its org id everywhere that
// matters (data, quod-server, GitHub links). The slug is only the URL segment:
// made per viewer from the org's name, so it differs from the viewer's other
// teams and from the app's own top-level paths. quod-server is never asked
// (its `POST /v1/teams/resolve` is 501, and is not needed: docs/api.md).

import { TEAMS } from "./fixture-data";
import type { OrgRef, Team } from "./types";

const slugify = (s: string) =>
  s
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 40) || "team";

/** Top-level paths a team slug must not take. */
const RESERVED_TEAM_SLUGS = new Set(["api", "callback", "github", "invite", "reset-password", "setup", "sign-in", "sign-out", "sign-up", "signed-out", "verify-email"]);

/**
 * The demo session's teams keep their slugs. Every other org is a real team,
 * never a sample team, whatever its name; its slug only has to differ from the
 * viewer's other teams (slugs are per viewer, so a real org named "quodci" is /quodci).
 */
export function resolveTeamsLocally(orgs: OrgRef[]): Team[] {
  const known = new Map(orgs.map((o) => [o.orgId, TEAMS.find((t) => t.orgId === o.orgId)]));
  const taken = new Set([...known.values()].flatMap((t) => (t ? [t.slug] : [])));
  return orgs.map((org) => {
    const demo = known.get(org.orgId);
    if (demo) return { ...demo };
    const base = slugify(org.name);
    let slug = base;
    for (let i = 2; taken.has(slug) || RESERVED_TEAM_SLUGS.has(slug); i++) slug = `${base}-${i}`;
    taken.add(slug);
    return { slug, name: org.name, plan: "Hobby", orgId: org.orgId } satisfies Team;
  });
}

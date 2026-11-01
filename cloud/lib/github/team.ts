// A Quod team as the GitHub tables know it: the WorkOS organization id.
// Every query in lib/github/live.ts takes a TeamId, and the only way to get
// one is from a TeamContext, which requireTeam (lib/dal.ts) returns after its
// membership check. A raw installation id or team string from a client never
// reaches a query.

declare const brand: unique symbol;
export type TeamId = string & { readonly [brand]: "TeamId" };

export function teamIdOf(ctx: { team: { orgId: string } }): TeamId {
  if (!ctx.team.orgId) throw new Error("team has no organization id");
  return ctx.team.orgId as TeamId;
}

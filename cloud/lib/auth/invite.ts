// When an invitation counts as proof that someone owns an inbox. Pure, so it is unit-tested.
//
// WorkOS only ever mails an invitation token to the invited address, so
// whoever holds a live one has read that inbox. WorkOS applies the same rule
// to invitations accepted through AuthKit ("accepting an invitation counts as
// email verification"). Quod's own sign-up uses it so an invited teammate is
// never asked for a second, separate email code.

export interface InvitationLike {
  email: string;
  state: string;
  expiresAt: Date | string | null;
  organizationId?: string | null;
}

const same = (a: string, b: string) => a.trim().toLowerCase() === b.trim().toLowerCase();

/** True when this invitation is pending, unexpired and addressed to exactly this email. */
export function invitationProvesEmail(inv: InvitationLike | null | undefined, email: string, now: number = Date.now()): boolean {
  if (!inv || inv.state !== "pending") return false;
  if (!email || !same(inv.email, email)) return false;
  const expires = inv.expiresAt ? new Date(inv.expiresAt).getTime() : NaN;
  // An invitation without a readable expiry is not trusted as proof.
  return Number.isFinite(expires) && expires > now;
}

/** True when a signed-in user may accept this invitation themselves: their verified email is the one invited. */
export function mayAcceptInvitation(inv: InvitationLike | null | undefined, user: { email: string; emailVerified: boolean } | null | undefined, now: number = Date.now()): boolean {
  if (!user?.emailVerified) return false;
  return invitationProvesEmail(inv, user.email, now) && Boolean(inv?.organizationId);
}

/** WorkOS role slugs. Quod uses the two default roles: admins change settings, members look. */
export type Role = "admin" | "member" | (string & {});

export interface Membership {
  orgId: string;
  orgName: string;
  role: Role;
}

export interface Viewer {
  id: string;
  name: string;
  email: string;
  avatarUrl: string | null;
  /** True for the local demo session (no WorkOS configured, not production). */
  demo: boolean;
  /** The organization the WorkOS session is currently scoped to. */
  currentOrgId: string | null;
  memberships: Membership[];
}

export interface Member {
  id: string;
  name: string | null;
  email: string;
  role: Role;
  status: "active" | "invited" | "inactive";
  /** Provisioned by directory sync: change it in the identity provider, not here. */
  directoryManaged: boolean;
}

export const isAdmin = (role: Role) => role === "admin";

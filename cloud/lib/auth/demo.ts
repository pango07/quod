// The local demo session: used only when authMode() says "demo", which needs
// every WorkOS variable absent AND NODE_ENV other than "production"
// (config.ts). Nothing here is reachable in a production server.

import type { Member, Viewer } from "./types";

export const DEMO_VIEWER: Viewer = {
  id: "user_demo",
  name: "Jamie Rivera",
  email: "jamie@example.com",
  avatarUrl: null,
  demo: true,
  currentOrgId: "org_demo_quodci",
  memberships: [
    { orgId: "org_demo_quodci", orgName: "quodci", role: "admin" },
    { orgId: "org_demo_jvaught", orgName: "jvaught", role: "member" },
  ],
};

export const DEMO_MEMBERS: Record<string, Member[]> = {
  org_demo_quodci: [
    { id: "om_demo_1", name: "Jamie Rivera", email: "jamie@example.com", role: "admin", status: "active", directoryManaged: false },
    { id: "om_demo_2", name: "Morgan Lee", email: "morgan@example.com", role: "member", status: "active", directoryManaged: false },
    { id: "om_demo_3", name: "Ade Okafor", email: "ade@example.com", role: "member", status: "active", directoryManaged: false },
    { id: "inv_demo_1", name: null, email: "sam@example.com", role: "member", status: "invited", directoryManaged: false },
  ],
  org_demo_jvaught: [
    { id: "om_demo_4", name: "Jamie Rivera", email: "jamie@example.com", role: "member", status: "active", directoryManaged: false },
  ],
};

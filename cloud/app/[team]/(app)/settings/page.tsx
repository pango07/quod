import { InviteForm, PortalButtons } from "@/components/settings-forms";
import { Chip } from "@/components/ui";
import { listMembers } from "@/lib/auth/directory";
import { requireTeam } from "@/lib/dal";

export const metadata = { title: "Settings" };

export default async function SettingsPage({ params }: PageProps<"/[team]/settings">) {
  const { team: slug } = await params;
  const { team, admin, viewer } = await requireTeam(slug);
  const members = await listMembers(team.orgId);
  return (
    <div className="page" style={{ maxWidth: 880 }}>
      <h1 className="h1">Settings</h1>

      <section className="card" aria-labelledby="general-h" style={{ padding: 20, display: "flex", flexDirection: "column", gap: 8 }}>
        <h2 id="general-h" className="h2">
          Team
        </h2>
        <dl style={{ margin: 0 }}>
          {[
            ["Name", team.name],
            ["URL", `app.quodci.com/${team.slug}`],
            ["Plan", team.plan],
            ["Your role", admin ? "Admin" : "Member"],
            ["WorkOS organization", team.orgId],
          ].map(([k, v]) => (
            <div key={k} className="kv">
              <dt>{k}</dt>
              <dd style={{ margin: 0 }}>{v}</dd>
            </div>
          ))}
        </dl>
      </section>

      <section className="card" aria-labelledby="members-h" style={{ overflow: "hidden" }}>
        <div className="card-head">
          <h2 id="members-h" className="h2">
            Members
          </h2>
          <span className="faint" style={{ fontSize: 13 }}>
            Admins change settings. Members see everything.
          </span>
        </div>
        <ul style={{ listStyle: "none", margin: 0, padding: 0 }}>
          {members.map((m) => (
            <li key={m.id} className="trow" style={{ gridTemplateColumns: "minmax(0, 1fr) auto" }}>
              <span className="cell-main">
                <span style={{ fontWeight: 500 }}>
                  {m.name ?? m.email}
                  {m.email === viewer.email ? <span className="faint"> (you)</span> : null}
                </span>
                {m.name ? <span className="cell-sub">{m.email}</span> : null}
              </span>
              <span style={{ display: "flex", gap: 6, alignItems: "center", flexWrap: "wrap", justifyContent: "flex-end" }}>
                {m.directoryManaged ? <Chip>From directory</Chip> : null}
                {m.status === "invited" ? <Chip tone="amber">Invited</Chip> : null}
                <Chip tone={m.role === "admin" ? "blue" : "gray"}>{m.role === "admin" ? "Admin" : "Member"}</Chip>
              </span>
            </li>
          ))}
        </ul>
        {admin ? (
          <div style={{ padding: 20, borderTop: "1px solid var(--rule)" }}>
            <InviteForm team={team.slug} />
          </div>
        ) : null}
      </section>

      <section className="card" aria-labelledby="sso-h" style={{ padding: 20, display: "flex", flexDirection: "column", gap: 6 }}>
        <h2 id="sso-h" className="h2">
          SSO &amp; directory sync
        </h2>
        <p className="muted" style={{ fontSize: 13, lineHeight: 1.6 }}>
          Set these up yourself, or send the page to your IT admin: each button opens a guided setup (the WorkOS Admin Portal) for this team only. The link works for a few minutes.
        </p>
        <PortalButtons team={team.slug} canEdit={admin} />
      </section>
    </div>
  );
}

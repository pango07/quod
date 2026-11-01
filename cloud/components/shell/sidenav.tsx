import { api } from "@/lib/api";
import { requireTeam } from "@/lib/dal";
import { formatCount } from "@/lib/format";
import { Sk } from "@/components/ui";
import { NavLinks } from "./nav-links";

export async function Sidenav({ params }: { params: Promise<{ team: string }> }) {
  const { team: slug } = await params;
  const { team } = await requireTeam(slug);
  const health = await api().getTeamHealth(team.slug);
  return (
    <nav className="sidenav" aria-label="Main">
      <NavLinks team={team.slug} />
      <div className="spacer" />
      {health.repos > 0 ? (
        <div className="card side-card" style={{ padding: 12, display: "flex", flexDirection: "column", gap: 6 }}>
          <span style={{ fontSize: 13, fontWeight: 500 }}>
            {health.wrongReplays === 0 ? "0 wrong replays" : `${formatCount(health.wrongReplays)} wrong replays`}
          </span>
          <span style={{ fontSize: 12, color: "var(--muted)", lineHeight: 1.5 }}>
            Every replay is checked against its inputs and re-run at random.
          </span>
        </div>
      ) : null}
    </nav>
  );
}

export function SidenavSkeleton() {
  return (
    <nav className="sidenav" aria-label="Main" aria-busy="true">
      {Array.from({ length: 7 }, (_, i) => (
        <div key={i} style={{ padding: "9px 10px" }}>
          <Sk w={i % 2 ? 96 : 120} h={14} />
        </div>
      ))}
    </nav>
  );
}

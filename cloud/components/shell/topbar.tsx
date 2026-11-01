import { api } from "@/lib/api";
import { requireTeam } from "@/lib/dal";
import { Logo, Sk } from "@/components/ui";
import { LiveBadge } from "@/components/live/parts";
import { CommandMenu } from "./command-menu";
import { Crumbs } from "./crumbs";
import { TeamSwitcher } from "./team-switcher";
import { UserMenu } from "./user-menu";

export async function Topbar({ params }: { params: Promise<{ team: string }> }) {
  const { team: slug } = await params;
  const { viewer, team, teams } = await requireTeam(slug);
  const index = await api().getSearchIndex(team.slug);
  return (
    <header className="topbar">
      <div className="crumbs">
        <a href={`/${team.slug}`} aria-label="Quod Cloud home" style={{ display: "flex" }}>
          <Logo />
        </a>
        <span className="hide-sm" style={{ fontWeight: 600, fontSize: 15 }}>
          Quod
        </span>
        <span className="sep hide-sm" aria-hidden="true">
          /
        </span>
        <TeamSwitcher current={team.slug} teams={teams.map((t) => ({ slug: t.slug, name: t.name, plan: t.plan, role: t.role }))} />
        <Crumbs team={team.slug} />
      </div>
      <div className="top-right">
        {viewer.demo ? (
          <span className="demo-chip" title="WorkOS is not configured, so you are signed in as a local demo user. Data is sample data.">
            <span className="long" aria-hidden="true">
              Demo session
            </span>
            <span className="short" aria-hidden="true">
              Demo
            </span>
            <span className="sr-only">Demo session: WorkOS is not configured, so you are signed in as a local demo user and the data is sample data.</span>
          </span>
        ) : null}
        <LiveBadge />
        <CommandMenu team={team.slug} index={index} />
        <a className="docs-link" href="https://www.quodci.com/under-the-hood">
          Docs
        </a>
        <UserMenu name={viewer.name} email={viewer.email} avatarUrl={viewer.avatarUrl} team={team.slug} />
      </div>
    </header>
  );
}

export function TopbarSkeleton() {
  return (
    <header className="topbar" aria-busy="true">
      <div className="crumbs">
        <Logo />
        <span style={{ fontWeight: 600, fontSize: 15 }}>Quod</span>
        <span className="sep">/</span>
        <Sk w={110} h={18} />
      </div>
      <div className="top-right">
        <Sk w={30} h={30} r={15} />
      </div>
    </header>
  );
}

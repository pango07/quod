import { MinutesChart } from "@/components/charts";
import { RangeToggle } from "@/components/range-toggle";
import { Stat } from "@/components/ui";
import { api } from "@/lib/api";
import { requireTeam } from "@/lib/dal";
import { formatCount, formatUsd } from "@/lib/format";
import { parseRange, RANGE_FROM } from "@/lib/params";

export const metadata = { title: "Usage" };

export default async function UsagePage({ params, searchParams }: PageProps<"/[team]/usage">) {
  const [{ team: slug }, sp] = await Promise.all([params, searchParams]);
  const range = parseRange(sp.range);
  const { team } = await requireTeam(slug);
  const ov = await api().getOverview(team.slug, range);
  const t = ov.totals;
  const ran = ov.days.reduce((a, d) => a + d.ranMinutes, 0);
  return (
    <div className="page">
      <div className="row-between">
        <h1 className="h1">Usage</h1>
        <RangeToggle base={`/${team.slug}/usage`} current={range} />
      </div>
      <section aria-label="Usage totals" className="stats">
        <Stat label="CI minutes replayed" value={formatCount(t.savedMinutes)} note="not run, so not billed by your CI provider" />
        <Stat label="CI minutes ran" value={formatCount(ran)} note="steps that had to run" />
        <Stat label="Est. runner cost saved" value={formatUsd(t.costSavedUsd)} note={`at $${t.usdPerMinute} per Linux minute`} />
        <Stat label="Plan" value={team.plan} note={team.plan === "Hobby" ? "Free for personal projects" : "Billed per team"} />
      </section>
      <section className="card" aria-labelledby="usage-h" style={{ padding: 20, display: "flex", flexDirection: "column", gap: 16 }}>
        <div className="row-between">
          <h2 id="usage-h" className="h2">
            CI minutes per day
          </h2>
          <div className="legend" aria-hidden="true">
            <span>
              <span className="swatch" style={{ background: "var(--blue)" }} />
              Replayed (saved)
            </span>
            <span>
              <span className="swatch" style={{ background: "var(--ran)" }} />
              Ran
            </span>
          </div>
        </div>
        {ov.days.length ? <MinutesChart days={ov.days} from={RANGE_FROM[range]} /> : <p className="muted">No CI minutes yet.</p>}
      </section>
    </div>
  );
}

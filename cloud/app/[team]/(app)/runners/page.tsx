import { unstable_rethrow } from "next/navigation";
import Link from "next/link";
import { ciState } from "@/components/github-ci";
import { RunnersView } from "@/components/runners-view";
import { SectionUnavailable } from "@/components/section";
import { api } from "@/lib/api";
import { section } from "@/lib/api/section";
import type { GitHubLink } from "@/lib/api/types";
import { requireTeam } from "@/lib/dal";
import { teamRunners, type SeenRunner } from "@/lib/github/pages";
import { teamGitHub } from "@/lib/github/team-data";

export const metadata = { title: "Runners" };

export default async function RunnersPage({ params }: PageProps<"/[team]/runners">) {
  const { team: slug } = await params;
  const { team } = await requireTeam(slug);
  const [quodRunners, gh] = await Promise.all([section(api().listRunners(team.slug)), teamGitHub(team.orgId)]);
  const quod = quodRunners.ok ? quodRunners.value : [];
  const now = new Date();
  let link: GitHubLink | undefined = gh.link;
  let seen: SeenRunner[] | null = null;
  if (gh.source) {
    try {
      seen = await teamRunners(gh.source.teamId, gh.source.db, { now });
    } catch (e) {
      unstable_rethrow(e);
      link = { state: "unavailable" };
    }
  }
  return (
    <div className="page">
      <div className="row-between">
        <h1 className="h1">Runners</h1>
        {gh.sample ? (
          <Link className="btn solid" href={`/${team.slug}/connect?host=own`} style={{ fontSize: 14 }}>
            Add runner
          </Link>
        ) : null}
      </div>
      {quodRunners.ok ? null : <SectionUnavailable s={quodRunners} what="Quod's runners" />}
      <RunnersView team={team.slug} asOf={now.toISOString()} state={gh.sample ? "sample" : ciState(link, !!seen?.length)} quod={quod} seen={seen} />
    </div>
  );
}

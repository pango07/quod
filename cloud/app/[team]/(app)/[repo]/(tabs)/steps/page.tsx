import { notFound } from "next/navigation";
import { StepsTable } from "@/components/steps-table";
import { SectionUnavailable } from "@/components/section";
import { api } from "@/lib/api";
import { section } from "@/lib/api/section";
import { requireTeam } from "@/lib/dal";
import { parseStepFilter } from "@/lib/params";
import { plainUi } from "@/lib/ui";
import { PlainRepoSteps } from "@/components/trust-plain-pages";

export const metadata = { title: "Steps" };

export default async function RepoStepsPage({ params, searchParams }: PageProps<"/[team]/[repo]/steps">) {
  const [{ team: slug, repo }, sp] = await Promise.all([params, searchParams]);
  const { team } = await requireTeam(slug);
  const found = await section(api().listSteps(team.slug, { repo }));
  if (!found.ok) return <SectionUnavailable s={found} what="Steps" page />;
  const steps = found.value;
  if (steps.length === 0) {
    const repoFound = await section(api().getRepo(team.slug, repo));
    if (repoFound.ok && !repoFound.value) notFound();
  }
  if (await plainUi(sp)) return <PlainRepoSteps team={team} repo={repo} steps={steps} />;
  return (
    <div className="page">
      <h1 className="sr-only">Steps of {repo}</h1>
      <StepsTable team={team.slug} steps={steps} initial={parseStepFilter(sp.state)} />
    </div>
  );
}

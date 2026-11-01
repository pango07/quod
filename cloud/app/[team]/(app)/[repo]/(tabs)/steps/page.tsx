import { notFound } from "next/navigation";
import { StepsTable } from "@/components/steps-table";
import { api } from "@/lib/api";
import { requireTeam } from "@/lib/dal";
import { parseStepFilter } from "@/lib/params";

export const metadata = { title: "Steps" };

export default async function RepoStepsPage({ params, searchParams }: PageProps<"/[team]/[repo]/steps">) {
  const [{ team: slug, repo }, sp] = await Promise.all([params, searchParams]);
  const { team } = await requireTeam(slug);
  const steps = await api().listSteps(team.slug, { repo });
  if (steps.length === 0 && !(await api().getRepo(team.slug, repo))) notFound();
  return (
    <div className="page">
      <h1 className="sr-only">Steps of {repo}</h1>
      <StepsTable team={team.slug} steps={steps} initial={parseStepFilter(sp.state)} />
    </div>
  );
}

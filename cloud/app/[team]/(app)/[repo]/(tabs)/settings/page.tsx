import Link from "next/link";
import { notFound } from "next/navigation";
import { SectionUnavailable } from "@/components/section";
import { api } from "@/lib/api";
import { section } from "@/lib/api/section";
import { requireTeam } from "@/lib/dal";

export const metadata = { title: "Repository settings" };

export default async function RepoSettingsPage({ params }: PageProps<"/[team]/[repo]/settings">) {
  const { team: slug, repo: repoSlug } = await params;
  const { team } = await requireTeam(slug);
  const found = await section(api().getRepo(team.slug, repoSlug));
  if (!found.ok) return <SectionUnavailable s={found} what="This repository" />;
  const repo = found.value;
  if (!repo) notFound();
  const s = repo.summary;
  return (
    <div className="page">
      <h1 className="h1" style={{ fontSize: 22 }}>
        Settings for {s.fullName}
      </h1>
      <section className="card" aria-label="Connection" style={{ padding: 20 }}>
        <dl style={{ margin: 0 }}>
          {(s.hasQuodData === false
            ? [
                ["Repository", s.fullName],
                ["CI data", "From GitHub (Quod CI App). Quod's settings appear once Quod runs in this repo."],
              ]
            : [
                ["Repository", s.fullName],
                ["Default branch", repo.defaultBranch],
                ["Runners", s.runner === "self-hosted" ? "Your own runners" : "GitHub-hosted, via quodci/setup"],
                ["Language", s.language || "Not known yet"],
              ]
          ).map(([k, v], i) => (
            <div key={k} className="kv" style={i === 0 ? { borderTop: 0, paddingTop: 0 } : undefined}>
              <dt>{k}</dt>
              <dd style={{ margin: 0 }}>{v}</dd>
            </div>
          ))}
        </dl>
      </section>
      <section className="card" aria-labelledby="proofs-h" style={{ padding: 20, display: "flex", flexDirection: "column", gap: 10 }}>
        <h2 id="proofs-h" className="h2">
          Trust rules
        </h2>
        <p className="muted" style={{ fontSize: 13, lineHeight: 1.6 }}>
          A step replays after 2 identical runs from the same inputs, and about 5% of replays are re-run at random as spot checks. To ask for more runs on one step, open it from{" "}
          <Link href={`/${team.slug}/${s.slug}/steps`} style={{ textDecoration: "underline" }}>
            Steps
          </Link>
          .
        </p>
      </section>
    </div>
  );
}

import { ProtectionWarning, protectionConcern } from "@/components/section";
import { api } from "@/lib/api";
import { requireTeam } from "@/lib/dal";

/**
 * The team-wide writer-trust warning, on every team page while it holds
 * (DECISIONS.md 2026-11-02): Quod has data for a repo whose default branch is
 * not protected on GitHub, or whose protection the App cannot read.
 */
export async function ProtectionBanner({ params }: { params: Promise<{ team: string }> }) {
  const { team: slug } = await params;
  const { team } = await requireTeam(slug);
  const concern = protectionConcern(await api().listRepos(team.slug));
  if (!concern) return null;
  return (
    <div style={{ marginBottom: 16 }}>
      <ProtectionWarning team={team.slug} state={concern.state} repos={concern.repos} branch={concern.repos.length === 1 ? concern.repos[0].protection?.branch : null} />
    </div>
  );
}

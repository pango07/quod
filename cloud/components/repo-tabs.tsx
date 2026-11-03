import { TabLinks } from "./tab-links";

export async function RepoTabs({ params }: { params: Promise<{ team: string; repo: string }> }) {
  const { team, repo } = await params;
  const base = `/${team}/${repo}`;
  return (
    <TabLinks
      label="Repository"
      tabs={[
        { href: base, label: "Overview" },
        { href: `${base}/runs`, label: "Runs" },
        { href: `${base}/steps`, label: "Steps" },
        { href: `${base}/tests`, label: "Tests" },
        { href: `${base}/settings`, label: "Settings" },
      ]}
    />
  );
}

import { Suspense } from "react";
import { RepoTabs } from "@/components/repo-tabs";

export default function RepoTabsLayout({ children, params }: LayoutProps<"/[team]/[repo]">) {
  return (
    <>
      <Suspense fallback={<div className="tabs" style={{ height: 46 }} />}>
        <RepoTabs params={params} />
      </Suspense>
      {children}
    </>
  );
}

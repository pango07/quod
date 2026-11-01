import { Suspense } from "react";
import { Sidenav, SidenavSkeleton } from "@/components/shell/sidenav";
import { Topbar, TopbarSkeleton } from "@/components/shell/topbar";

// The dashboard frame (header, sidebar). The parts that need the session or
// the team stream in behind their own boundaries; the frame itself is static.
export default function AppLayout({ children, params }: LayoutProps<"/[team]">) {
  return (
    <div className="app">
      <a href="#main" className="skip">
        Skip to content
      </a>
      <Suspense fallback={<TopbarSkeleton />}>
        <Topbar params={params} />
      </Suspense>
      <div className="frame">
        <Suspense fallback={<SidenavSkeleton />}>
          <Sidenav params={params} />
        </Suspense>
        <main id="main" className="content" tabIndex={-1}>
          {children}
        </main>
      </div>
    </div>
  );
}

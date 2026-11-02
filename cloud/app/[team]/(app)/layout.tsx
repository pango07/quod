import { Suspense } from "react";
import { LiveProvider } from "@/components/live/provider";
import { ProtectionBanner } from "@/components/shell/protection-banner";
import { Sidenav, SidenavSkeleton } from "@/components/shell/sidenav";
import { Topbar, TopbarSkeleton } from "@/components/shell/topbar";

// The dashboard frame (header, sidebar). The parts that need the session or
// the team stream in behind their own boundaries; the frame itself is static.
// LiveProvider holds the tab's one connection to /[team]/events.
export default function AppLayout({ children, params }: LayoutProps<"/[team]">) {
  return (
    <LiveProvider>
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
          <Suspense fallback={null}>
            <ProtectionBanner params={params} />
          </Suspense>
          {children}
        </main>
      </div>
    </div>
    </LiveProvider>
  );
}

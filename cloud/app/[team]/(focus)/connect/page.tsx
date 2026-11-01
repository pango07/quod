import Link from "next/link";
import { Suspense } from "react";
import { ConnectFlow } from "@/components/connect-flow";
import { Logo, Sk } from "@/components/ui";
import { api } from "@/lib/api";
import type { Host, Store } from "@/lib/api/types";
import { requireTeam } from "@/lib/dal";
import { one } from "@/lib/params";

export const metadata = { title: "Connect a repository" };

// A focused page without the sidebar, as in Connect.dc.html.
export default function ConnectPage(props: PageProps<"/[team]/connect">) {
  return (
    <div className="app">
      <header className="topbar">
        <Suspense
          fallback={
            <span className="brand">
              <Logo />
              Quod
            </span>
          }
        >
          <HomeLink params={props.params} />
        </Suspense>
        <Suspense fallback={null}>
          <BackLink params={props.params} />
        </Suspense>
      </header>
      <main id="main" className="focus-page">
        <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
          <h1 style={{ fontSize: 32, fontWeight: 600, letterSpacing: "-0.03em" }}>Connect a repository</h1>
          <p className="muted" style={{ fontSize: 15 }}>
            About two minutes. Your workflows stay the same.
          </p>
        </div>
        <Suspense fallback={<FlowSkeleton />}>
          <Flow {...props} />
        </Suspense>
      </main>
    </div>
  );
}

async function HomeLink({ params }: { params: Promise<{ team: string }> }) {
  const { team } = await params;
  return (
    <Link href={`/${team}`} className="brand">
      <Logo />
      Quod
    </Link>
  );
}

async function BackLink({ params }: { params: Promise<{ team: string }> }) {
  const { team } = await params;
  return (
    <Link href={`/${team}`} className="link-muted">
      Back to dashboard
    </Link>
  );
}

async function Flow({ params, searchParams }: PageProps<"/[team]/connect">) {
  const [{ team: slug }, sp] = await Promise.all([params, searchParams]);
  const { team } = await requireTeam(slug);
  const setup = await api().getConnectSetup(team.slug);
  const store = (one(sp.store) === "self" ? "self" : one(sp.store) === "cloud" ? "cloud" : (setup.saved?.store ?? "cloud")) as Store;
  const host = (one(sp.host) === "own" ? "own" : one(sp.host) === "github" ? "github" : (setup.saved?.host ?? "github")) as Host;
  return <ConnectFlow team={team.slug} setup={setup} initial={{ store, host }} />;
}

function FlowSkeleton() {
  return (
    <div aria-busy="true" style={{ display: "flex", flexDirection: "column", gap: 28 }}>
      {[0, 1].map((i) => (
        <div key={i} style={{ display: "flex", flexDirection: "column", gap: 12 }}>
          <Sk w={260} h={16} />
          <div style={{ display: "flex", gap: 12, flexWrap: "wrap" }}>
            <Sk w="48%" h={96} r={12} />
            <Sk w="48%" h={96} r={12} />
          </div>
        </div>
      ))}
      <Sk h={220} r={12} />
    </div>
  );
}

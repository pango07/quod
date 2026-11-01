import { connection } from "next/server";
import { Suspense } from "react";
import { StepExpired } from "@/components/auth/expired";
import { OrganizationPicker, StartOver } from "@/components/auth/flows";
import { AuthShell, AuthSkeleton, DemoNote } from "@/components/auth/shell";
import { organizationsOf, readPending } from "@/lib/auth/flow";
import { isDemo } from "@/lib/auth/pages";

export const metadata = { title: "Choose a team" };

export default function OrganizationPage() {
  return (
    <Suspense fallback={<AuthSkeleton />}>
      <Organization />
    </Suspense>
  );
}

const DEMO_ORGS = [
  { id: "org_demo00000quodci", name: "quodci" },
  { id: "org_demo0000jvaught", name: "jvaught" },
];

// WorkOS answered "organization_selection_required": this account is in several organizations.
// The cookie holds only the user id; the list comes from WorkOS here, so it is always current.
async function Organization() {
  await connection(); // reads the sealed pending cookie: per request
  let organizations: { id: string; name: string }[] = [];
  if (isDemo()) {
    organizations = DEMO_ORGS;
  } else {
    const pending = await readPending();
    if (!pending?.userId || pending.step !== "select-org") return <StepExpired />;
    try {
      organizations = await organizationsOf(pending.userId);
    } catch (e) {
      console.warn("[quod-cloud] could not list organizations", { status: (e as { status?: number })?.status });
    }
  }
  if (!organizations.length) return <StepExpired />;
  return (
    <AuthShell
      title="Choose a team"
      lede="Your account is in more than one team. Pick the one to open; you can switch later."
      footer={<StartOver />}
      aside={isDemo() ? <DemoNote /> : null}
    >
      <OrganizationPicker organizations={organizations} />
    </AuthShell>
  );
}

import { connection } from "next/server";
import { Suspense } from "react";
import { StepExpired } from "@/components/auth/expired";
import { OrganizationPicker, StartOver } from "@/components/auth/flows";
import { AuthShell, AuthSkeleton, DemoNote } from "@/components/auth/shell";
import { readPending } from "@/lib/auth/flow";
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
async function Organization() {
  await connection(); // reads the sealed pending cookie: per request
  const pending = isDemo() ? { organizations: DEMO_ORGS } : await readPending();
  if (!pending?.organizations?.length || (!isDemo() && "step" in pending && pending.step !== "select-org")) return <StepExpired />;
  return (
    <AuthShell
      title="Choose a team"
      lede="Your account is in more than one team. Pick the one to open; you can switch later."
      footer={<StartOver />}
      aside={isDemo() ? <DemoNote /> : null}
    >
      <OrganizationPicker organizations={pending.organizations} />
    </AuthShell>
  );
}

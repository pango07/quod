"use client";

import { ErrorView } from "@/components/error-view";

// Inside the dashboard frame: the header and sidebar stay, only the page shows the error.
export default function Error({ error, retry }: { error: unknown; retry: () => void }) {
  return <ErrorView error={error} retry={retry} inShell />;
}

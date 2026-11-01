"use client";

import { ErrorView } from "@/components/error-view";

export default function Error({ error, retry }: { error: unknown; retry: () => void }) {
  return (
    <main id="main">
      <ErrorView error={error} retry={retry} />
    </main>
  );
}

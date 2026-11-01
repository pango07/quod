"use client";

import { useActionState } from "react";
import { disconnectGitHub, type DisconnectResult } from "@/lib/github/actions";

/** Team settings: Disconnect one GitHub installation from this team (admins only; the server checks). */
export function DisconnectGitHub({ team, installationId, account }: { team: string; installationId: number; account: string }) {
  const [state, action, pending] = useActionState(disconnectGitHub.bind(null, team), null as DisconnectResult);
  return (
    <form action={action} style={{ display: "flex", flexDirection: "column", gap: 6, alignItems: "flex-end" }}>
      <input type="hidden" name="installation" value={installationId} />
      <button
        className="btn sm"
        type="submit"
        disabled={pending}
        onClick={(e) => {
          if (!confirm(`Disconnect ${account}? This team stops seeing its CI jobs. The App stays installed on GitHub.`)) e.preventDefault();
        }}
      >
        {pending ? "Disconnecting…" : "Disconnect"}
      </button>
      {state ? (
        <p role="status" className={`toast ${state.ok ? "ok" : "err"}`}>
          {state.message}
        </p>
      ) : null}
    </form>
  );
}

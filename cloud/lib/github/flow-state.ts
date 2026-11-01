import { timingSafeEqual } from "node:crypto";
import { sealData, unsealData } from "iron-session";

// The install / OAuth state for /github/installed (review M1), pure so it is
// tested. A GitHub code is consumed only when it returns with a state this
// browser got from Quod for the signed-in user. Anything else is dropped and
// the user goes through GitHub's OAuth with a fresh state of ours.

export const FLOW_STATE_TTL = 900; // seconds

export interface GitHubFlowState {
  purpose: "quod-gh-state";
  state: string;
  /** The WorkOS user the flow was started for. */
  userId: string;
  /** Set when /github/installed started the OAuth flow for a known installation. */
  installationId?: number;
}

export function sealFlowState(v: Omit<GitHubFlowState, "purpose">, password: string): Promise<string> {
  return sealData({ ...v, purpose: "quod-gh-state" } satisfies GitHubFlowState, { password, ttl: FLOW_STATE_TTL });
}

/** The sealed state if `returned` equals it and it was started for `userId`; else null. */
export async function checkFlowState(raw: string | undefined, returned: string | null, userId: string, password: string): Promise<GitHubFlowState | null> {
  if (!raw || !returned) return null;
  try {
    const v = await unsealData<GitHubFlowState>(raw, { password, ttl: FLOW_STATE_TTL });
    if (v?.purpose !== "quod-gh-state" || typeof v.state !== "string" || v.userId !== userId) return null;
    if (v.state.length !== returned.length || !timingSafeEqual(Buffer.from(v.state), Buffer.from(returned))) return null;
    return v;
  } catch {
    return null;
  }
}

export type InstalledPlan =
  | { kind: "requested" }
  | { kind: "no-installation" }
  /** A code that came back with our state: exchange it and verify. */
  | { kind: "verify"; code: string; installationId: number }
  /** Ask GitHub who this is, with a fresh state. Any code that came without our state is dropped. */
  | { kind: "authorize"; installationId: number; droppedCode: boolean }
  /** No code: done if this team already has the installation, else authorize. */
  | { kind: "check-linked"; installationId: number };

const ID = /^\d{1,15}$/;

/** What /github/installed does with GitHub's query string, given the state check's result. */
export function planInstalled(params: URLSearchParams, trusted: GitHubFlowState | null): InstalledPlan {
  if (params.get("setup_action") === "request") return { kind: "requested" };
  const fromQuery = ID.test(params.get("installation_id") ?? "") ? Number(params.get("installation_id")) : null;
  const installationId = trusted?.installationId ?? fromQuery;
  if (!installationId) return { kind: "no-installation" };
  const code = params.get("code");
  if (code && trusted) return { kind: "verify", code, installationId };
  if (code) return { kind: "authorize", installationId, droppedCode: true };
  return { kind: "check-linked", installationId };
}

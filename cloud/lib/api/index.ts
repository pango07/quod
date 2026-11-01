import "server-only";
import { accessToken } from "@/lib/auth/session";
import { FixtureCloudApi, newFixtureState, type FixtureState } from "./fixture";
import { HttpCloudApi } from "./http";
import { resolveApiMode } from "./mode";
import type { CloudApi } from "./types";

export type * from "./types";

// The fixture's changes (proofs needed, onboarding choice) live in one object
// per process, kept across dev reloads, so they stick until restart.
const g = globalThis as unknown as { __quodFixtureState?: FixtureState };

/** The CloudApi for this request. Pages and actions read data only through this. */
export function api(): CloudApi {
  const mode = resolveApiMode(process.env);
  if (mode.kind === "misconfigured") throw new Error(`Quod Cloud data source is not configured. ${mode.problem}`);
  if (mode.kind === "http") return new HttpCloudApi(mode.url, accessToken);
  return new FixtureCloudApi((g.__quodFixtureState ??= newFixtureState()));
}

export function usingSampleData(): boolean {
  return resolveApiMode(process.env).kind === "fixture";
}

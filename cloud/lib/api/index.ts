import "server-only";
import { notFound, redirect } from "next/navigation";
import { accessToken } from "@/lib/auth/session";
import { FixtureCloudApi, newFixtureState, type FixtureState } from "./fixture";
import { HttpCloudApi } from "./http";
import { resolveApiMode } from "./mode";
import { ApiForbiddenError, ApiInvalidRequestError, type CloudApi } from "./types";

export type * from "./types";

// The fixture's changes (proofs needed, onboarding choice) live in one object
// per process, kept across dev reloads, so they stick until restart.
const g = globalThis as unknown as { __quodFixtureState?: FixtureState };

/** The CloudApi for this request. Pages and actions read data only through this. */
export function api(): CloudApi {
  const mode = resolveApiMode(process.env);
  if (mode.kind === "misconfigured") throw new Error(`Quod Cloud data source is not configured. ${mode.problem}`);
  const inner = mode.kind === "http" ? new HttpCloudApi(mode.url, accessToken) : new FixtureCloudApi((g.__quodFixtureState ??= newFixtureState()));
  return withRefusals(inner);
}

/**
 * Turns the backend's refusals into navigation: 401 signs in again, 403 and
 * invalid ids are a 404 (so a team's existence does not leak).
 */
function withRefusals(inner: CloudApi): CloudApi {
  return new Proxy(inner, {
    get(target, prop, receiver) {
      const v = Reflect.get(target, prop, receiver);
      if (typeof v !== "function") return v;
      return async (...args: unknown[]) => {
        try {
          return await v.apply(target, args);
        } catch (e) {
          if (e instanceof ApiForbiddenError && e.status === 401) redirect("/sign-in");
          if (e instanceof ApiForbiddenError) notFound();
          if (e instanceof ApiInvalidRequestError) notFound();
          throw e;
        }
      };
    },
  });
}

export function usingSampleData(): boolean {
  return resolveApiMode(process.env).kind === "fixture";
}

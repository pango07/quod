import { unstable_rethrow } from "next/navigation";
import { ApiNotAvailableError, ApiUnavailableError } from "./types";

// A page section that reads Quod's data on its own: quod-server's "not
// available yet" (501) and "unavailable" (down, 5xx, a protocol mismatch)
// become a value the section renders as a card (components/section.tsx), so
// one failing section never takes the page down. Sign-in redirects and
// not-found still navigate.

export type Section<T> = { ok: true; value: T } | { ok: false; kind: "not-available" | "unavailable"; message: string };

export async function section<T>(p: Promise<T>): Promise<Section<T>> {
  try {
    return { ok: true, value: await p };
  } catch (e) {
    unstable_rethrow(e);
    if (e instanceof ApiNotAvailableError) return { ok: false, kind: "not-available", message: e.detail };
    if (e instanceof ApiUnavailableError) return { ok: false, kind: "unavailable", message: e.message };
    throw e;
  }
}

/** For server actions: quod-server's refusals as the action's message. */
export function actionFailure(e: unknown): { ok: false; message: string } | null {
  if (e instanceof ApiNotAvailableError) return { ok: false, message: "Not available yet: quod-server does not support this yet." };
  if (e instanceof ApiUnavailableError) return { ok: false, message: "Quod data is unavailable right now. Try again in a minute." };
  return null;
}

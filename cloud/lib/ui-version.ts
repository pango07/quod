// Which wording the trust pages use: "classic" (the pages as they were) or
// "plain" (cloud/trust-plain: grouped steps, states in words). A switch so
// the owner can compare the two before choosing, and so going back is one
// setting, not a revert:
//
//   ?ui=classic or ?ui=plain   this page, and remembered in a cookie
//                              (quod_ui, set by proxy.ts) for the next ones
//   NEXT_PUBLIC_TRUST_V2_UI=0  classic for everyone (build time);
//                              unset or anything else: plain
//
// Pure; lib/ui.ts reads the request.

export type UiVersion = "classic" | "plain";
export const UI_COOKIE = "quod_ui";

export function parseUi(v: unknown): UiVersion | null {
  const s = Array.isArray(v) ? v[0] : v;
  return s === "classic" || s === "plain" ? s : s === "new" ? "plain" : null;
}

/** The query wins, then the cookie, then the build's default. */
export function pickUi(query: unknown, cookie: unknown, env: string | undefined = process.env.NEXT_PUBLIC_TRUST_V2_UI): UiVersion {
  return parseUi(query) ?? parseUi(cookie) ?? (env === "0" ? "classic" : "plain");
}

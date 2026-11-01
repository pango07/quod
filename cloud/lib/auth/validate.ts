// Input checks for the sign-in forms. Pure, so they are unit-tested and the
// same rules run in every server action.

/** Where to go after signing in: a same-origin path only, else "/". Never an open redirect. */
export function sanitizeReturnTo(value: unknown): string {
  if (typeof value !== "string" || value.length === 0 || value.length > 1024) return "/";
  // Only absolute paths; "//host" and "/\host" are protocol-relative in browsers.
  if (!value.startsWith("/") || value.startsWith("//") || value.startsWith("/\\")) return "/";
  // No control characters or backslashes anywhere (browsers treat "\" like "/").
  if (/[\u0000-\u001f\u007f\\]/.test(value)) return "/";
  let url: URL;
  try {
    url = new URL(value, "https://quod.invalid");
  } catch {
    return "/";
  }
  if (url.origin !== "https://quod.invalid") return "/";
  const path = url.pathname + url.search + url.hash;
  // Check the result, not just the input: dot segments can normalize "/..//evil.com" to "//evil.com".
  if (!/^\/(?![/\\])/.test(path) || /[\u0000-\u001f\u007f\\]/.test(path)) return "/";
  // Sending someone back to a sign-in page after signing in would loop.
  if (/^\/(sign-in|sign-up|verify-email|reset-password|invite|callback|signed-out)(\/|$|\?|#)/.test(path)) return "/";
  return path;
}

export const MIN_PASSWORD = 10;
export const MAX_PASSWORD = 256;

export type Check<T> = { ok: true; value: T } | { ok: false; error: string };

export function checkEmail(value: unknown): Check<string> {
  const email = typeof value === "string" ? value.trim().toLowerCase() : "";
  if (email.length === 0) return { ok: false, error: "Enter your email address." };
  if (email.length > 254 || !/^[^\s@]+@[^\s@.]+(\.[^\s@.]+)+$/.test(email)) return { ok: false, error: "Enter an email address like you@company.com." };
  return { ok: true, value: email };
}

/** A password being chosen (sign-up, reset). Signing in only checks that one was given. */
export function checkNewPassword(value: unknown): Check<string> {
  const password = typeof value === "string" ? value : "";
  if (password.length < MIN_PASSWORD) return { ok: false, error: `Use at least ${MIN_PASSWORD} characters.` };
  if (password.length > MAX_PASSWORD) return { ok: false, error: `Use at most ${MAX_PASSWORD} characters.` };
  return { ok: true, value: password };
}

export function checkPassword(value: unknown): Check<string> {
  const password = typeof value === "string" ? value : "";
  if (password.length === 0) return { ok: false, error: "Enter your password." };
  if (password.length > MAX_PASSWORD) return { ok: false, error: "That password is too long." };
  return { ok: true, value: password };
}

/** A six-digit code from an email or an authenticator app. Spaces and dashes from pasting are dropped. */
export function checkCode(value: unknown): Check<string> {
  const code = typeof value === "string" ? value.replace(/[\s-]/g, "") : "";
  if (!/^\d{6}$/.test(code)) return { ok: false, error: "Enter the 6-digit code." };
  return { ok: true, value: code };
}

export function checkName(value: unknown, label: string): Check<string> {
  const name = typeof value === "string" ? value.trim().replace(/\s+/g, " ") : "";
  if (name.length === 0) return { ok: false, error: `Enter your ${label}.` };
  if (name.length > 100 || /[\u0000-\u001f\u007f<>]/.test(name)) return { ok: false, error: `That ${label} can't be used.` };
  return { ok: true, value: name };
}

/** Opaque tokens from emailed links (password reset, invitation). */
export function checkToken(value: unknown): Check<string> {
  const token = typeof value === "string" ? value.trim() : "";
  if (!/^[A-Za-z0-9._~-]{8,512}$/.test(token)) return { ok: false, error: "This link is not valid. Ask for a new one." };
  return { ok: true, value: token };
}

/** WorkOS ids: org_..., auth_factor_..., etc. */
export function checkId(value: unknown, prefix: string): Check<string> {
  const id = typeof value === "string" ? value : "";
  if (!new RegExp(`^${prefix}_[A-Za-z0-9]{10,64}$`).test(id)) return { ok: false, error: "Choose one of the options." };
  return { ok: true, value: id };
}

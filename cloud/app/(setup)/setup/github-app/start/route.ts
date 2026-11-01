import { cookies } from "next/headers";
import { connection, NextResponse, type NextRequest } from "next/server";
import { getViewer } from "@/lib/auth/session";
import { appOrigin, isOperator } from "@/lib/github/env";
import { esc, htmlPage, NO_STORE_HEADERS } from "@/lib/github/html";
import { buildManifest, checkAppName, GITHUB_NEW_APP_URL, MANIFEST_COOKIE, MANIFEST_TTL, newState, sealState } from "@/lib/github/manifest";
import { requestOrigin, sealPassword, secureCookies } from "@/lib/github/server";

// POST from /setup/github-app: makes a random state, seals it in an httpOnly
// cookie, and hands the browser a form that posts the manifest to GitHub.

export async function POST(request: NextRequest) {
  await connection();
  const viewer = await getViewer();
  if (!isOperator(viewer.email)) return new NextResponse("Not found", { status: 404 });

  const here = requestOrigin(request.headers);
  const back = (q: string) => NextResponse.redirect(new URL(`/setup/github-app?${q}`, here), 303);
  // Same-origin form posts only.
  if (request.headers.get("origin") !== here) return back("error=origin");

  const form = await request.formData();
  const name = checkAppName(form.get("name"));
  if (!name.ok) return back("error=name");

  const state = newState();
  const manifest = buildManifest(appOrigin(here), name.value);
  (await cookies()).set(MANIFEST_COOKIE, await sealState(state, sealPassword()), {
    httpOnly: true,
    secure: secureCookies(here),
    // GitHub sends the browser back with a top-level GET: lax is enough.
    sameSite: "lax",
    path: "/setup/github-app",
    maxAge: MANIFEST_TTL,
  });

  const action = `${GITHUB_NEW_APP_URL}?state=${encodeURIComponent(state)}`;
  const body = `
<h1>Opening GitHub…</h1>
<p>Creating “${esc(name.value)}”. If nothing happens, tap the button.</p>
<form id="f" method="post" action="${esc(action)}">
  <input type="hidden" name="manifest" value="${esc(JSON.stringify(manifest))}">
  <button type="submit">Continue to GitHub</button>
</form>`;
  return new NextResponse(htmlPage("Opening GitHub", body, `document.getElementById("f").submit()`), { status: 200, headers: NO_STORE_HEADERS });
}

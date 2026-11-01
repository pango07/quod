import { cookies } from "next/headers";
import { connection, NextResponse, type NextRequest } from "next/server";
import { getViewer } from "@/lib/auth/session";
import { convertManifest, GitHubApiError } from "@/lib/github/api";
import { isOperator } from "@/lib/github/env";
import { esc, htmlPage, NO_STORE_HEADERS } from "@/lib/github/html";
import { envFromConversion, MANIFEST_COOKIE, stateMatches } from "@/lib/github/manifest";
import { routeOrigin, sealPassword } from "@/lib/github/server";

// GitHub's redirect_url after the App is created: ?code=&state=. Checks the
// state against the sealed cookie, exchanges the code (single use), and shows
// the App's keys ONCE, for the operator to paste into Vercel. The keys are
// not logged, not stored, and the page is never cached; reloading it fails
// because GitHub's code is spent.

export async function GET(request: NextRequest) {
  await connection();
  const viewer = await getViewer();
  if (!isOperator(viewer.email)) return new NextResponse("Not found", { status: 404 });

  const here = routeOrigin(request);
  const back = (error: string) => NextResponse.redirect(new URL(`/setup/github-app?error=${error}`, here), 303);
  const jar = await cookies();
  const params = request.nextUrl.searchParams;
  const ok = await stateMatches(jar.get(MANIFEST_COOKIE)?.value, params.get("state"), sealPassword());
  jar.delete({ name: MANIFEST_COOKIE, path: "/setup/github-app" });
  if (!ok) return back("state");

  const code = params.get("code") ?? "";
  let app;
  try {
    app = await convertManifest(code);
  } catch (e) {
    console.warn("[quod-cloud] github app manifest conversion failed", { status: e instanceof GitHubApiError ? e.status : undefined });
    return back("exchange");
  }
  console.info("[quod-cloud] github app created", { appId: app.id, slug: app.slug });

  const vars = envFromConversion(app);
  const rows = vars
    .map(
      ([name, value, secret], i) => `
<section class="var">
  <div class="row"><span class="name">${esc(name)}</span><span class="tag">${secret ? "secret" : "not secret"}</span></div>
  <pre class="val" id="v${i}">${esc(value)}</pre>
  <button type="button" class="ghost" data-copy="v${i}">Copy ${esc(name)}</button>
</section>`,
    )
    .join("");
  const noSecret = app.webhook_secret
    ? ""
    : `<p class="warn">GitHub returned no webhook secret. Set one on the App's settings page on GitHub and put the same value in GITHUB_APP_WEBHOOK_SECRET; webhooks are refused without it.</p>`;
  const body = `
<h1>“${esc(app.name)}” is created</h1>
<p class="warn"><strong>Shown once.</strong> Quod does not keep these values. Copy each into Vercel now: project <strong>quod-cloud</strong>, Settings, Environment Variables, Production. Mark the secret ones Sensitive.</p>
${noSecret}
${rows}
<ol>
  <li>Paste all six into Vercel, then <strong>Redeploy</strong> production.</li>
  <li>Open <a href="${esc(app.html_url)}" rel="noreferrer">the App on GitHub</a> to check its settings.</li>
  <li>After the redeploy, install it from Quod: <a href="${esc(here)}/github/link">${esc(here.replace(/^https?:\/\//, ""))}/github/link</a>.</li>
</ol>
<p>Lost this page? Generate a new private key and client secret on the App's GitHub settings page instead.</p>`;
  // m6: leaving the page (or the browser freezing it into the back-forward cache) blanks the values,
  // so Back or a restored tab never shows them again.
  const blank = `addEventListener("pagehide",function(){document.querySelectorAll(".val").forEach(function(e){e.textContent=""});document.querySelectorAll("[data-copy]").forEach(function(b){b.disabled=true})});addEventListener("pageshow",function(e){if(e.persisted){document.querySelector("main").innerHTML="<h1>Values hidden</h1><p>They were shown once. On the App's GitHub settings page, generate a new private key and client secret if you need them.</p>"}});`;
  const script = blank + `document.querySelectorAll("[data-copy]").forEach(function(b){b.addEventListener("click",function(){var t=document.getElementById(b.getAttribute("data-copy")).textContent;navigator.clipboard.writeText(t).then(function(){var l=b.textContent;b.textContent="Copied";setTimeout(function(){b.textContent=l},1500)},function(){var r=document.createRange();r.selectNodeContents(document.getElementById(b.getAttribute("data-copy")));var s=getSelection();s.removeAllRanges();s.addRange(r)})})})`;
  return new NextResponse(htmlPage("GitHub App created", body, script), { status: 200, headers: NO_STORE_HEADERS });
}

import { headers } from "next/headers";
import { notFound } from "next/navigation";
import { connection } from "next/server";
import { AuthShell } from "@/components/auth/shell";
import { getViewer } from "@/lib/auth/session";
import { appOrigin, githubAppConfig, isOperator } from "@/lib/github/env";
import { buildManifest, checkAppName, DEFAULT_APP_NAME } from "@/lib/github/manifest";
import { requestOrigin } from "@/lib/github/server";
import { one } from "@/lib/params";

export const metadata = { title: "Create the GitHub App" };

const ERRORS: Record<string, string> = {
  name: "That name can't be used. Use up to 34 letters, digits, spaces, dots, dashes or underscores.",
  state: "That link from GitHub didn't match this browser, or it expired after 10 minutes. Tap Create again.",
  exchange: "GitHub didn't accept the code (it works once, for an hour). If the App was created, find it at github.com/settings/apps; otherwise tap Create again.",
  origin: "That request didn't come from this page. Tap Create again.",
};

// For the Quod operators (QUOD_OPERATOR_EMAILS) only: creates the Quod CI
// GitHub App through GitHub's App Manifest flow. Everyone else gets a 404.
export default async function GitHubAppSetup({ searchParams }: { searchParams: Promise<Record<string, string | string[] | undefined>> }) {
  await connection();
  const viewer = await getViewer();
  if (!isOperator(viewer.email)) notFound();

  const sp = await searchParams;
  const asked = checkAppName(one(sp.name));
  const name = asked.ok ? asked.value : DEFAULT_APP_NAME;
  const error = ERRORS[one(sp.error) ?? ""];
  const origin = appOrigin(requestOrigin(await headers()));
  const manifest = buildManifest(origin, name);
  const existing = githubAppConfig();

  return (
    <AuthShell
      title="Create the GitHub App"
      lede={
        <>
          One tap here, one on GitHub. The App is created under the GitHub account you are signed in to on github.com (for Quod: <strong>pango07</strong>).
        </>
      }
      footer={<span>Signed in as {viewer.email}</span>}
    >
      {existing.ok ? (
        <p className="notice" role="status">
          This deploy already has a GitHub App ({process.env.GITHUB_APP_SLUG}). Creating another makes a second, separate App.
        </p>
      ) : null}
      {error ? (
        <p className="setup-error" role="alert">
          {error}
        </p>
      ) : null}
      <form method="post" action="/setup/github-app/start" className="auth-form">
        <div className="auth-field">
          <label htmlFor="name">App name</label>
          <input id="name" name="name" className="input auth-input" defaultValue={name} maxLength={34} required autoComplete="off" autoCapitalize="words" />
          <p className="auth-hint">If GitHub says the name is taken, come back here, change it, and tap Create again.</p>
        </div>
        <button type="submit" className="btn solid lg auth-btn">
          Create GitHub App
        </button>
      </form>
      <ol className="setup-steps">
        <li>
          On GitHub, check the name and tap <strong>Create GitHub App</strong>.
        </li>
        <li>GitHub sends you back here with the App&apos;s keys, shown once.</li>
        <li>
          Copy each into Vercel (project quod-cloud, Settings, Environment Variables), then <strong>redeploy</strong>.
        </li>
      </ol>
      <details className="setup-details">
        <summary>What GitHub will be asked for</summary>
        <pre className="well">{JSON.stringify(manifest, null, 2)}</pre>
      </details>
    </AuthShell>
  );
}

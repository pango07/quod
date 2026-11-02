/**
 * The "Learn more" of the writer-trust warning (/[team]/trust#protected-branch).
 * The decision it explains: DECISIONS.md 2026-11-02.
 */
export function ProtectedBranchHelp({ bare }: { bare?: boolean } = {}) {
  const body = <ProtectedBranchBody />;
  if (bare) return <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>{body}</div>;
  return (
    <section id="protected-branch" className="card" aria-labelledby="protected-branch-h" style={{ padding: 20, display: "flex", flexDirection: "column", gap: 10, scrollMarginTop: 80 }}>
      <h2 id="protected-branch-h" className="h2">
        Who can record results others replay
      </h2>
      {body}
    </section>
  );
}

function ProtectedBranchBody() {
  return (
    <>
      <p className="muted" style={{ fontSize: 14, lineHeight: 1.6, maxWidth: "75ch" }}>
        Results Quod gives back to every CI job of a repository are recorded only by jobs on its default branch (a push, a schedule or a manual run there). If that branch is not
        protected on GitHub, anyone who can push to it can record a result that later jobs reuse. Quod still checks every replay against its inputs and re-runs some at random,
        but protecting the branch keeps the recording itself in reviewed hands.
      </p>
      <ul className="muted" style={{ fontSize: 14, lineHeight: 1.7, margin: 0, paddingLeft: 18, maxWidth: "75ch", listStyle: "disc", display: "flex", flexDirection: "column", gap: 6 }}>
        <li>
          Protect the default branch on GitHub with a branch protection rule or a ruleset (
          <a
            href="https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-protected-branches/about-protected-branches"
            rel="noreferrer"
            target="_blank"
            style={{ textDecoration: "underline" }}
          >
            GitHub&apos;s guide
          </a>
          ). The warning clears once Quod reads the change: within minutes when GitHub tells it, at most 6 hours otherwise.
        </li>
        <li>
          Or turn on <span className="mono">require_protected_ref</span> for the team or the repository: then only jobs on a protected branch record results, and Quod runs every
          step elsewhere.
        </li>
        <li>&ldquo;Protection unknown&rdquo; means the Quod CI GitHub App could not read it: it needs the Administration (read) permission, which an owner approves on GitHub.</li>
      </ul>
    </>
  );
}

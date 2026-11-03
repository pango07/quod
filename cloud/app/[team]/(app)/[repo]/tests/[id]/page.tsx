import Link from "next/link";
import { notFound, redirect } from "next/navigation";
import { TestRunRefresh } from "@/components/live/test-run-refresh";
import { SectionUnavailable } from "@/components/section";
import { Chip, Dot } from "@/components/ui";
import { api } from "@/lib/api";
import { section } from "@/lib/api/section";
import { requireTeam } from "@/lib/dal";
import { formatAgo, formatCount, shortSha } from "@/lib/format";
import { isSlug } from "@/lib/params";
import { exitLabel, formatSaved, formatSavedMs, kindLabel, KIND_EXPLAINED, ledgerSentence, SAVED_EXPLAINED, showSpotChecks, spotCheckSentence, testRunHeadline, testRunHref } from "@/lib/test-runs";

export const metadata = { title: "Test run" };

const ms = (v: number | null) => (v === null ? "n/a" : formatSavedMs(v));

export default async function TestRunPage({ params }: PageProps<"/[team]/[repo]/tests/[id]">) {
  const { team: slug, repo, id } = await params;
  // quod-server's ids are path-safe (letters, digits, dot, dash, underscore); anything else is not one.
  if (!isSlug(id)) notFound();
  const { team } = await requireTeam(slug);
  const found = await section(api().getTestRun(team.slug, id));
  if (!found.ok) return <SectionUnavailable s={found} what="This test run" page />;
  const run = found.value;
  if (!run) notFound();
  // The id is the team's; the repo in the URL is only for navigation. Keep one URL per run.
  if (run.repoSlug.toLowerCase() !== repo.toLowerCase()) redirect(testRunHref(team.slug, run));

  const asOf = new Date().toISOString();
  const exit = exitLabel(run.exit);
  const spot = spotCheckSentence(run);
  const { replayed, narrowed, total } = run.files;
  return (
    <div className="page tight">
      <TestRunRefresh run={{ id: run.id, repoSlug: run.repoSlug, sha: run.sha, suite: run.suite }} />
      <div className="row-between" style={{ alignItems: "flex-start", gap: 16 }}>
        <div style={{ display: "flex", flexDirection: "column", gap: 8, minWidth: 0 }}>
          <h1 style={{ fontSize: 24, fontWeight: 600, letterSpacing: "-0.02em", lineHeight: 1.3 }}>{testRunHeadline(run)}</h1>
          <div style={{ display: "flex", gap: 14, alignItems: "center", fontSize: 13, color: "var(--muted)", flexWrap: "wrap" }}>
            <span style={{ display: "flex", alignItems: "center", gap: 6 }}>
              <Dot tone={exit.tone} />
              {exit.text}
            </span>
            <span title={KIND_EXPLAINED[run.kind]}>
              <Chip tone={run.kind === "shadow" ? "gray" : "blue"}>{kindLabel(run.kind)}</Chip>
            </span>
            {run.branch || run.sha ? (
              <span className="mono">
                {run.branch ?? "unknown branch"}
                {run.sha ? ` · ${shortSha(run.sha)}` : ""}
              </span>
            ) : null}
            {run.workflow || run.job ? <span>{[run.workflow, run.job].filter(Boolean).join(" / ")}</span> : null}
            <time dateTime={run.at}>{formatAgo(run.at, asOf)}</time>
          </div>
          <p className="faint" style={{ fontSize: 12, lineHeight: 1.5, maxWidth: "80ch" }}>
            {SAVED_EXPLAINED}
          </p>
        </div>
        {run.githubUrl ? (
          <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
            <a className="btn" href={run.githubUrl} rel="noreferrer" target="_blank">
              View on GitHub
            </a>
          </div>
        ) : null}
      </div>

      <section aria-label="Numbers" className="stats sm">
        <div className="card stat">
          <span className="label">Skipped (unchanged)</span>
          <span className="value">
            {formatCount(replayed)} of {formatCount(total)}
          </span>
          {narrowed > 0 ? (
            <span className="note">
              {formatCount(narrowed)} of these had changes in their code, but not in the parts these tests use.
            </span>
          ) : null}
        </div>
        <div className="card stat" title={SAVED_EXPLAINED}>
          <span className="label">Saved (test-file time)</span>
          <span className="value">{formatSaved(run.timing.savedMs)}</span>
          <span className="note">An estimate: the skipped files&apos; recorded times.</span>
        </div>
        <div className="card stat">
          <span className="label">Took</span>
          <span className="value">{ms(run.timing.wallMs)}</span>
          {run.timing.selectMs !== null ? <span className="note">Picking the files to run: {ms(run.timing.selectMs)}</span> : null}
        </div>
        <div className="card stat" title="The recorded time of every test file, added up across workers.">
          <span className="label">All files, as recorded</span>
          <span className="value" style={{ color: "var(--muted)" }}>
            {ms(run.timing.estPlainMs)}
          </span>
        </div>
      </section>

      <section className="card" aria-labelledby="ran-h" style={{ overflow: "hidden" }}>
        <div className="card-head">
          <h2 id="ran-h" className="h2">
            Files that ran ({formatCount(run.ranFiles.length)})
          </h2>
        </div>
        {run.ranFiles.length ? (
          <div>
            <div className="table-head cols-files" aria-hidden="true">
              <span>File</span>
              <span>Why it ran</span>
              <span>Took</span>
            </div>
            {run.ranFiles.map((f, i) => (
              <div key={`${f.project}/${f.file}/${i}`} className="trow cols-files">
                <span className="cell-main">
                  <span className="mono" style={{ fontSize: 13 }} title={f.file}>
                    {f.file}
                  </span>
                  {f.state && f.state !== "pass" && f.state !== "passed" ? <span className="cell-sub">{f.state === "fail" ? "failed" : f.state}</span> : null}
                </span>
                <span style={{ fontSize: 13, color: "var(--muted)" }}>
                  <span className="label-sm">Why it ran </span>
                  {f.why || "not reported"}
                </span>
                <span className="mono" style={{ fontSize: 13 }}>
                  <span className="label-sm" style={{ fontFamily: "var(--sans)" }}>
                    Took{" "}
                  </span>
                  {ms(f.durationMs)}
                </span>
              </div>
            ))}
          </div>
        ) : (
          <p className="muted" style={{ fontSize: 14, padding: "0 20px 18px" }}>
            None. Every test file was skipped because nothing it depends on changed.
          </p>
        )}
      </section>

      {showSpotChecks(run) ? (
      <section className="card" aria-labelledby="spot-h" style={{ padding: 18, display: "flex", flexDirection: "column", gap: 10 }}>
        <h2 id="spot-h" className="h2">
          Spot-checks
        </h2>
        <p style={{ fontSize: 14 }}>
          {spot ??
            (run.kind === "critical" && !run.shadow
              ? "None yet. The background recording for this commit has not reported; this page updates when it does."
              : "None for this run.")}
        </p>
        {run.auditFiles.length ? (
          <ul style={{ listStyle: "none", margin: 0, padding: 0, display: "flex", flexDirection: "column", gap: 8 }}>
            {run.auditFiles.map((f) => (
              <li key={f.file} style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 13 }}>
                <Dot tone={f.ok ? "green" : "red"} />
                <span className="mono ellipsis">{f.file}</span>
                <span className={f.ok ? "faint" : ""} style={f.ok ? undefined : { color: "var(--red)" }}>
                  {f.ok ? "same result" : "different result"}
                </span>
              </li>
            ))}
          </ul>
        ) : null}
        {run.shadow ? (
          <p className="muted" style={{ fontSize: 13 }}>
            <Link href={testRunHref(team.slug, run.shadow)} style={{ textDecoration: "underline" }}>
              The background recording for this commit
            </Link>{" "}
            ran them.
          </p>
        ) : null}
      </section>
      ) : null}

      <section className="card" aria-labelledby="ledger-h" style={{ padding: 18, display: "flex", flexDirection: "column", gap: 8 }}>
        <h2 id="ledger-h" className="h2">
          Earlier recording reused
        </h2>
        <p style={{ fontSize: 14 }}>{ledgerSentence(run.ledger)}</p>
      </section>
    </div>
  );
}

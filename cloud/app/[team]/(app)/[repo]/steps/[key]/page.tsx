import Link from "next/link";
import { notFound } from "next/navigation";
import { LiveRefresh } from "@/components/live/parts";
import { History, ProofsCard } from "@/components/step-trust";
import { Dot } from "@/components/ui";
import { SectionUnavailable } from "@/components/section";
import { api } from "@/lib/api";
import { section } from "@/lib/api/section";
import { requireTeam } from "@/lib/dal";
import { formatCount, formatDuration, formatMinutes, formatPercent } from "@/lib/format";
import { trustLabel, trustMeaning, trustTone } from "@/lib/trust";
import { plainState } from "@/lib/trust-plain";
import { plainUi } from "@/lib/ui";

export const metadata = { title: "Step trust" };

export default async function StepPage({ params, searchParams }: PageProps<"/[team]/[repo]/steps/[key]">) {
  const [{ team: slug, repo: repoSlug, key }, sp] = await Promise.all([params, searchParams]);
  const { team, admin } = await requireTeam(slug);
  const found = await section(api().getStepTrust(team.slug, repoSlug, key));
  if (!found.ok) return <SectionUnavailable s={found} what="This step" page />;
  const step = found.value;
  if (!step) notFound();
  const base = `/${team.slug}/${repoSlug}`;
  const t = step.trust;
  const plain = await plainUi(sp);
  const ps = plainState(t, null, step.replaySeconds);
  const tone = plain ? ps.tone : trustTone(t.kind);
  const d = step.divergence;
  const totalInputs = step.inputs.filesRead + step.inputs.foldersListed + step.inputs.pathsNotFound;

  return (
    <div className="page tight">
      <LiveRefresh scope={{ kind: "step", repo: repoSlug, key: step.key }} types={["trust.changed", "run.finished"]} />
      <div className="row-between" style={{ alignItems: "flex-start", gap: 16 }}>
        <div style={{ display: "flex", flexDirection: "column", gap: 8, minWidth: 0 }}>
          <span style={{ fontSize: 13, color: "var(--faint)" }}>{step.job}</span>
          <h1 className={plain && !step.command ? "" : "mono"} style={{ fontSize: 22, fontWeight: 500, overflowWrap: "anywhere" }}>
            {plain && !step.command ? "Unnamed step" : step.command}
          </h1>
          {plain ? (
            <span className="mono faint" style={{ fontSize: 12 }}>
              version {step.key.slice(0, 8)}
              {step.trustRef ? ` · from runs on ${step.trustRef} (the default branch has not recorded it yet)` : ""}
            </span>
          ) : null}
        </div>
        <span className={`status-pill ${tone === "amber" || tone === "blue" ? tone : ""}`} title={plain ? ps.help : undefined}>
          <Dot tone={tone === "gray" ? "white" : tone} />
          {plain ? ps.label : trustLabel(t)}
        </span>
      </div>

      {t.kind === "paused" || t.kind === "proving" ? (
        <ProofsCard
          team={team.slug}
          repo={repoSlug}
          stepKey={step.key}
          paused={t.kind === "paused"}
          have={t.kind === "paused" ? t.consistent : t.have}
          need={step.proofsNeeded}
          canEdit={admin}
          plain={plain}
        >
          {d ? (
            <Link className="btn" href={`${base}/runs/${d.recheckRun}`}>
              {plain ? "Open the run that differed" : "Open the run that diverged"}
            </Link>
          ) : null}
        </ProofsCard>
      ) : (
        <section className="card" aria-label="Status" style={{ padding: 20, display: "flex", flexDirection: "column", gap: 16 }}>
          <p style={{ fontSize: 15, lineHeight: 1.6 }}>{plain ? ps.help : trustMeaning(t)}</p>
          <dl className="stats sm" style={{ margin: 0 }}>
            {(plain
              ? [
                  ["Skipped", step.replayRate === null || step.replayRate === 0 ? "Not yet" : `${formatPercent(step.replayRate)} of runs`],
                  ["When it runs", formatDuration(step.ranSeconds)],
                  ["When skipped", step.replaySeconds === null ? "Not skipped yet" : formatDuration(step.replaySeconds)],
                  ["Time saved", step.savedMinutes > 0 ? formatMinutes(step.savedMinutes) : step.replaySeconds === null ? "None yet" : "Under a minute"],
                ]
              : [
              ["Replayed", step.replayRate === null ? "n/a" : formatPercent(step.replayRate)],
              ["When it runs", formatDuration(step.ranSeconds)],
              ["When it replays", step.replaySeconds === null ? "n/a" : formatDuration(step.replaySeconds)],
              ["Saved", formatMinutes(step.savedMinutes)],
                ]
            ).map(([k, v]) => (
              <div key={k} style={{ display: "flex", flexDirection: "column", gap: 4 }}>
                <dt style={{ fontSize: 12, color: "var(--faint)" }}>{k}</dt>
                <dd className="mono" style={{ margin: 0, fontSize: 20 }}>
                  {v}
                </dd>
              </div>
            ))}
          </dl>
        </section>
      )}

      <div className="split">
        {d ? (
          <section id={plain ? "diff" : undefined} className="card" aria-labelledby="diff-h" style={{ flex: "1 1 420px", padding: 20, display: "flex", flexDirection: "column", gap: 12 }}>
            <h2 id="diff-h" className="h2">
              What differed
            </h2>
            <p className="muted" style={{ fontSize: 13 }}>
              {plain
                ? `Same ${formatCount(d.inputs)} inputs, two runs, different output. Usually a test that depends on time or order.`
                : `Same ${formatCount(d.inputs)} inputs, two runs, different output. Usually a timing-dependent test.`}
            </p>
            <pre className="well diff" style={{ lineHeight: 1.8 }}>
              {`  ${d.stream}, line ${d.line}\n`}
              <span className="faint">{`- recorded (run #${d.recordedRun})`}</span>
              {"\n"}
              <span className="old">{d.recorded.map((l) => `- ${l}`).join("\n")}</span>
              {"\n"}
              <span className="faint">{`+ re-check (run #${d.recheckRun})`}</span>
              {"\n"}
              <span className="new">{d.recheck.map((l) => `+ ${l}`).join("\n")}</span>
              {`\n\n  exit code   ${d.exitCode.recorded} → ${d.exitCode.recheck}`}
            </pre>
          </section>
        ) : null}
        <History events={step.history} asOf={step.asOf} plain={plain} />
      </div>

      <section className="card" aria-labelledby="inputs-h" style={{ padding: 20, display: "flex", flexDirection: "column", gap: 14 }}>
        <h2 id="inputs-h" className="h2">
          What this step reads
        </h2>
        <dl style={{ display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(160px, 1fr))", gap: 12, margin: 0 }}>
          {[
            [step.inputs.filesRead, "files read"],
            [step.inputs.foldersListed, "folders listed"],
            [step.inputs.pathsNotFound, "paths looked for, not found"],
            [step.inputs.networkAccess, "network access"],
          ].map(([v, k]) => (
            <div key={k} style={{ display: "flex", flexDirection: "column-reverse", gap: 4 }}>
              <dt style={{ fontSize: 12, color: "var(--faint)" }}>{k}</dt>
              <dd className="mono" style={{ margin: 0, fontSize: 22 }}>
                {formatCount(v as number)}
              </dd>
            </div>
          ))}
        </dl>
        <p className="muted" style={{ fontSize: 13 }}>
          Recorded by watching the step run. Nothing to configure, and nothing it reads is left out.
          <span className="sr-only"> {formatCount(totalInputs)} inputs in all.</span>
        </p>
      </section>
    </div>
  );
}

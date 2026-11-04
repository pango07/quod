// The plain wording of the trust pages (cloud/trust-plain; lib/ui-version.ts
// switches it on). One vocabulary everywhere:
//
//   skip / skipped     Quod gave back a recorded result instead of running
//   learning           not skipped yet: needs N more good runs
//   ready to skip      it will be skipped the next time nothing it reads changed
//   re-check           a skippable step Quod ran anyway to compare
//   paused             a re-check found different output; it runs every time
//   always runs        Quod never skips it (uses the network, ...)
//
// Pure: no React, no fetching. lib/trust.ts keeps the classic wording.

import { formatCount, formatDuration } from "./format";
import { receiptFor, type Receipt, type Tone } from "./trust";
import type { RunDetail, RunStep, StepSummary, TrustEvent, TrustState } from "./api/types";

const plural = (n: number, one: string, many = `${one}s`) => `${formatCount(n)} ${n === 1 ? one : many}`;

// ---------------------------------------------------------------- States

export type PlainKind = "learning" | "ready" | "skipping" | "paused" | "recording" | "always";

export interface PlainState {
  kind: PlainKind;
  /** The chip: "Learning: needs 1 good run". */
  label: string;
  /** The tooltip: what it means and what happens next. */
  help: string;
  tone: Tone;
}

/** What one step's recent runs add to its trust state (lib/trust-plain.ts `recentByKey`). */
export interface Seen {
  /** The newest run it appeared in. */
  at: string;
  run: number;
  repoSlug: string;
  outcome: "replayed" | "ran" | "running" | "queued";
  seconds: number;
  savedSeconds: number;
  /** GitHub's names, for a step without a command. */
  jobName: string;
  stepName: string;
  index?: number;
  /** In the runs looked at. */
  skips: number;
  checks: number;
  savedTotal: number;
  /** Its newest run said it uses the network. */
  network: boolean;
}

export const LEGEND: { kind: PlainKind; label: string; help: string }[] = [
  { kind: "learning", label: "Learning", help: "Quod has not seen this step give a result it can reuse yet. It runs every time, and Quod records what it reads, until it has the good runs it needs." },
  { kind: "ready", label: "Ready to skip", help: "Recorded. The next time nothing this step reads has changed, Quod gives back the recorded result instead of running it." },
  { kind: "skipping", label: "Skipping", help: "Quod skips it when nothing it reads changed. The first skips are re-run and compared, then a few at random, so a wrong result is caught." },
  { kind: "paused", label: "Paused", help: "A re-check ran the step on the same inputs and got different output. It runs every time until it gives the same result again." },
  { kind: "recording", label: "Watching only", help: "Quod is set to observe: the step always runs, Quod records what it reads, and counts how often it could have been skipped." },
  { kind: "always", label: "Always runs", help: "Quod never skips it: it uses the network, so Quod cannot see everything it depends on." },
];

const HELP = Object.fromEntries(LEGEND.map((l) => [l.kind, l.help])) as Record<PlainKind, string>;

export function plainState(t: TrustState, seen?: Pick<Seen, "checks" | "skips" | "network"> | null, replaySeconds?: number | null): PlainState {
  if (seen?.network) return { kind: "always", label: "Always runs: uses the network", help: HELP.always, tone: "dim" };
  switch (t.kind) {
    case "proving": {
      const left = Math.max(0, t.need - t.have);
      if (left === 0) return { kind: "ready", label: "Ready to skip", help: HELP.ready, tone: "blue" };
      return { kind: "learning", label: `Learning: needs ${plural(left, "good run")}`, help: HELP.learning, tone: "gray" };
    }
    case "replaying": {
      const skipped = (seen?.skips ?? 0) > 0 || (replaySeconds !== undefined && replaySeconds !== null);
      if (!skipped) return { kind: "ready", label: "Ready to skip", help: HELP.ready, tone: "blue" };
      const checks = seen?.checks ?? 0;
      return { kind: "skipping", label: checks > 0 ? `Skipping: checked ${plural(checks, "time")}` : "Skipping", help: HELP.skipping, tone: "blue" };
    }
    case "watching":
      return {
        kind: "recording",
        label: t.wouldHaveReplayed > 0 ? `Watching only: could have skipped ${formatCount(t.wouldHaveReplayed)}×` : "Watching only",
        help: HELP.recording,
        tone: "dim",
      };
    case "paused":
      return { kind: "paused", label: "Paused: a re-check found different output", help: `${HELP.paused} ${t.consistent} of ${t.need} matching runs so far.`, tone: "amber" };
  }
}

// ---------------------------------------------------------------- Recent runs, per step key

/** Per step key, what the given runs (newest first or not) say about it. */
export function recentByKey(runs: RunDetail[]): Map<string, Seen> {
  const out = new Map<string, Seen>();
  const sorted = [...runs].sort((a, b) => b.startedAt.localeCompare(a.startedAt));
  for (const run of sorted)
    for (const job of run.jobs)
      for (const s of job.steps) {
        const key = stepKeyOf(s);
        if (!key) continue;
        const saved = s.why.kind === "replayed" ? s.why.savedSeconds : 0;
        let e = out.get(key);
        if (!e) {
          e = {
            at: run.startedAt,
            run: run.number,
            repoSlug: run.repoSlug,
            outcome: s.outcome,
            seconds: s.seconds,
            savedSeconds: saved,
            jobName: job.name,
            stepName: s.name,
            index: s.index,
            skips: 0,
            checks: 0,
            savedTotal: 0,
            network: false,
          };
          out.set(key, e);
        }
        if (s.outcome === "replayed") e.skips++;
        if (s.why.kind === "spot-check") e.checks++;
        e.savedTotal += saved;
      }
  return out;
}

function stepKeyOf(s: RunStep): string | null {
  const w = s.why;
  return "stepKey" in w ? w.stepKey : null;
}

// ---------------------------------------------------------------- Grouping: one row per job and command

export interface Version {
  step: StepSummary;
  seen: Seen | null;
  state: PlainState;
}

export interface StepGroup {
  id: string;
  repoSlug: string;
  job: string;
  /** The command, or GitHub's job and step name, or "Unnamed step (#n)". */
  title: string;
  /** True when `title` is not the command (it had none). */
  untitled: boolean;
  /** The version seen most recently. */
  current: Version;
  /** Older versions, newest first. */
  earlier: Version[];
  /** Why a new version started: what the dashboard can tell. */
  reason: string;
  /** The group's time saved, in seconds: versions that replayed (not "watching only"). */
  savedSeconds: number;
  /** Time the group's watching-only versions could have saved, in seconds. Never counted as saved. */
  couldSaveSeconds: number;
}

/**
 * Steps with the same repository, job and command are one step whose
 * settings changed (each change is a new step key: the environment, a tool
 * or Quod itself was updated). The version seen in the newest run is on top.
 */
export function groupSteps(steps: StepSummary[], seen: Map<string, Seen>): StepGroup[] {
  const groups = new Map<string, Version[]>();
  let unnamed = 0;
  const titles = new Map<string, { title: string; untitled: boolean }>();
  for (const step of steps) {
    const sn = seen.get(step.key) ?? null;
    const command = step.command.trim();
    const job = step.job || sn?.jobName || "";
    let id: string;
    let title: { title: string; untitled: boolean };
    if (command) {
      id = `${step.repoSlug}\u0000${job}\u0000${command}`;
      title = { title: command, untitled: false };
    } else if (sn?.stepName) {
      id = `${step.repoSlug}\u0000${job}\u0000#${sn.stepName}`;
      title = { title: sn.jobName ? `${sn.jobName} › ${sn.stepName}` : sn.stepName, untitled: true };
    } else {
      id = `${step.repoSlug}\u0000key\u0000${step.key}`;
      unnamed++;
      title = { title: `Unnamed step (#${sn?.index ?? unnamed})`, untitled: true };
    }
    if (!titles.has(id)) titles.set(id, title);
    const v: Version = { step, seen: sn, state: plainState(step.trust, sn, step.replaySeconds) };
    groups.set(id, [...(groups.get(id) ?? []), v]);
  }
  const out: StepGroup[] = [];
  for (const [id, versions] of groups) {
    // Newest first: by the run it was last seen in; never-seen versions keep the server's order, after.
    const ordered = versions
      .map((v, i) => ({ v, i }))
      .sort((a, b) => (b.v.seen?.at ?? "").localeCompare(a.v.seen?.at ?? "") || a.i - b.i)
      .map((x) => x.v);
    const [current, ...earlier] = ordered;
    const secs = (v: Version) => Math.max(v.step.savedMinutes * 60, v.seen?.savedTotal ?? 0);
    // Each version's time goes under its own state's wording: watching-only time is never labelled saved.
    const savedSeconds = ordered.filter((v) => v.state.kind !== "recording").reduce((a, v) => a + secs(v), 0);
    const couldSaveSeconds = ordered.filter((v) => v.state.kind === "recording").reduce((a, v) => a + secs(v), 0);
    out.push({
      id,
      repoSlug: current.step.repoSlug,
      job: current.step.job || current.seen?.jobName || "",
      ...titles.get(id)!,
      current,
      earlier,
      reason: versionReason(),
      savedSeconds,
      couldSaveSeconds,
    });
  }
  // Paused first, then the most recently seen.
  const rank = (g: StepGroup) => (g.current.state.kind === "paused" ? 0 : 1);
  return out.sort((a, b) => rank(a) - rank(b) || (b.current.seen?.at ?? "").localeCompare(a.current.seen?.at ?? ""));
}

/**
 * quod-server does not say yet what differs between two keys of a step
 * (cloud/docs/api.md, "What the server could add": the key documents can be
 * diffed); until it does, say what can change one.
 */
export function versionReason(): string {
  return "Step settings changed: the environment, a tool version, or Quod itself was updated. Each change starts a new version, which learns again.";
}

// ---------------------------------------------------------------- Columns

/** "Ran in 11.9s · 4m ago", "Skipped in 0.2s · saved 11.7s · 4m ago". */
export function lastResult(v: Version, asOf: string, ago: (iso: string, now: string) => string): string {
  const s = v.seen;
  if (s && (s.outcome === "replayed" || s.outcome === "ran")) {
    const when = ago(s.at, asOf);
    if (s.outcome === "replayed") return `Skipped in ${formatDuration(s.seconds)} · saved ${formatDuration(s.savedSeconds)} · ${when}`;
    return `Ran in ${formatDuration(s.seconds)} · ${when}`;
  }
  const st = v.step;
  if (st.replaySeconds !== null) return `Usually skipped in ${formatDuration(st.replaySeconds)} (runs in ${formatDuration(st.ranSeconds)})`;
  return st.ranSeconds > 0 ? `Runs in ${formatDuration(st.ranSeconds)}` : "No recent runs";
}

/** "Skipped 3×", "Not skipped yet"; never "n/a". */
export function skippedText(v: Version): string {
  const n = v.seen?.skips ?? 0;
  if (n > 0) return `Skipped ${formatCount(n)}×`;
  if (v.step.replayRate !== null && v.step.replayRate > 0) return `Skipped in ${Math.round(v.step.replayRate * 100)}% of runs`;
  return "Not skipped yet";
}

export function savedText(seconds: number): string {
  return seconds >= 1 ? formatDuration(seconds) : "None yet";
}

// ---------------------------------------------------------------- A run's steps

export const PLAIN_OUTCOME = { replayed: "Skipped", ran: "Ran", running: "Running", queued: "Queued" } as const;

const learningLine = (have: number, need: number) =>
  have >= need ? "ready to skip next time nothing it reads changes" : `learning: needs ${plural(need - have, "more good run")} before it can be skipped`;

/** The receipt and the "why" for a step in a run, in the plain wording. */
export function plainReceiptFor(step: RunStep): Receipt {
  const r = receiptFor(step);
  const w = step.why;
  const state = PLAIN_OUTCOME[step.outcome] as Receipt["state"];
  const run = (n: number) => `run #${n}`;
  switch (w.kind) {
    case "replayed":
      return {
        ...r,
        state,
        why: `Skipped. Nothing this step reads changed since ${run(w.sourceRun)}, so Quod gave back that run’s result instead of running it.`,
        receipt: [
          `QUOD  SKIPPED · result from ${run(w.sourceRun)}`,
          `  ${plural(w.inputsMatched, "input")} checked, none changed`,
          `  took ${formatDuration(step.seconds)} · saved ${formatDuration(w.savedSeconds)}`,
          `  re-checks that found different output: ${formatCount(w.divergences)}`,
        ].join("\n"),
        facts: [
          { k: "Inputs checked", v: formatCount(w.inputsMatched) },
          { k: "Time saved", v: formatDuration(w.savedSeconds) },
          { k: "Result from", v: run(w.sourceRun) },
        ],
      };
    case "spot-check":
      return {
        ...r,
        state,
        why: w.identical
          ? `Re-check. Quod could have skipped this step, but ran it to compare with the recorded result${w.sourceRun ? ` from ${run(w.sourceRun)}` : ""}. Same output, so skipping stays on.`
          : `Re-check. Quod ran this step to compare with the recorded result and got different output from the same inputs, so it paused skipping: the step runs every time until it gives the same result again.`,
        receipt: [`QUOD  RAN · RE-CHECK · ${w.identical ? "SAME OUTPUT" : "DIFFERENT OUTPUT"}`, `  ${plural(w.inputsMatched, "input")} unchanged`, w.identical ? "  skipping stays on" : "  skipping paused"].join("\n"),
        facts: [
          { k: "Kind", v: "re-check" },
          { k: "Output", v: w.identical ? "same" : "different" },
        ],
      };
    case "changed": {
      const what = { content: "changed", missing: "was removed", created: "was added", listing: "folder listing changed" }[w.firstChange.what];
      return {
        ...r,
        state,
        why: `Ran: ${w.firstChange.path} ${what}${w.otherChanges ? ` (and ${plural(w.otherChanges, "other input")})` : ""}, so the recorded result did not apply. Quod recorded this run: ${learningLine(w.proof.have, w.proof.need)}.`,
        receipt: [`QUOD  RAN · an input changed`, `  ${w.firstChange.path} ${what}`, `  recorded · ${learningLine(w.proof.have, w.proof.need)}`].join("\n"),
      };
    }
    case "first-run":
      return {
        ...r,
        state,
        why: `Ran: the first run of this step (or of its current settings). Quod recorded what it read and its result: ${learningLine(w.proof.have, w.proof.need)}.`,
        receipt: ["QUOD  RAN · first run", `  ${plural(w.recordedInputs, "input")} recorded`, `  ${learningLine(w.proof.have, w.proof.need)}`].join("\n"),
      };
    case "paused":
      return {
        ...r,
        state,
        why: `Ran: skipping is paused because a re-check found different output. It runs every time until it gives the same result ${w.need} times in a row (${w.consistent} so far).`,
        receipt: ["QUOD  RAN · skipping paused", `  matching runs since the pause: ${w.consistent} of ${w.need}`].join("\n"),
        facts: [{ k: "Matching runs", v: `${w.consistent} of ${w.need}` }],
      };
    case "watching":
      return {
        ...r,
        state,
        why: w.couldHaveReplayed
          ? "Ran: Quod is only recording this step. Nothing it reads changed, so it could have been skipped."
          : "Ran: Quod is only recording this step. Something it reads changed, so it would have run anyway.",
        receipt: ["QUOD  RAN · recording only", `  could have skipped: ${w.couldHaveReplayed ? "yes" : "no"}`, `  ${plural(w.recordedInputs, "input")} recorded`].join("\n"),
      };
    case "network":
      return { ...r, state, why: "Ran: steps that use the network always run. Quod cannot see what they send or receive.", receipt: "QUOD  RAN · always runs (uses the network)" };
    case "running":
      return { ...r, state, why: "Running now. Quod records what it reads; the receipt appears when it finishes." };
    case "queued":
      return { ...r, state };
    case "not-memoized":
      return { ...r, state, receipt: "QUOD  always runs (not a step Quod skips)" };
  }
}

// ---------------------------------------------------------------- A step's history

export function plainEvent(e: TrustEvent): { what: string; detail: string; tone: "white" | "amber" | "green" | "blue" | "dim" } {
  switch (e.kind) {
    case "consistent":
      return { what: "Matching run", detail: `run #${e.run} · same output as #${e.sameAs}`, tone: "white" };
    case "paused":
      return { what: "Paused: a re-check found different output", detail: `run #${e.run}`, tone: "amber" };
    case "spot-check-passed":
      return { what: "Re-check: same output", detail: `run #${e.run}`, tone: "green" };
    case "replayed":
      return { what: `Skipped ${formatCount(e.times)}×`, detail: e.savedMinutes > 0 ? `saved ${e.savedMinutes}m` : "saved under a minute", tone: "blue" };
    case "proven":
      return { what: "Ready to skip", detail: `same output in runs #${e.runs[0]} and #${e.runs[1]}`, tone: "green" };
    case "first-recorded":
      return { what: "First recorded", detail: `run #${e.run} · ran in ${formatDuration(e.seconds)}`, tone: "dim" };
    case "proofs-changed":
      return { what: "Good runs needed changed", detail: `${e.from} → ${e.to} by ${e.by}`, tone: "white" };
  }
}

// ---------------------------------------------------------------- One step, in sentences (the Trust list)

/** The chip's word for each state: short, the sentence says the rest. */
export const STATE_WORD: Record<PlainKind, string> = {
  learning: "Learning",
  ready: "Ready to skip",
  skipping: "Skipping",
  paused: "Paused",
  recording: "Watching only",
  always: "Always runs",
};

/** What is happening with a step and what happens next, in one or two plain sentences. `recentRuns`: how many runs `seen` looked at. */
export function stepSentence(v: Version, recentRuns: number): string {
  const t = v.step.trust;
  switch (v.state.kind) {
    case "recording": {
      const n = t.kind === "watching" ? t.wouldHaveReplayed : 0;
      return n > 0
        ? `Quod only watches this step, so it always runs. Nothing it reads had changed ${n === 1 ? "once" : `${formatCount(n)} times`}: it could have been skipped then.`
        : "Quod only watches this step, so it always runs. It records what the step reads, to show how often it could be skipped.";
    }
    case "learning": {
      const left = t.kind === "proving" ? Math.max(0, t.need - t.have) : 1;
      return `It runs every time while Quod learns it. After ${plural(left, "more good run")} with the same inputs, it can be skipped.`;
    }
    case "ready":
      return "Quod has a result it trusts. The next time nothing this step reads has changed, it is skipped and that result is reused.";
    case "skipping": {
      const skips = v.seen?.skips ?? 0;
      const checks = v.seen?.checks ?? 0;
      const head = skips > 0 ? `Skipped ${skips === 1 ? "once" : `${formatCount(skips)} times`} in the last ${formatCount(recentRuns)} runs` : "Skipped when nothing it reads has changed";
      return `${head}. ${checks > 0 ? `${plural(checks, "skip was", "skips were")} re-run to check, and matched.` : "A few skips are re-run at random to check them."}`;
    }
    case "paused": {
      const extra = t.kind === "paused" ? ` (${formatCount(t.consistent)} of ${formatCount(t.need)} so far)` : "";
      return `A re-check got different output from the same inputs. It runs every time until it gives the same result again${extra}.`;
    }
    case "always":
      return "It uses the network, so Quod can't see everything it depends on. It always runs.";
  }
}

/** "Saved 2m 10s", "Could have saved 2m 10s" (watching only), or "Nothing saved yet". */
export function savedLine(kind: PlainKind, seconds: number): string {
  if (seconds < 1) return "Nothing saved yet";
  return kind === "recording" ? `Could have saved ${formatDuration(seconds)}` : `Saved ${formatDuration(seconds)}`;
}

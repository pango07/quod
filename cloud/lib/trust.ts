// Labels, colors and plain-language copy for trust states and step receipts.
// Shared by server and client components; pure.

import { formatCount, formatDuration } from "./format";
import type { RunStep, StepWhy, TrustState } from "./api/types";

export type Tone = "blue" | "amber" | "green" | "gray" | "dim";

export function trustLabel(t: TrustState): string {
  switch (t.kind) {
    case "replaying":
      return "Replaying";
    case "proving":
      return `Proving ${t.have} of ${t.need}`;
    case "watching":
      return "Watching";
    case "paused":
      return "Paused";
  }
}

export function trustTone(t: TrustState["kind"]): Tone {
  return t === "replaying" ? "blue" : t === "paused" ? "amber" : t === "proving" ? "gray" : "dim";
}

/** One sentence on what the state means, for a junior dev. */
export function trustMeaning(t: TrustState): string {
  switch (t.kind) {
    case "replaying":
      return "Proven. When nothing it reads has changed, Quod gives back the stored result instead of running it.";
    case "proving":
      return `It has given the same result from the same inputs ${t.have} of the ${t.need} times Quod needs before it replays.`;
    case "watching":
      return `Quod is only watching this step: it runs every time and Quod notes what it reads. It could have been replayed ${t.wouldHaveReplayed} times.`;
    case "paused":
      return "A spot check re-ran this step and got a different result from the same inputs, so Quod runs it every time until it is consistent again.";
  }
}

const runRef = (n: number) => `#${n}`;

export const OUTCOME_LABEL = { replayed: "Replayed", ran: "Ran", running: "Running", queued: "Queued" } as const;

export interface Receipt {
  title: string;
  state: (typeof OUTCOME_LABEL)[keyof typeof OUTCOME_LABEL];
  tone: Tone;
  why: string;
  receipt: string;
  facts: { k: string; v: string }[];
  stepKey: string | null;
}

/** The receipt and the "why" for a step in a run (Run.dc.html, right panel). */
export function receiptFor(step: RunStep): Receipt {
  const w: StepWhy = step.why;
  const base = { title: step.name, state: OUTCOME_LABEL[step.outcome], tone: step.outcome === "replayed" || step.outcome === "running" ? "blue" : "gray" } as const;
  switch (w.kind) {
    case "replayed":
      return {
        ...base,
        stepKey: w.stepKey,
        why: `Nothing this step reads changed since run ${runRef(w.sourceRun)}, so Quod gave back that run’s result instead of running it.`,
        receipt: [
          "QUOD  REPLAYED",
          `  proved by run ${runRef(w.sourceRun)}`,
          `  ${formatCount(w.inputsMatched)} inputs matched`,
          `  proven ${w.proof.have} of ${w.proof.need} · ${w.divergences} divergences`,
          `  replay ${formatDuration(step.seconds)} · saved ${formatDuration(w.savedSeconds)}`,
        ].join("\n"),
        facts: [
          { k: "Inputs checked", v: formatCount(w.inputsMatched) },
          { k: "Time saved", v: formatDuration(w.savedSeconds) },
          { k: "Source run", v: runRef(w.sourceRun) },
        ],
      };
    case "spot-check":
      return {
        ...base,
        stepKey: w.stepKey,
        why: w.identical
          ? `This step could have replayed, but Quod picked it for a random spot check: it ran it for real and compared the result with run ${runRef(w.sourceRun)}. Identical.`
          : `Quod picked this step for a random spot check and the real run gave a different result from run ${runRef(w.sourceRun)}, with the same inputs. Quod paused it: it runs every time until it is consistent again.`,
        receipt: [
          `QUOD  SPOT CHECK · ${w.identical ? "IDENTICAL" : "DIFFERENT"}`,
          `  compared with run ${runRef(w.sourceRun)}`,
          `  ${formatCount(w.inputsMatched)} inputs matched`,
          w.identical ? "  replay stays trusted" : "  step paused until consistent again",
        ].join("\n"),
        facts: [
          { k: "Compared with", v: runRef(w.sourceRun) },
          { k: "Result", v: w.identical ? "identical" : "different" },
        ],
      };
    case "changed": {
      const what = { content: "file content changed", missing: "file was removed", created: "file was added", listing: "folder listing changed" }[w.firstChange.what];
      const others = w.otherChanges ? `, ${w.otherChanges} more` : "";
      return {
        ...base,
        stepKey: w.stepKey,
        why: `This step read ${w.firstChange.path}, and that ${w.firstChange.what === "listing" ? "folder" : "file"} changed in this commit, so it had to run.`,
        receipt: [
          "QUOD  EXECUTED",
          `  ${w.firstChange.path}: ${what}`,
          `    (${w.firstChange.was} → ${w.firstChange.now})${others}`,
          `  recorded · proof ${w.proof.have} of ${w.proof.need} for this input set`,
        ].join("\n"),
        facts: [
          { k: "First change", v: w.firstChange.path },
          { k: "Other changes", v: w.otherChanges === 1 ? "1 file" : `${w.otherChanges} files` },
          { k: "Recorded inputs", v: formatCount(w.recordedInputs) },
        ],
      };
    }
    case "first-run":
      return {
        ...base,
        stepKey: w.stepKey,
        why: "Quod had not seen this step before. It ran, and Quod noted everything it read.",
        receipt: ["QUOD  EXECUTED", "  first run of this step", `  recorded · proof ${w.proof.have} of ${w.proof.need}`].join("\n"),
        facts: [{ k: "Recorded inputs", v: formatCount(w.recordedInputs) }],
      };
    case "paused":
      return {
        ...base,
        stepKey: w.stepKey,
        why: "This step is paused: a spot check gave a different result from the same inputs. It runs every time until it gives the same result again.",
        receipt: ["QUOD  EXECUTED · PAUSED", `  consistent runs since the pause: ${w.consistent} of ${w.need}`].join("\n"),
        facts: [{ k: "Consistent runs", v: `${w.consistent} of ${w.need}` }],
      };
    case "watching":
      return {
        ...base,
        stepKey: w.stepKey,
        why: w.couldHaveReplayed
          ? "Quod is only watching this step. Nothing it reads changed, so it would have replayed; it ran because replay is not on for it yet."
          : "Quod is only watching this step. Something it reads changed, so it would have run anyway.",
        receipt: ["QUOD  WATCHING", `  would have ${w.couldHaveReplayed ? "replayed" : "run"}`, `  ${formatCount(w.recordedInputs)} inputs recorded`].join("\n"),
        facts: [{ k: "Recorded inputs", v: formatCount(w.recordedInputs) }],
      };
    case "network":
      return {
        ...base,
        stepKey: null,
        why: "Steps that use the network never replay: Quod cannot see what they send or receive.",
        receipt: "QUOD  network step · always runs",
        facts: [{ k: "Reason", v: "uses the network" }],
      };
    case "running":
      return {
        ...base,
        stepKey: w.stepKey,
        why: "Running now. Quod is watching what it reads; the receipt appears when it finishes.",
        receipt: "QUOD  RUNNING",
        facts: [{ k: "Started", v: "just now" }],
      };
    case "queued":
      return {
        ...base,
        stepKey: w.stepKey,
        why: "Waiting for the steps before it.",
        receipt: "QUOD  QUEUED",
        facts: [],
      };
    case "not-memoized":
      return {
        ...base,
        stepKey: null,
        why:
          w.what === "checkout"
            ? "Fetching the code always runs. Quod reads the checkout to know what changed."
            : w.what === "runner setup"
              ? "Runner setup always runs."
              : "Cleanup always runs.",
        receipt: "QUOD  not a memoized step",
        facts: [{ k: "Kind", v: w.what }],
      };
  }
}

// A GitHub workflow run's jobs and steps, with what Quod did in each step.
//
// GitHub's jobs carry their display name ("test (ubuntu-24.04)") and steps
// numbered from 1; quod-server's run has jobs by the workflow's job id
// (GITHUB_JOB, "test") and steps by the name the runner reported, with
// GitHub's step number when it was known (`RunStep.index`). Pure, so it is
// tested on its own (github-merge.test.ts).

import type { RunDetail, RunStep } from "./types";

export interface GhJobLike {
  id: number;
  name: string;
  steps: { number: number; name: string }[];
}

/** Quod's step for each GitHub step it handled, keyed `${ghJobId}:${stepNumber}`; and the run's totals. */
export interface QuodMarks {
  steps: Map<string, RunStep>;
  replayed: number;
  total: number;
  savedSeconds: number;
}

/**
 * Is Quod's run (found by run number, which GitHub numbers per workflow) this
 * GitHub run? Its githubUrl names the run id when the runner reported it.
 */
export function sameGitHubRun(githubUrl: string, runId: number): boolean {
  const m = /\/actions\/runs\/(\d+)/.exec(githubUrl);
  return !m || Number(m[1]) === runId;
}

const norm = (s: string) => s.trim().toLowerCase().replace(/\s+/g, " ");

/** Does GitHub's job name belong to Quod's job (GITHUB_JOB)? "test", "test (a, b)", "test / lint". */
export function sameJob(ghName: string, quodJob: string): boolean {
  const g = norm(ghName);
  const q = norm(quodJob);
  return g === q || g.startsWith(`${q} (`) || g.startsWith(`${q} / `);
}

/** Does GitHub's step name name Quod's step? GitHub calls an unnamed `run:` step "Run <command>". */
export function sameStep(ghName: string, quodName: string): boolean {
  const g = norm(ghName);
  const q = norm(quodName);
  return g === q || g === `run ${q}` || (q.length >= 8 && g.startsWith("run ") && g.slice(4).startsWith(q));
}

export function quodMarks(jobs: GhJobLike[], run: RunDetail | null): QuodMarks {
  const marks: QuodMarks = { steps: new Map(), replayed: 0, total: 0, savedSeconds: 0 };
  if (!run) return marks;
  for (const qj of run.jobs) {
    for (const s of qj.steps) {
      if (s.outcome !== "replayed" && s.outcome !== "ran") continue;
      marks.total++;
      if (s.outcome === "replayed") {
        marks.replayed++;
        if (s.why.kind === "replayed") marks.savedSeconds += s.why.savedSeconds;
      }
    }
    const key = qj.id ?? qj.name;
    // The GitHub jobs this Quod job ran in: by name, or the only one when each side has one.
    let targets = jobs.filter((j) => sameJob(j.name, key) || sameJob(j.name, qj.name));
    if (!targets.length && jobs.length === 1 && run.jobs.length === 1) targets = jobs;
    const used = new Set<number>();
    for (const gj of targets) {
      for (const gs of gj.steps) {
        const i = qj.steps.findIndex((s, n) => !used.has(n) && (s.index !== undefined ? s.index === gs.number : sameStep(gs.name, s.name)));
        if (i < 0) continue;
        used.add(i);
        marks.steps.set(`${gj.id}:${gs.number}`, qj.steps[i]);
      }
    }
  }
  return marks;
}

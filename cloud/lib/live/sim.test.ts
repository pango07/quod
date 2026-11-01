import { describe, expect, it } from "vitest";
import { inScope } from "./scope";
import { cycleFor, cycleStart, detailAt, eventsBetween, jobsAt, MAX_RUN_MS, parseEventId, PERIOD_MS, planCycle, runEvents, T0 } from "./sim";

const NOW = T0 + 3 * 24 * 3600_000 + 12_345; // a fixed moment three days in

describe("simulator: deterministic from the clock", () => {
  it("plans the same run for the same cycle, every time", () => {
    const plain = (c: number) => JSON.stringify({ ...planCycle(c), repo: planCycle(c).repo.summary.slug });
    const a = plain(4321);
    expect(plain(4321)).toBe(a);
    expect(plain(4322)).not.toBe(a);
    // Not just the cache: a known cycle always comes out the same, on any instance, in any process.
    const r = planCycle(4321);
    expect([r.sha, r.repo.summary.slug, r.number, r.jobs.length, r.finishedAt - r.startedAt]).toEqual(["aec8ba747a64c865f452c183a341dcb587a389e5", "web", 1932, 1, 29520]);
  });

  it("gives the same events for the same window, however it is split", () => {
    const whole = eventsBetween(NOW - 60_000, NOW).map((e) => e.id);
    const parts = [...eventsBetween(NOW - 60_000, NOW - 25_000), ...eventsBetween(NOW - 25_000, NOW)].map((e) => e.id);
    expect(whole.length).toBeGreaterThan(10);
    expect(parts).toEqual(whole);
  });

  it("resumes exactly after Last-Event-ID: nothing missed, nothing repeated", () => {
    const all = eventsBetween(NOW - 90_000, NOW);
    const cut = all[Math.floor(all.length / 2)];
    const rest = eventsBetween(NOW - 90_000, NOW, { lastEventId: cut.id });
    expect(rest.map((e) => e.id)).toEqual(all.slice(all.indexOf(cut) + 1).map((e) => e.id));
  });

  it("orders event ids by time", () => {
    const ids = eventsBetween(NOW - 120_000, NOW).map((e) => parseEventId(e.id)!);
    for (let i = 1; i < ids.length; i++) expect(ids[i][0]).toBeGreaterThanOrEqual(ids[i - 1][0]);
  });

  it("maps run numbers back to their cycle", () => {
    for (let c = 1000; c < 1040; c++) {
      const run = planCycle(c);
      expect(cycleFor(run.repo.summary.slug, run.number)).toBe(c);
    }
  });

  it("keeps every run within MAX_RUN_MS, and has steps start, then finish", () => {
    for (let c = 0; c < 300; c++) {
      const run = planCycle(c);
      expect(run.finishedAt - run.startedAt).toBeLessThan(MAX_RUN_MS);
      for (const j of run.jobs) for (const s of j.steps) expect(s.endAt).toBeGreaterThanOrEqual(s.startAt);
    }
  });

  it("plays a run from Running to Replayed: a step is running, then has its final outcome", () => {
    const c = 2000;
    const run = planCycle(c);
    const step = run.jobs[0].steps.find((s) => s.outcome === "replayed") ?? run.jobs[0].steps[2];
    const during = detailAt(run, step.startAt + 1).jobs[0].steps[step.index];
    const after = detailAt(run, step.endAt).jobs[0].steps[step.index];
    expect(during.outcome).toBe("running");
    expect(after.outcome).toBe(step.outcome);
    expect(detailAt(run, run.startedAt).status).toBe("running");
    expect(detailAt(run, run.finishedAt).status).toBe("passed");
  });

  it("overlaps runs and sometimes queues jobs, so 'Now running' has something to show", () => {
    let overlap = 0;
    let queued = 0;
    for (let k = 0; k < 200; k++) {
      const jobs = jobsAt(cycleStart(5000 + k) + 4000);
      if (jobs.filter((j) => j.status === "in_progress").length >= 2) overlap++;
      if (jobs.some((j) => j.status === "queued")) queued++;
    }
    expect(overlap).toBeGreaterThan(50);
    expect(queued).toBeGreaterThan(20);
  });

  it("emits job, run, step and stats events with the fields a monitor needs", () => {
    const events = runEvents(planCycle(3000));
    const types = new Set(events.map((e) => e.type));
    for (const t of ["run.started", "job.queued", "job.started", "step.started", "step.finished", "stats.updated", "job.finished", "run.finished"]) expect(types).toContain(t);
    const job = events.find((e) => e.type === "job.queued");
    expect(job && "job" in job && job.job).toMatchObject({ workflow: "CI", status: "queued", githubUrl: expect.stringContaining("github.com") });
  });

  it("filters by scope", () => {
    const run = planCycle(3001);
    const events = runEvents(run);
    const repo = run.repo.summary.slug;
    expect(events.every((e) => inScope(e, { kind: "repo", repo }))).toBe(true);
    expect(events.some((e) => inScope(e, { kind: "repo", repo: "nope" }))).toBe(false);
    expect(events.filter((e) => inScope(e, { kind: "run", repo, run: run.number })).length).toBe(events.filter((e) => e.type !== "stats.updated" && e.type !== "trust.changed").length);
  });

  it("starts a run every PERIOD_MS", () => {
    expect(planCycle(7).startedAt - planCycle(6).startedAt).toBe(PERIOD_MS);
  });
});

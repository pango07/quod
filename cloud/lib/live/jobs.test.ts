import { describe, expect, it } from "vitest";
import type { LiveEvent, LiveJob } from "@/lib/api/types";
import { applyToDays, applyToJobs, applyToRunJobs } from "./jobs";
import { cursorAt, cycleStart, detailAt, eventsBetween, jobsAt, planCycle } from "./sim";

const key = (jobs: LiveJob[]) => jobs.map((j) => `${j.id}:${j.status}`).sort();

describe("snapshot then stream: nothing lost in between (review major 2)", () => {
  it("snapshot at T plus every event after its cursor equals the snapshot at T + x", () => {
    for (const offset of [0, 3_000, 7_777, 12_345, 19_999]) {
      const t = cycleStart(9000) + offset;
      let jobs = jobsAt(t);
      const later = t + 25_000;
      for (const e of eventsBetween(t, later, { lastEventId: cursorAt(t) })) jobs = applyToJobs(jobs, e);
      expect(key(jobs), `offset ${offset}`).toEqual(key(jobsAt(later)));
    }
  });

  it("the cursor excludes everything the snapshot already reflects", () => {
    const t = cycleStart(9100) + 1500;
    const after = eventsBetween(t - 60_000, t + 10_000, { lastEventId: cursorAt(t) });
    expect(after.every((e) => Date.parse(e.at) > t)).toBe(true);
  });
});

describe("run page steps follow step events (review major 3)", () => {
  it("applies step.started and step.finished for its run only", () => {
    const run = planCycle(9200);
    const slug = run.repo.summary.slug;
    const step = run.jobs[0].steps[2];
    let jobs = detailAt(run, step.startAt - 1).jobs;
    expect(jobs[0].steps[2].outcome).toBe("queued");
    const events = eventsBetween(step.startAt - 1, step.endAt, { lastEventId: cursorAt(step.startAt - 1) });
    const other: LiveEvent = { ...events.find((e) => e.type === "step.finished")!, run: run.number + 1 } as LiveEvent;
    jobs = applyToRunJobs(jobs, other, slug, run.number);
    for (const e of events) jobs = applyToRunJobs(jobs, e, slug, run.number);
    expect(jobs[0].steps[2]).toEqual(detailAt(run, step.endAt).jobs[0].steps[2]);
  });
});

describe("per-day chart at UTC midnight (review minor 11)", () => {
  const days = [
    { day: "2026-11-01", replayedMinutes: 10, ranMinutes: 5 },
    { day: "2026-11-02", replayedMinutes: 20, ranMinutes: 5 },
  ];
  it("adds to today's bar", () => {
    expect(applyToDays(days, "2026-11-02", { replayedMinutes: 1, ranMinutes: 2 }).at(-1)).toEqual({ day: "2026-11-02", replayedMinutes: 21, ranMinutes: 7 });
  });
  it("opens a new day and keeps the range length", () => {
    const next = applyToDays(days, "2026-11-03", { replayedMinutes: 1, ranMinutes: 0 });
    expect(next.map((d) => d.day)).toEqual(["2026-11-02", "2026-11-03"]);
  });
});


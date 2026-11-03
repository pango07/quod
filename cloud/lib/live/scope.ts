import type { LiveEvent, LiveScope } from "@/lib/api/types";
import { isSlug, parseRunNumber } from "@/lib/params";

/** ?scope=team | repo&repo= | run&repo=&run= | step&repo=&key= ; anything malformed is null (400). */
export function parseScope(q: URLSearchParams): LiveScope | null {
  const kind = q.get("scope") ?? "team";
  if (kind === "team") return { kind: "team" };
  const repo = q.get("repo");
  if (!isSlug(repo)) return null;
  if (kind === "repo") return { kind: "repo", repo };
  if (kind === "run") {
    const run = parseRunNumber(q.get("run"));
    return run === null ? null : { kind: "run", repo, run };
  }
  if (kind === "step") {
    const key = q.get("key");
    return isSlug(key) ? { kind: "step", repo, key } : null;
  }
  return null;
}

export function scopeQuery(scope: LiveScope): string {
  const q = new URLSearchParams({ scope: scope.kind });
  if (scope.kind !== "team") q.set("repo", scope.repo);
  if (scope.kind === "run") q.set("run", String(scope.run));
  if (scope.kind === "step") q.set("key", scope.key);
  return q.toString();
}

/** The repo and run an event is about (run is null for trust and stats events). */
function subject(e: LiveEvent): { repo: string; run: number | null; key: string | null } {
  switch (e.type) {
    case "job.queued":
    case "job.started":
    case "job.finished":
      return { repo: e.job.repoSlug, run: e.job.runNumber, key: null };
    case "run.started":
    case "run.finished":
      return { repo: e.run.repoSlug, run: e.run.number, key: null };
    case "step.started":
    case "step.finished":
      return { repo: e.repoSlug, run: e.run, key: e.stepKey };
    case "trust.changed":
      return { repo: e.repoSlug, run: null, key: e.key };
    case "stats.updated":
    case "test.run":
      return { repo: e.repoSlug, run: null, key: null };
  }
}

/** Does an event belong to a subscription's scope? */
export function inScope(e: LiveEvent, scope: LiveScope): boolean {
  if (scope.kind === "team") return true;
  const s = subject(e);
  if (s.repo !== scope.repo) return false;
  if (scope.kind === "repo") return true;
  if (scope.kind === "run") return s.run === scope.run;
  return s.key === scope.key;
}


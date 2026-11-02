#!/usr/bin/env python3
"""The publishable report of a bench-real-small run (scripts/bench-real/real.py).

    report.py ROWS.jsonl [--json SUMMARY.json] > report.md

Plain-language summary first, then the method, the exact repos, commits and
versions, the hardware, per-repo tables, and where the raw rows are.
"""
import argparse
import json
import statistics as st
import sys
from collections import defaultdict


def load(path):
    rows, final = [], {}
    for line in open(path):
        r = json.loads(line)
        key = (r.get("t"), r.get("repo"), r.get("series"), r.get("step"))
        if r.get("amended") and key in final:
            rows[final[key]] = r
        else:
            final[key] = len(rows)
            rows.append(r)
    return rows


def fmt_s(x):
    return f"{x:,.0f} s" if x >= 100 else f"{x:.1f} s"


def summarize(rows):
    meta = next(r for r in rows if r.get("series") == "meta")
    out = {"meta": meta, "repos": {}}
    for name in meta["repos"]:
        rr = [r for r in rows if r.get("repo") == name]
        commits = next((r for r in rr if r["series"] == "commits"), {})
        plain = [r for r in rr if r["series"] == "plain" and r.get("why") == "merge"]
        pj = defaultdict(float)
        for r in plain:
            pj[r["i"]] += r["wall"]
        pstep = {r["i"]: r for r in plain if r["step"] == "step"}
        rep = {"commits": commits.get("commits", []), "subjects": commits.get("subjects", []),
               "pr_heads": commits.get("pr_heads", []),
               "prefetch_s": next((r["wall"] for r in rr if r["series"] == "prefetch"), None),
               "wall_s": next((r["wall"] for r in rr if r["series"] == "repo-done"), None),
               "plain": {"jobs": len(pj), "total_s": sum(pj.values()),
                         "median_job_s": st.median(pj.values()) if pj else None,
                         "median_step_s": st.median(r["wall"] for r in pstep.values()) if pstep else None,
                         "failed": sum(r["exit"] != 0 for r in plain if r["step"] == "step")}}
        for series in ("history", "traffic"):
            q = [r for r in rr if r["series"] == series]
            jobs = defaultdict(float)
            for r in q:
                if r["step"] in ("checkout", "setup", "step"):
                    jobs[(r["phase"], r["i"])] += r["wall"]
            steps = [r for r in q if r["step"] == "step"]
            audits = [r for r in q if r["step"] == "audit"]
            by_phase = defaultdict(lambda: defaultdict(int))
            for r in steps:
                by_phase[r["phase"]][r["outcome"]] += 1
            miss_over = [r["wall"] - pstep[r["i"]]["wall"] for r in steps
                         if r["outcome"] != "replayed" and r["phase"] == "merge" and r["i"] in pstep]
            mism = defaultdict(int)
            for r in steps:
                if r["outcome"] != "replayed" and r.get("first_mismatch"):
                    top = r["first_mismatch"].split("/")[0] if not r["first_mismatch"].startswith(".git") else r["first_mismatch"]
                    mism[top] += 1
            merge_jobs = [v for (ph, _), v in jobs.items() if ph == "merge"]
            rep[series] = {
                "jobs": len(jobs), "total_s": sum(jobs.values()), "merge_total_s": sum(merge_jobs),
                "median_job_s": st.median(jobs.values()) if jobs else None,
                "audit_s": sum(r["wall"] for r in audits),
                "outcomes": {k: dict(v) for k, v in by_phase.items()},
                "replayed": sum(r["outcome"] == "replayed" for r in steps),
                "executed": sum(r["outcome"] != "replayed" for r in steps),
                "diverged": sum(r["outcome"] == "diverged" for r in steps),
                "wrong_replays": sum(bool(r.get("wrong_replay")) for r in steps),
                "mismatches": sum(r.get("matches_plain") is False for r in steps),
                "replay_median_s": st.median([r["wall"] for r in steps if r["outcome"] == "replayed"])
                if any(r["outcome"] == "replayed" for r in steps) else None,
                "miss_overhead_median_s": st.median(miss_over) if miss_over else None,
                "first_mismatch": dict(sorted(mism.items(), key=lambda kv: -kv[1])[:6]),
                "failed": sum(r["exit"] != 0 for r in steps),
            }
        out["repos"][name] = rep
    return out


def report(s, rows_path):
    m = s["meta"]
    v = m["versions"]
    L = []
    tot_wrong = sum(r[x]["wrong_replays"] for r in s["repos"].values() for x in ("history", "traffic"))
    L.append("# Quod on three small real repositories (bench-real-small)\n")
    L.append("## Summary\n")
    for name, r in s["repos"].items():
        cfg = m["repos"][name]
        repo = cfg["url"].split("github.com/")[1]
        h, t, p = r["history"], r["traffic"], r["plain"]
        if not p["jobs"]:
            L.append(f"- **{repo}**: did not run (see the log).")
            continue
        ratio = h["merge_total_s"] / p["total_s"] if p["total_s"] else 0
        L.append(f"- **{repo}** ({cfg['toolchain']}, `{cfg['step']}`, {p['jobs']} commits): one CI job per commit took "
                 f"{fmt_s(p['total_s'])} in total without Quod (median {p['median_job_s']:.1f} s) and "
                 f"{fmt_s(h['merge_total_s'])} with it ({ratio:.2f}x). Quod replayed the test step on "
                 f"{h['replayed']} of {h['replayed'] + h['executed']} commits. In pull-request traffic (each commit "
                 f"as its PR head, then its merge, plus re-runs) it replayed {t['replayed']} of "
                 f"{t['replayed'] + t['executed']} test steps.")
    L.append(f"- **Correctness: {tot_wrong} wrong replays.** Every Quod result, replayed or executed, was compared "
             "with a plain run of the same source tree (exit code and output, durations aside).\n")
    L.append("## Method\n")
    L.append(f"For each repository, the {m['n']} consecutive first-parent commits of its default branch ending at the "
             "pinned commit, all of them, oldest first (`git log --first-parent --reverse -n N PIN`). Nothing was "
             "picked or skipped. Three series ran interleaved on one machine (their order alternates per commit):\n")
    L.append("- **plain**: what a CI job does without Quod. A fresh checkout at a constant path (`git clone --shared`, "
             "new file times, as actions/checkout gives), the dependency install, then the test command, with warm "
             "caches that persist across commits (Go's build cache and module cache, npm's cache, uv's cache): a "
             "perfect `actions/cache`, the strongest baseline without Quod.")
    L.append("- **history**: the same jobs through Quod. The dependency install runs with network and is never "
             "memoized (`quod run --allow-net`); the test command runs `quod run --mode replay` (Go: `--warm`) with "
             "Quod's shipped trust settings (a result replays after one recording; the next two would-be replays of "
             "a step are re-executed and compared, then 5%). Audits Quod defers run right after each job "
             "(`quod audit`), timed separately and not counted in the job.")
    L.append(f"- **traffic**: realistic CI traffic, in its own Quod state: each commit first as its pull request head "
             f"(a merge commit's second parent; the commit itself otherwise), then as the merge on `main`, then with "
             f"probability {m['rerun']:.0%} (seeded, {m['seed']}) a re-run of the merge job.\n")
    L.append("Before the timed passes every commit's dependency install ran once plainly and once through Quod, so no "
             "timed run waited on the network. A job's time is checkout, install and tests.\n")
    L.append("## What was measured\n")
    L.append("| repository | license | pinned commit | dependency step | test step |")
    L.append("|---|---|---|---|---|")
    for name, cfg in m["repos"].items():
        L.append(f"| [{cfg['url'].split('github.com/')[1]}]({cfg['url']}) | {cfg.get('license', '?')} | "
                 f"`{cfg['commit'][:12]}` | `{cfg['setup']}` | `{cfg['step']}` |")
    L.append("")
    L.append(f"Quod `{m.get('quod_commit', '?')[:12]}` ({v.get('quod')}); {v.get('go')}; node {v.get('node')}; "
             f"{v.get('python')}; {v.get('uv')}; {v.get('git')}.\n")
    L.append(f"Hardware: {m.get('cpu_model') or 'CPU model not reported'}, {m['cpus']} CPUs used (of {m.get('cpus_total', m['cpus'])}), {m.get('mem_mib', 0) / 1024:.1f} GiB, "
             f"{m['arch']}, kernel {m['kernel']}, host `{m['host']}`"
             + (f", GitHub Actions run {m['github_run']}" if m.get("github_run") else "") + ".\n")
    L.append("## Results\n")
    L.append("| repository | plain total (median job) | Quod history total (median job) | Quod / plain | "
             "test step replayed / executed | Quod overhead per executed test (median) | replay (median) | "
             "traffic: replayed / executed | wrong replays |")
    L.append("|---|---:|---:|---:|---:|---:|---:|---:|---:|")
    for name, r in s["repos"].items():
        h, t, p = r["history"], r["traffic"], r["plain"]
        if not p["jobs"] or not h["jobs"]:
            continue
        ov = "-" if h["miss_overhead_median_s"] is None else f"{h['miss_overhead_median_s']:+.1f} s"
        rp = "-" if h["replay_median_s"] is None else f"{h['replay_median_s']:.2f} s"
        L.append(f"| {m['repos'][name]['url'].split('github.com/')[1]} | {fmt_s(p['total_s'])} ({p['median_job_s']:.1f} s) | "
                 f"{fmt_s(h['merge_total_s'])} ({h['median_job_s']:.1f} s) | {h['merge_total_s'] / p['total_s']:.2f}x | "
                 f"{h['replayed']} / {h['executed']} | {ov} | {rp} | {t['replayed']} / {t['executed']} | "
                 f"{h['wrong_replays'] + t['wrong_replays']} |")
    L.append("")
    for name, r in s["repos"].items():
        L.append(f"### {m['repos'][name]['url'].split('github.com/')[1]}\n")
        for series in ("history", "traffic"):
            x = r[series]
            L.append(f"- {series}: outcomes by phase {json.dumps(x['outcomes'])}; audits that diverged "
                     f"{x['diverged']}; deferred audits {fmt_s(x['audit_s'])}; where executed tests first differed "
                     f"from their closest recording: {json.dumps(x['first_mismatch'])}; failed test steps {x['failed']}; "
                     f"results that differed from plain {x['mismatches']}.")
        L.append(f"- plain: failed test steps {r['plain']['failed']}; prefetch {fmt_s(r['prefetch_s'] or 0)}; "
                 f"this repo's whole run {fmt_s(r['wall_s'] or 0)}.\n")
        L.append("| # | commit | subject | merge? |")
        L.append("|---|---|---|---|")
        for i, c in enumerate(r["commits"]):
            subj = r["subjects"][i].replace("|", "\\|")[:80] if i < len(r["subjects"]) else ""
            L.append(f"| {i} | `{c[:10]}` | {subj} | {'yes' if r['pr_heads'][i] != c else ''} |")
        L.append("")
    L.append("## Raw data\n")
    L.append(f"Every step run is one line of `{rows_path}`: series, phase, commit, tree, the exact command, wall time, "
             "exit code, Quod's outcome and phase times, the first input that differed on a miss, and the hash of "
             "the normalized output compared with plain. Logs of every run are kept beside it in the run directory.")
    return "\n".join(L) + "\n"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("rows")
    ap.add_argument("--json")
    a = ap.parse_args()
    s = summarize(load(a.rows))
    if a.json:
        with open(a.json, "w") as f:
            json.dump(s, f, indent=1, default=str)
    sys.stdout.write(report(s, a.rows))
    wrong = sum(r[x]["wrong_replays"] for r in s["repos"].values() for x in ("history", "traffic"))
    return 1 if wrong else 0


if __name__ == "__main__":
    sys.exit(main())

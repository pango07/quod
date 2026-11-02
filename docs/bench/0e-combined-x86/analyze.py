#!/usr/bin/env python3
"""0e tables from the two drivers' rows (markdown on stdout).

    analyze.py HIST_ROWS TRAFFIC_ROWS

History (history.py rows): job = checkout + step A + 3 steps for plain,
step A + 3 steps for quod (0b/0c's accounting); commit 1 (i=0) excluded.
Traffic (traffic.py MODE=both rows): job rows as the driver wrote them
(plain: checkout + A + 3 steps; quod: A + 3 steps); the tables show all
jobs and the jobs after commit 1.
"""
import json, statistics as st, sys
from collections import Counter, defaultdict

STEPS = ["go build ./...", "go vet ./...", "go test ./..."]
SHORT = {"go build ./...": "build", "go vet ./...": "vet", "go test ./...": "test", "go mod download": "A"}
PHASES = ["setup_ms", "import_ms", "lookup_ms", "mount_ms", "serve_ms", "overlay_ms", "seal_ms", "exec_ms",
          "diff_ms", "outputs_ms", "unmount_ms", "manifest_ms", "record_ms", "cleanup_ms"]


def ev(r):
    e = (r or {}).get("run_event") or {}
    return e.get("fields", e)


def outcome(r):
    return ev(r).get("outcome") or ("exit %d" % r["exit"])


def p90(xs):
    xs = sorted(xs)
    return xs[min(len(xs) - 1, int(round(0.9 * (len(xs) - 1))))]


def f(x):
    return f"{x:,.0f}" if abs(x) >= 100 else f"{x:.1f}"


def history(path):
    rows = [json.loads(l) for l in open(path)]
    by = defaultdict(dict)
    meta = {}
    for r in rows:
        if r["series"] in ("warm", "quod"):
            by[(r["series"], r["i"])][r["step"]] = r
        elif r["series"] == "meta":
            meta[r["i"]] = r
    done = lambda i: all(x in by[(s, i)] for s in ("warm", "quod") for x in STEPS + ["go mod download"])
    idx = sorted(i for i in meta if i > 0 and done(i))

    def job(s, i):
        d = by[(s, i)]
        return sum(d[x]["wall"] for x in STEPS) + d["go mod download"]["wall"] + d.get("checkout", {}).get("wall", 0)

    bad = [(s, i, x) for (s, i), d in by.items() for x, r in d.items() if r.get("exit") not in (0, None)]
    pj = [job("warm", i) for i in idx]
    qj = [job("quod", i) for i in idx]
    wins = [i for i in idx if job("quod", i) < job("warm", i)]
    print("## History: totals, commits 2-40\n")
    print("| | Total | Median job | p90 job | Min-max job |")
    print("| --- | --- | --- | --- | --- |")
    print(f"| Plain, warm Go cache | {f(sum(pj))} s | {st.median(pj):.1f} s | {p90(pj):.1f} s | {min(pj):.1f}-{max(pj):.1f} s |")
    print(f"| Quod (main) | {f(sum(qj))} s | {st.median(qj):.1f} s | {p90(qj):.1f} s | {min(qj):.1f}-{max(qj):.1f} s |")
    print(f"\nQuod / plain total: **{sum(qj)/sum(pj):.2f}x**; median job ratio {st.median([q/p for q, p in zip(qj, pj)]):.2f}x; "
          f"Quod faster on **{len(wins)} of {len(idx)}** commits ({', '.join(str(i+1) for i in wins) or 'none'}); "
          f"Quod - plain = {sum(qj)-sum(pj):+.0f} s. Non-zero exits: {bad or 'none'}.\n")
    print("### Per step, commits 2-40\n")
    print("| Step | Plain median / total | Quod median / total | Quod outcomes | Quod exec median / total |")
    print("| --- | --- | --- | --- | --- |")
    for x in ["go mod download"] + STEPS:
        pw = [by[("warm", i)][x]["wall"] for i in idx]
        qw = [by[("quod", i)][x]["wall"] for i in idx]
        oc = Counter(outcome(by[("quod", i)][x]) for i in idx)
        ex = [(ev(by[("quod", i)][x]).get("exec_ms") or 0) / 1000 if x != "go mod download" else by[("quod", i)][x]["wall"] for i in idx]
        print(f"| `{x}` | {st.median(pw):.2f} s / {f(sum(pw))} s | {st.median(qw):.2f} s / {f(sum(qw))} s | "
              f"{', '.join(f'{k} {v}' for k, v in oc.most_common())} | {st.median(ex):.2f} s / {f(sum(ex))} s |")
    print("\n### Where the difference goes (Quod - plain, seconds, commits 2-40)\n")
    print("Per step, split into the step's execution against plain's whole step (exec - plain wall) and Quod's own")
    print("time around it (wall - exec: setup, import, lookup, mount, diff, outputs, record, cleanup, process start).\n")
    print("| Step | Quod - plain | of which exec - plain | of which Quod around exec | on replays | on audits | on misses, heavy commits (plain go test > 30 s) | on misses, other commits |")
    print("| --- | --- | --- | --- | --- | --- | --- | --- |")
    tot = Counter()
    for x in ["go mod download"] + STEPS:
        d = e = o = 0.0
        parts = Counter(rep=0.0, aud=0.0, heavy=0.0, exe=0.0)
        for i in idx:
            q, p = by[("quod", i)][x], by[("warm", i)][x]
            ex = (ev(q).get("exec_ms") or 0) / 1000 if x != "go mod download" else q["wall"]
            dd = q["wall"] - p["wall"]
            d += dd; e += ex - p["wall"]; o += q["wall"] - ex
            heavy = by[("warm", i)]["go test ./..."]["wall"] > 30
            oc = outcome(q)
            kk = "rep" if oc == "replayed" else "aud" if oc == "audit" else "heavy" if heavy else "exe"
            parts[kk] += dd
        tot.update(dict(d=d, e=e, o=o, **parts))
        print(f"| `{x}` | {d:+.0f} | {e:+.0f} | {o:+.0f} | {parts['rep']:+.0f} | {parts['aud']:+.0f} | {parts['heavy']:+.0f} | {parts['exe']:+.0f} |")
    co = sum(by[("warm", i)].get("checkout", {}).get("wall", 0) for i in idx)
    print(f"| plain's checkout (Quod's is not timed) | {-co:+.0f} | | | | | | |")
    print(f"| **job** | **{tot['d']-co:+.0f}** | {tot['e']:+.0f} | {tot['o']:+.0f} | {tot['rep']:+.0f} | {tot['aud']:+.0f} | {tot['heavy']:+.0f} | {tot['exe']:+.0f} |")
    print("\n### Quod phases per keyed step, median ms over commits 2-40 (executions / replays)\n")
    print("| Step | n exec / replay | " + " | ".join(p[:-3] for p in PHASES) + " | wall - total_ms |")
    print("| --- | --- | " + " | ".join("---" for _ in PHASES) + " | --- |")
    for x in STEPS:
        for kind in ("exec", "replay"):
            sel = [by[("quod", i)][x] for i in idx if (outcome(by[("quod", i)][x]) == "replayed") == (kind == "replay")]
            if not sel:
                continue
            cells = [f"{st.median([ev(r).get(p) or 0 for r in sel]):.0f}" for p in PHASES]  # noqa
            out = st.median([r["wall"] * 1000 - (ev(r).get("total_ms") or 0) for r in sel])
            print(f"| `{x}` {kind} | {len(sel)} | " + " | ".join(cells) + f" | {out:.0f} |")
    print("\n### Per commit\n")
    print("job = checkout + A + 3 steps (plain), A + 3 steps (Quod). Quod cells: outcome and wall; the last column is go test's first changed input.\n")
    print("| # | Commit | Subject | plain job | quod job | quod - plain | build | vet | test | plain test | test first changed input |")
    print("| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |")
    for i in sorted(meta):
        if not done(i):
            continue
        q = by[("quod", i)]
        cells = [f"{outcome(q[x]).replace('replayed', '**HIT**')} {q[x]['wall']:.1f}s" for x in STEPS]
        fm = ev(q["go test ./..."]).get("first_mismatch") or "-"
        pjob, qjob = job("warm", i), job("quod", i)
        print(f"| {i+1} | `{meta[i]['commit'][:10]}` | {meta[i]['subject'][:48].replace('|', '/')} | {pjob:.1f}s | {qjob:.1f}s | "
              f"{qjob-pjob:+.1f}s | " + " | ".join(cells) + f" | {by[('warm', i)]['go test ./...']['wall']:.1f}s | `{fm}` |")
    print()


def traffic(path):
    rows = [json.loads(l) for l in open(path)]
    jobs = defaultdict(dict)   # (i, phase) -> series -> job row
    steps = defaultdict(dict)  # (i, phase, series) -> step -> row
    for r in rows:
        if r.get("step") == "job":
            jobs[(r["i"], r["phase"])][r["series"]] = r
        elif r.get("series") in ("plain", "quod") and r.get("step") in STEPS + ["go mod download", "checkout"]:
            steps[(r["i"], r["phase"], r["series"])][r["step"]] = r
    keys = sorted((k for k in jobs if len(jobs[k]) == 2), key=lambda k: (k[0], ["pr", "merge", "rerun"].index(k[1])))
    bad = [(k, s, x) for k in keys for s in ("plain", "quod") for x, r in steps[(k[0], k[1], s)].items() if r.get("exit") not in (0, None)]
    print("## Realistic traffic: PR head, merge, 10% re-runs\n")
    print(f"{len(keys)} jobs ({', '.join(f'{p} {n}' for p, n in Counter(k[1] for k in keys).items())}). Non-zero exits: {bad or 'none'}.\n")
    print("| Jobs | n | Plain total | Quod total | Quod / plain | Plain median | Quod median | Quod faster on |")
    print("| --- | --- | --- | --- | --- | --- | --- | --- |")
    for label, sel in [("all", keys), ("after commit 1", [k for k in keys if k[0] > 0])] + \
            [(f"{ph}, after commit 1", [k for k in keys if k[0] > 0 and k[1] == ph]) for ph in ("pr", "merge", "rerun")]:
        if not sel:
            continue
        p = [jobs[k]["plain"]["wall"] for k in sel]
        q = [jobs[k]["quod"]["wall"] for k in sel]
        w = sum(1 for a, b in zip(q, p) if a < b)
        print(f"| {label} | {len(sel)} | {f(sum(p))} s | {f(sum(q))} s | **{sum(q)/sum(p):.2f}x** | {st.median(p):.1f} s | {st.median(q):.1f} s | {w} of {len(sel)} |")
    print("\n### Keyed steps: outcomes and hit rate (all jobs)\n")
    print("| Step | phase | runs | miss | replayed | audit | other | replayed / unchanged-input | plain wall | quod wall | quod wall on replays |")
    print("| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |")
    g = Counter()
    for x in STEPS:
        for ph in ("pr", "merge", "rerun", "all"):
            sel = [k for k in keys if ph == "all" or k[1] == ph]
            if not sel:
                continue
            qs = [steps[(k[0], k[1], "quod")][x] for k in sel]
            ps = [steps[(k[0], k[1], "plain")][x] for k in sel]
            c = Counter(outcome(r) for r in qs)
            n, miss, rep, aud = len(qs), c.get("miss", 0), c.get("replayed", 0), c.get("audit", 0)
            un = n - miss
            share = f"{rep}/{un}" + (f" ({100*rep/un:.0f}%)" if un else "")
            print(f"| `{x}` | {ph} | {n} | {miss} | **{rep}** | {aud} | {n-miss-rep-aud} | {share} | {f(sum(r['wall'] for r in ps))} s | "
                  f"{f(sum(r['wall'] for r in qs))} s | {f(sum(r['wall'] for r in qs if outcome(r) == 'replayed'))} s |")
            if ph == "all":
                g.update(dict(n=n, miss=miss, rep=rep, aud=aud, un=un))
    print(f"\nAll keyed steps: {g['n']} runs, {g['miss']} misses, **{g['rep']} replayed of {g['un']} unchanged-input runs "
          f"({100*g['rep']/max(1, g['un']):.0f}%)**, {g['aud']} audits; replayed / all keyed runs {100*g['rep']/g['n']:.0f}%.\n")
    print("### go test replays against plain's go test on the same job\n")
    print("| # | phase | commit | quod replay | plain go test |")
    print("| --- | --- | --- | --- | --- |")
    for k in keys:
        q = steps[(k[0], k[1], "quod")]["go test ./..."]
        if outcome(q) == "replayed":
            print(f"| {k[0]+1} | {k[1]} | `{q['commit'][:10]}` | {q['wall']:.1f} s | {steps[(k[0], k[1], 'plain')]['go test ./...']['wall']:.1f} s |")
    print("\n### Where the difference goes (Quod - plain, seconds, jobs after commit 1)\n")
    print("| Step | Quod - plain | exec - plain | Quod around exec | on replays | on audits | on misses, heavy commits (plain go test > 30 s) | on misses, other commits |")
    print("| --- | --- | --- | --- | --- | --- | --- | --- |")
    sel = [k for k in keys if k[0] > 0]
    tot = Counter()
    for x in ["go mod download"] + STEPS:
        d = e = o = 0.0
        parts = Counter(rep=0.0, aud=0.0, heavy=0.0, exe=0.0)
        for k in sel:
            q, p = steps[(k[0], k[1], "quod")][x], steps[(k[0], k[1], "plain")][x]
            ex = (ev(q).get("exec_ms") or 0) / 1000 if x != "go mod download" else q["wall"]
            dd = q["wall"] - p["wall"]
            d += dd; e += ex - p["wall"]; o += q["wall"] - ex
            heavy = steps[(k[0], k[1], "plain")]["go test ./..."]["wall"] > 30
            oc = outcome(q)
            kk = "rep" if oc == "replayed" else "aud" if oc == "audit" else "heavy" if heavy else "exe"
            parts[kk] += dd
        tot.update(dict(d=d, e=e, o=o, **parts))
        print(f"| `{x}` | {d:+.0f} | {e:+.0f} | {o:+.0f} | {parts['rep']:+.0f} | {parts['aud']:+.0f} | {parts['heavy']:+.0f} | {parts['exe']:+.0f} |")
    co = sum(steps[(k[0], k[1], "plain")]["checkout"]["wall"] for k in sel)
    print(f"| plain's checkout | {-co:+.0f} | | | | | | |")
    print(f"| **job** | **{tot['d']-co:+.0f}** | {tot['e']:+.0f} | {tot['o']:+.0f} | {tot['rep']:+.0f} | {tot['aud']:+.0f} | {tot['heavy']:+.0f} | {tot['exe']:+.0f} |")
    print("\n### Per job\n")
    print("| # | phase | commit | plain job | quod job | quod - plain | build | vet | test | plain test |")
    print("| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |")
    for k in keys:
        q = steps[(k[0], k[1], "quod")]
        cells = [f"{outcome(q[x]).replace('replayed', '**HIT**')} {q[x]['wall']:.1f}s" for x in STEPS]
        pj, qj = jobs[k]["plain"]["wall"], jobs[k]["quod"]["wall"]
        print(f"| {k[0]+1} | {k[1]} | `{q['go test ./...']['commit'][:10]}` | {pj:.1f}s | {qj:.1f}s | {qj-pj:+.1f}s | "
              + " | ".join(cells) + f" | {steps[(k[0], k[1], 'plain')]['go test ./...']['wall']:.1f}s |")
    print()


if __name__ == "__main__":
    history(sys.argv[1])
    if len(sys.argv) > 2:
        traffic(sys.argv[2])

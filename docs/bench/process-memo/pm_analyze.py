#!/usr/bin/env python3
"""Offline replay of the per-process records: what a per-process memo could
have skipped over the 40-commit history, against what Go's own cache skipped.

Reads /work/pm/hist (pmwrap records and step walls), /work/hist (the bench's
plain-warm and Quod rows and the plain-warm `go test` outputs with their
`(cached)` marks) and /work/pm/survey.txt (which test binaries exec).
Prints markdown.
"""
import json, os, sys, glob, re, collections

PM = os.environ.get("PM", "/work/pm/hist")
HIST = os.environ.get("HIST", "/work/hist")
SURVEY = os.environ.get("SURVEY", "/work/pm/survey.txt")
STEPS = {"s0": "go build ./...", "s1": "go vet ./...", "s2": "go test ./..."}
KINDS = ["compile", "asm", "vet", "link", "test", "other"]
REPLAY_COST_MS = int(os.environ.get("REPLAY_COST_MS", "0"))
KEYF = os.environ.get("KEY", "key")   # "key" (path-keyed, generic) or "ckey" (content-only, tool-aware)


def kindof(r):
    k = r["kind"]
    if k in ("compile", "asm", "vet", "link", "test"):
        return k
    return "other"


def pkg_of_cwd(cwd):
    rel = cwd.split("/hist/cli", 1)[1].strip("/")
    return "github.com/cli/cli/v2" + ("/" + rel if rel else "")


def load_procs(path):
    out = []
    if not os.path.exists(path):
        return out
    for l in open(path):
        try:
            r = json.loads(l)
        except json.JSONDecodeError:
            continue
        if r["kind"] == "version":
            continue
        out.append(r)
    return out


def main():
    rows = [json.loads(l) for l in open(f"{PM}/rows.jsonl")]
    meta = {r["i"]: r for r in rows if r["series"] == "meta"}
    walls = {(r["i"], r["step"]): r for r in rows if r["series"] == "pm" and "step" in r}
    again = {r["step"]: r for r in rows if r["series"] == "again" and "step" in r}
    N = max(meta) + 1

    bench = [json.loads(l) for l in open(f"{HIST}/rows.jsonl")]
    bw = {(r["i"], r["step"]): r["wall"] for r in bench if r["series"] == "warm" and "step" in r}
    bq = {(r["i"], r["step"]): r for r in bench if r["series"] == "quod" and "step" in r}

    # Go's own verdicts per commit from the bench's plain-warm go test output.
    gocached = {}
    for f in glob.glob(f"{HIST}/logs/*-warm-s2.out"):
        i = int(os.path.basename(f)[:2])
        c, ran = set(), set()
        for ln in open(f):
            if ln.startswith("ok"):
                parts = ln.split()
                (c if "(cached)" in ln else ran).add(parts[1])
        gocached[i] = (c, ran)

    spawners = {}
    if os.path.exists(SURVEY):
        for ln in open(SURVEY):
            n, name = ln.split()
            rel = name.replace("_work_pm_hist_cli", "").strip("_").replace("_", "/")
            spawners["github.com/cli/cli/v2" + ("/" + rel if rel else "")] = int(n)

    store = collections.Counter()      # key -> commits that produced it
    per_commit = []
    unit_rows = []
    totals = collections.defaultdict(lambda: collections.Counter())
    reasons = collections.Counter()
    prev_units = {}
    for i in range(N):
        row = {"i": i, "commit": meta[i]["commit"][:10], "subject": meta[i]["subject"][:45]}
        seen_this = set()
        for tag, step in STEPS.items():
            procs = load_procs(f"{PM}/procs/{i:02d}-pm-{tag}.jsonl")
            w = walls.get((i, step))
            st = {"wall": w["wall"] if w else None, "exit": w["exit"] if w else None}
            by = collections.defaultdict(lambda: {"n": 0, "ms": 0, "hit_n": 0, "hit_ms": 0, "proven_n": 0, "proven_ms": 0, "cpu_ms": 0})
            for r in procs:
                k = kindof(r)
                b = by[k]
                b["n"] += 1; b["ms"] += r["wall_ms"]; b["cpu_ms"] += r["user_ms"] + r["sys_ms"]
                hits = store[r.get(KEYF, r["key"])]
                if hits >= 1:
                    b["hit_n"] += 1; b["hit_ms"] += r["wall_ms"]
                if hits >= 2:
                    b["proven_n"] += 1; b["proven_ms"] += r["wall_ms"]
                seen_this.add(r.get(KEYF, r["key"]))
                if tag == "s2" and k == "test":
                    pkg = pkg_of_cwd(r["cwd"])
                    gc = pkg in gocached.get(i, (set(), set()))[0]
                    gr = pkg in gocached.get(i, (set(), set()))[1]
                    unit_rows.append(dict(i=i, pkg=pkg, wall=r["wall_ms"], hit=hits >= 1, proven=hits >= 2,
                                          go_cached=gc, go_ran=gr, exit=r["exit"], last=r.get("last_line"),
                                          orphans=r.get("orphans", 0), exec_like=bool(r.get("exec_like")),
                                          spawns=spawners.get(pkg, 0) > 1, nfiles=r["n_files"], nscratch=r.get("n_scratch", 0)))
                    # Why did an unchanged-package unit miss? Compare with the previous commit's unit.
                    if i > 0 and hits == 0 and pkg in prev_units:
                        p = prev_units[pkg]
                        if p["argv_key"] != r["argv_key"]:
                            reasons["argv"] += 1
                        else:
                            pf, rf = p.get("files") or {}, r.get("files") or {}
                            binp = [x for x in rf if x.endswith(".test")]
                            if any(pf.get(x) != rf.get(x) for x in binp):
                                reasons["binary"] += 1
                            else:
                                ch = [x for x in set(pf) | set(rf) if pf.get(x) != rf.get(x)]
                                if ch:
                                    reasons["files:" + ",".join(sorted(ch))[:120]] += 1
                                elif (p.get("getenv") or {}) != (r.get("getenv") or {}):
                                    reasons["env"] += 1
                                else:
                                    reasons["other"] += 1
            if tag == "s2":
                prev_units = {pkg_of_cwd(r["cwd"]): r for r in procs if kindof(r) == "test"}
            st["by"] = {k: dict(v) for k, v in by.items()}
            total_ms = sum(v["ms"] for v in by.values())
            st["proc_ms"] = total_ms
            st["hit_ms"] = sum(v["hit_ms"] for v in by.values())
            st["proven_ms"] = sum(v["proven_ms"] for v in by.values())
            st["hit_n"] = sum(v["hit_n"] for v in by.values())
            st["n"] = sum(v["n"] for v in by.values())
            # Effective parallelism of the step: process-seconds per wall second.
            P = max(1.0, total_ms / 1000.0 / st["wall"]) if st["wall"] else 1.0
            st["P"] = P
            st["est_wall_hit"] = st["wall"] - (st["hit_ms"] - REPLAY_COST_MS * st["hit_n"]) / 1000.0 / P if st["wall"] else None
            st["est_wall_proven"] = st["wall"] - (st["proven_ms"] - REPLAY_COST_MS * sum(v["proven_n"] for v in by.values())) / 1000.0 / P if st["wall"] else None
            row[tag] = st
            if i >= 1:
                for k, v in by.items():
                    for f in ("n", "ms", "hit_n", "hit_ms", "proven_n", "proven_ms", "cpu_ms"):
                        totals[(tag, k)][f] += v[f]
        for k in seen_this:
            store[k] += 1
        per_commit.append(row)

    # Stability: the last commit run twice.
    stab = {}
    for tag in STEPS:
        a = load_procs(f"{PM}/procs/{N-1:02d}-pm-{tag}.jsonl")
        b = load_procs(f"{PM}/procs/{N-1:02d}-again-{tag}.jsonl")
        ak = collections.Counter(r.get(KEYF, r["key"]) for r in a)
        same = sum(1 for r in b if ak[r.get(KEYF, r["key"])] > 0)
        unstable = [r for r in b if ak[r.get(KEYF, r["key"])] == 0]
        stab[tag] = (len(b), same, unstable, a)

    out = []
    p = out.append
    p(f"# Per-process upper bound on the 40-commit cli/cli history (key: {KEYF})\n")
    p("Every process the Go command ran (compile, asm, vet, link, each package's test binary) on every commit, under Quod's warm-miss conditions (persistent GOCACHE, `-count=1`, `goindex=0`, fresh HOME, fresh checkout), each keyed on exactly what it read. A **hit** is a process whose key was produced on an earlier commit; **proven** needs two earlier commits (the shipped trust rule). Commit 1 is excluded from totals as in the bench. The wall estimate removes a hit's wall time divided by the step's measured parallelism (process-seconds per wall second), i.e. assumes replay is free and the parent's critical path shrinks proportionally; replay cost per process in this run: %d ms.\n" % REPLAY_COST_MS)

    p("## Totals, commits 2-%d\n" % N)
    p("| Step | Kind | Processes | Serial wall | Hit (key seen before) | Hit wall | Proven (seen twice) | Proven wall |")
    p("| --- | --- | --- | --- | --- | --- | --- | --- |")
    for tag, step in STEPS.items():
        for k in KINDS:
            t = totals.get((tag, k))
            if not t or t["n"] == 0:
                continue
            p(f"| {step} | {k} | {t['n']} | {t['ms']/1000:.0f} s | {t['hit_n']} ({100*t['hit_n']/t['n']:.0f}%) | {t['hit_ms']/1000:.0f} s ({100*t['hit_ms']/max(1,t['ms']):.0f}%) | {t['proven_n']} | {t['proven_ms']/1000:.0f} s |")
    p("")

    p("## Step wall: measured, and estimated with hits replayed free\n")
    p("| Step | Instrumented wall (sum) | Bench plain-warm (sum) | Bench Quod miss (sum) | Est. with hits free | Est. with proven free |")
    p("| --- | --- | --- | --- | --- | --- |")
    for tag, step in STEPS.items():
        rs = [r[tag] for r in per_commit[1:] if r.get(tag) and r[tag]["wall"]]
        wall = sum(r["wall"] for r in rs)
        pw = sum(bw.get((r["i"], step), 0) for r in per_commit[1:])
        qw = sum(bq[(r["i"], step)]["wall"] for r in per_commit[1:] if (r["i"], step) in bq)
        eh = sum(r["est_wall_hit"] for r in rs)
        ep = sum(r["est_wall_proven"] for r in rs)
        p(f"| {step} | {wall:.0f} s | {pw:.0f} s | {qw:.0f} s | {eh:.0f} s ({100*(1-eh/wall):.0f}% less) | {ep:.0f} s ({100*(1-ep/wall):.0f}% less) |")
    p("")

    # Test units against Go's cache.
    p("## `go test ./...` units against Go's own test cache (commits 2-%d)\n" % N)
    cnt = collections.Counter(); ms = collections.Counter()
    for u in unit_rows:
        if u["i"] == 0:
            continue
        cls = ("memo-hit" if u["hit"] else "memo-miss") + "/" + ("go-cached" if u["go_cached"] else "go-ran")
        cnt[cls] += 1; ms[cls] += u["wall"]
    p("| Class | Units | Test-binary wall (serial) |")
    p("| --- | --- | --- |")
    for cls in sorted(cnt):
        p(f"| {cls} | {cnt[cls]} | {ms[cls]/1000:.1f} s |")
    p("")
    p("`memo-hit/go-ran`: what a content-keyed process memo skips that Go's cache re-ran (the win over Go). `memo-miss/go-cached`: what Go skipped but the content key did not match (a key leak or Go's own unsoundness).\n")
    # Which packages are memo-miss/go-cached, and why
    mm = collections.Counter(u["pkg"] for u in unit_rows if u["i"] > 0 and not u["hit"] and u["go_cached"])
    if mm:
        p("Packages Go cached but the memo missed (count over commits): " + ", ".join(f"`{k.split('/v2/')[-1]}` x{v}" for k, v in mm.most_common(15)) + "\n")
    mh = collections.Counter(u["pkg"] for u in unit_rows if u["i"] > 0 and u["hit"] and u["go_ran"])
    mhms = collections.Counter()
    for u in unit_rows:
        if u["i"] > 0 and u["hit"] and u["go_ran"]:
            mhms[u["pkg"]] += u["wall"]
    if mh:
        p("Packages the memo hit while Go re-ran them (count, serial ms over commits): " + ", ".join(f"`{k.split('/v2/')[-1]}` x{v} ({mhms[k]/1000:.1f} s)" for k, v in mh.most_common(12)) + "\n")
    p("Why test units missed when the same package ran on the previous commit: " + ", ".join(f"{k}: {v}" for k, v in reasons.most_common(12)) + "\n")
    sp = sorted({u["pkg"] for u in unit_rows if u["spawns"]})
    p(f"Test binaries that exec a subprocess (strace survey at the last commit): {len(sp)}: " + ", ".join(f"`{x.split('/v2/')[-1]}`" for x in sp) + "\n")
    el = sorted({u["pkg"] for u in unit_rows if u["exec_like"]})
    p(f"Test binaries whose testlog stats an executable path: {len(el)}.\n")
    fails = [(u["i"], u["pkg"], u["exit"]) for u in unit_rows if u["exit"] != 0]
    p(f"Failing or killed test units: {len(fails)}" + (": " + "; ".join(f"commit {i+1} {pk.split('/v2/')[-1]} exit {e}" for i, pk, e in fails[:10]) if fails else "") + "\n")

    p("## Key stability: the last commit run twice on the same tree\n")
    p("| Step | Processes in the second run | Same key as the first | Unstable |")
    p("| --- | --- | --- | --- |")
    for tag, step in STEPS.items():
        n, same, unstable, a = stab[tag]
        desc = ", ".join(f"{kindof(r)} {r.get('out') or pkg_of_cwd(r['cwd']).split('/v2/')[-1]}" for r in unstable[:8])
        p(f"| {step} | {n} | {same} | {len(unstable)} {desc} |")
    p("")
    # Explain unstable test units
    n, same, unstable, a = stab["s2"]
    amap = {}
    for r in a:
        if kindof(r) == "test":
            amap[r["cwd"]] = r
    for r in unstable:
        if kindof(r) != "test":
            continue
        q = amap.get(r["cwd"])
        if not q:
            continue
        pf, rf = q.get("files") or {}, r.get("files") or {}
        ch = sorted(x for x in set(pf) | set(rf) if pf.get(x) != rf.get(x))
        envch = sorted(x for x in set(q.get("getenv") or {}) | set(r.get("getenv") or {}) if (q.get("getenv") or {}).get(x) != (r.get("getenv") or {}).get(x))
        p(f"- `{pkg_of_cwd(r['cwd']).split('/v2/')[-1]}`: argv {'same' if q['argv_key']==r['argv_key'] else 'differs'}; files differing: {', '.join(ch[:6])}{' ...' if len(ch)>6 else ''}; env differing: {', '.join(envch)}")
    p("")

    p("## Per commit\n")
    p("| # | Commit | Subject | build wall / hit% / est | vet wall / hit% / est | test wall / hit% / est | test: link hits | test: unit hits | Go cached | plain-warm test | Quod test |")
    p("| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |")
    for r in per_commit:
        i = r["i"]
        def cell(tag):
            s = r.get(tag)
            if not s or not s["wall"]:
                return "-"
            pct = 100 * s["hit_ms"] / max(1, s["proc_ms"])
            return f"{s['wall']:.1f}s / {pct:.0f}% / {s['est_wall_hit']:.1f}s"
        t = r.get("s2", {}).get("by", {})
        lk = t.get("link", {}); tu = t.get("test", {})
        gc = gocached.get(i, (set(), set()))
        p(f"| {i+1} | `{r['commit']}` | {r['subject']} | {cell('s0')} | {cell('s1')} | {cell('s2')} | {lk.get('hit_n',0)}/{lk.get('n',0)} | {tu.get('hit_n',0)}/{tu.get('n',0)} | {len(gc[0])}/{len(gc[0])+len(gc[1])} | {bw.get((i,'go test ./...'),0):.1f}s | {bq[(i,'go test ./...')]['wall'] if (i,'go test ./...') in bq else 0:.1f}s |")
    p("")
    print("\n".join(out))


if __name__ == "__main__":
    main()

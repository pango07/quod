#!/usr/bin/env python3
"""Summarize one or more traffic.py rows.jsonl files (markdown on stdout).

    summary.py LABEL=path/rows.jsonl [LABEL=path/rows.jsonl ...]

Per series: outcomes per keyed step and per phase (pr, merge, rerun),
replays and audits among the runs whose inputs were unchanged (a hit or a
would-be hit: everything but a miss), wall time, and the ledger's trust
tables if the state dir is beside the rows.
"""
import json, os, sqlite3, sys
from collections import Counter, defaultdict

STEPS = ["go build ./...", "go vet ./...", "go test ./..."]


def outcome(r):
    if r.get("phase") == "plain":
        return "plain" if r["exit"] == 0 else "exit %d" % r["exit"]
    e = r.get("run_event") or {}
    f = e.get("fields", e)
    return f.get("outcome") or ("exit %d" % r["exit"] if r["exit"] else "?")


def load(path):
    return [json.loads(l) for l in open(path)]


def series(label, path):
    rows = load(path)
    meta = next((r for r in rows if r.get("step") == "meta"), {})
    steps = [r for r in rows if r.get("step") in STEPS]
    jobs = [r for r in rows if r.get("step") == "job"]
    commits = [r for r in rows if r.get("step") == "commit"]
    print(f"## {label}")
    print()
    print(f"mode `{meta.get('mode')}`, policy `{' '.join(meta.get('policy') or []) or '(defaults)'}`, "
          f"{len(commits)} commits, {len(jobs)} jobs, {len(steps)} keyed step runs, "
          f"quod `{meta.get('quod_version', '?')}`")
    print()
    print("| step | phase | runs | miss | replayed | audit | proving | diverged | other | replayed / unchanged-input runs | wall total | wall of replays | wall of executions |")
    print("| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |")
    grand = Counter()
    for s in STEPS:
        phases = sorted({r["phase"] for r in steps}, key=lambda p: ["pr", "merge", "rerun"].index(p) if p in ("pr", "merge", "rerun") else 9)
        for ph in phases + ["all"]:
            sel = [r for r in steps if r["step"] == s and (ph == "all" or r["phase"] == ph)]
            if not sel:
                continue
            c = Counter(outcome(r) for r in sel)
            n = len(sel)
            miss = c.get("miss", 0)
            rep = c.get("replayed", 0)
            aud = c.get("audit", 0)
            prov = c.get("proving", 0)
            div = c.get("diverged", 0)
            other = n - miss - rep - aud - prov - div
            unchanged = n - miss
            wall = sum(r["wall"] for r in sel)
            wr = sum(r["wall"] for r in sel if outcome(r) == "replayed")
            we = wall - wr
            share = f"{rep}/{unchanged}" + (f" ({100*rep/unchanged:.0f}%)" if unchanged else "")
            print(f"| `{s}` | {ph} | {n} | {miss} | **{rep}** | {aud} | {prov} | {div} | {other} | {share} | {wall:.0f}s | {wr:.0f}s | {we:.0f}s |")
            if ph == "all":
                grand.update(dict(runs=n, miss=miss, replayed=rep, audit=aud, proving=prov, diverged=div, unchanged=unchanged))
                grand["wall"] += wall
    u = grand["unchanged"]
    print()
    print(f"All keyed steps: {grand['runs']} runs, {grand['miss']} miss, **{grand['replayed']} replayed**, "
          f"{grand['audit']} audited, {grand['proving']} proving, {grand['diverged']} diverged; "
          f"replayed {grand['replayed']} of {u} unchanged-input runs"
          + (f" ({100*grand['replayed']/u:.0f}%)" if u else "") + f"; wall {grand['wall']:.0f}s.")
    jw = sum(r["wall"] for r in jobs)
    print(f"Jobs: {len(jobs)}, total wall {jw:.0f}s ({jw/60:.1f} min), median {sorted(r['wall'] for r in jobs)[len(jobs)//2]:.1f}s." if jobs else "")
    # Mismatches in first-input terms for the misses of go test.
    fm = Counter()
    for r in steps:
        if r["step"] == "go test ./..." and outcome(r) == "miss":
            e = r.get("run_event") or {}
            f = e.get("fields", e)
            fm[(f.get("first_mismatch") or "-").split("/")[0]] += 1
    if fm:
        print()
        print("`go test` misses by first changed input (top-level path): " + ", ".join(f"`{k}` {v}" for k, v in fm.most_common(8)))
    ledger = os.path.join(os.path.dirname(path), "state", "ledger.sqlite")
    if os.path.exists(ledger):
        db = sqlite3.connect(ledger)
        print()
        print("Ledger: " + "; ".join(f"{o} {n}" for o, n in db.execute("select outcome, count(*) from runs group by outcome order by 2 desc")))
        print()
        print("| key | cmd | state | eligible | replays | audits | divergences | volatile | shadow_from |")
        print("| --- | --- | --- | --- | --- | --- | --- | --- | --- |")
        for k, cmd, st, el, rp, au, dv, vo, sf in db.execute("select substr(step_key,1,12), cmd, state, eligible, replays, audits, divergences, volatile, shadow_from from trust order by cmd"):
            print(f"| `{k}` | `{cmd}` | {st} | {el} | {rp} | {au} | {dv} | {vo} | {sf} |")
        ph = list(db.execute("select phase, diverged, count(*) from audits group by phase, diverged"))
        print()
        print("Audits: " + ("; ".join(f"{p} {'DIVERGED' if d else 'ok'} {n}" for p, d, n in ph) or "none"))
        sus = db.execute("select count(*) from runs where suspect is not null").fetchone()[0]
        print(f"Suspect replays: {sus}")
    print()


def main():
    for arg in sys.argv[1:]:
        label, path = arg.split("=", 1)
        series(label, path)


if __name__ == "__main__":
    main()

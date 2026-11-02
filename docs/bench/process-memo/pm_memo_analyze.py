#!/usr/bin/env python3
"""Summarize the prototype history (OUT/rows.jsonl, OUT/memo/NN.jsonl) and the
mutation results (OUT/mutate-results.json) as markdown."""
import json, os, collections, glob

OUT = os.environ.get("OUT", "/work/pm/memo")
HIST = "/work/hist"
rows = [json.loads(l) for l in open(f"{OUT}/rows.jsonl")]
meta = {r["i"]: r for r in rows if r["series"] == "meta"}
w = {(r["series"], r["i"]): r for r in rows if "step" in r}
N = max(meta) + 1
bench = [json.loads(l) for l in open(f"{HIST}/rows.jsonl")]
bq = {r["i"]: r["wall"] for r in bench if r["series"] == "quod" and r.get("step") == "go test ./..."}
bw = {r["i"]: r["wall"] for r in bench if r["series"] == "warm" and r.get("step") == "go test ./..."}

p = print
p("## Prototype on the history: `go test ./...` per commit\n")
p("| # | Commit | Subject | plain-warm (paired) | memo | bench plain-warm | bench Quod miss | links replayed | tests replayed | opaque | ran (first/proving/changed) | materialize | saved (recorded) |")
p("| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |")
tw = tm = tbw = tbq = 0.0
agg = collections.Counter()
mat_total = 0
opaque_units = collections.Counter()
faster = 0
for i in range(N):
    mw = w.get(("memo", i)); pw = w.get(("warm", i))
    procs = [json.loads(l) for l in open(f"{OUT}/memo/{i:02d}.jsonl")] if os.path.exists(f"{OUT}/memo/{i:02d}.jsonl") else []
    c = collections.Counter((r["kind"], r["outcome"]) for r in procs)
    for r in procs:
        if r["outcome"] == "opaque":
            opaque_units[r["unit"] + ": " + r.get("why", "")] += 1
    links = sum(v for (k, o), v in c.items() if k == "link")
    tests = sum(v for (k, o), v in c.items() if k == "test")
    lr = c[("link", "replayed")]; tr = c[("test", "replayed")]
    op = c[("test", "opaque")]
    ran = f"{c[('link','ran-first')]+c[('test','ran-first')]}/{c[('link','ran-proving')]+c[('test','ran-proving')]}/{c[('link','ran-changed')]+c[('test','ran-changed')]}"
    mat = sum(r.get("materialize_ms", 0) for r in procs)
    saved = sum(r.get("saved_ms", 0) for r in procs)
    mat_total += mat
    if i >= 1 and mw:
        pwall = pw["wall"] if pw else bw.get(i, 0)
        tw += pwall; tm += mw["wall"]; tbw += bw.get(i, 0); tbq += bq.get(i, 0)
        for k, v in c.items():
            agg[k] += v
        if mw["wall"] < pwall:
            faster += 1
    p(f"| {i+1} | `{meta[i]['commit'][:10]}` | {meta[i]['subject'][:40]} | {pw['wall'] if pw else 0:.1f}s | {mw['wall'] if mw else 0:.1f}s (exit {mw['exit'] if mw else '-'}) | {bw.get(i,0):.1f}s | {bq.get(i,0):.1f}s | {lr}/{links} | {tr}/{tests} | {op} | {ran} | {mat/1000:.1f}s | {saved/1000:.0f}s |")
p("")
p(f"**Totals, commits 2-{N}:** plain-warm (paired where run, else the 0b bench's) {tw:.0f} s, memo {tm:.0f} s ({100*(tm/tw-1):+.0f}%); bench plain-warm {tbw:.0f} s, bench Quod miss {tbq:.0f} s. Memo faster than plain-warm on {faster}/{N-1} commits.\n")
p("Outcomes over commits 2-%d: " % N + ", ".join(f"{k[0]} {k[1]}: {v}" for k, v in sorted(agg.items())) + f". Materialization (copying replayed link outputs into $WORK): {mat_total/1000:.0f} s serial in total.\n")
p("Opaque units (count over commits): " + "; ".join(f"`{k}` x{v}" for k, v in opaque_units.most_common(20)) + "\n")
mr = json.load(open(f"{OUT}/mutate-results.json"))
p("## Unit-level mutation tests\n")
p("Results: " + json.dumps(mr["results"]) + "\n")
for d in mr["detail"]:
    p("- " + d)
p("")
p("Link fidelity (the whole step twice more on the unchanged tree):")
for f in mr["fidelity"]:
    p(f"- wall {f['wall']:.1f}s exit {f['exit']}: tests replayed {f['tests_replayed']}/{f['tests']}, links replayed {f['links_replayed']}/{f['links']}; lookups {f.get('lookup_s',0):.1f}s, materialize {f.get('materialize_s',0):.1f}s, opaque units {f.get('opaque_wall_s',0):.1f}s, ran {f.get('ran_wall_s',0):.1f}s (all serial); not replayed: {json.dumps(f['tests_other'])[:900]}")

#!/usr/bin/env python3
"""Markdown tables from data/rows.jsonl (bin/analyze.py)."""
import json, pathlib, statistics as st, subprocess, collections

HERE = pathlib.Path(__file__).resolve().parent.parent
REPO = "pango07/quod-demo-moto"
import sys
rows = [json.loads(l) for l in open(HERE / (sys.argv[1] if len(sys.argv) > 1 else "data") / "rows.jsonl")]
by_commit = collections.defaultdict(list)
for r in rows:
    by_commit[r["i"]].append(r)


def subject(sha):
    return subprocess.run(["git", "-C", str(HERE / "moto"), "show", "-s", "--format=%s", sha],
                          capture_output=True, text=True).stdout.strip()


def touched(files):
    out = set()
    for f in files:
        p = f.split("/")
        if p[0] == "moto" and len(p) > 2:
            out.add(f"moto/{p[1]}")
        elif p[0] == "tests" and len(p) > 2:
            out.add(f"tests/{p[1]}")
        else:
            out.add(p[0])
    return ", ".join(sorted(out))


def m(s):
    return f"{s / 60:.1f} min"


def kind(r):
    if r["quod_test"] == "replayed":
        return "replayed"
    d = (r["quod_test_detail"] or [""])[0]
    return "ran (network)" if "tainted" in d else "ran"


def same(r):
    q = r["quod_pytest"][-len(r["plain_pytest"]):] if r["plain_pytest"] else r["quod_pytest"]
    return q == r["plain_pytest"]


print("### Per commit\n")
print("| # | upstream | subject | files touched | plain | quod | quod/plain | jobs replayed | ran | ran twice (network) | results equal | runs |")
print("|---|---|---|---|---|---|---|---|---|---|---|---|")
tot = collections.Counter()
plains, quods = [], []
for i in sorted(by_commit):
    rs = by_commit[i]
    r0 = rs[0]
    k = collections.Counter(kind(r) for r in rs)
    eq = sum(same(r) for r in rs)
    p, q = r0["plain_run_s"], r0["quod_run_s"]
    if i > 1:
        plains.append(p); quods.append(q)
        tot["plain"] += p; tot["quod"] += q
        tot["plain_job"] += sum(r["plain_s"] for r in rs); tot["quod_job"] += sum(r["quod_s"] for r in rs)
        for kk, v in k.items():
            tot[kk] += v
        tot["jobs"] += len(rs); tot["eq"] += eq
    links = (f"[plain](https://github.com/{REPO}/actions/runs/{r0['plain_run']}) "
             f"[quod](https://github.com/{REPO}/actions/runs/{r0['quod_run']})")
    print(f"| {i} | `{r0['upstream'][:8]}` | {subject(r0['upstream'])[:60]} | {touched(r0['files'])} | {m(p)} | {m(q)} | {q / p:.2f} | "
          f"{k['replayed']}/{len(rs)} | {k['ran']} | {k['ran (network)']} | {eq}/{len(rs)} | {links} |")

print("\n### Which jobs replayed and which ran, per commit\n")
print("| # | ran | ran twice (network) | replayed |")
print("|---|---|---|---|")
short = lambda j: j.replace("services ", "")
for i in sorted(by_commit):
    rs = by_commit[i]
    g = lambda k: ", ".join(short(r["job"]) for r in rs if kind(r) == k) or "-"
    print(f"| {i} | {g('ran')} | {g('ran (network)')} | {g('replayed')} |")

n = len(plains)
print(f"\n### Totals, commits 2-{max(by_commit)} ({n} commits)\n")
print("| | plain | quod | quod/plain |")
print("|---|---|---|---|")
print(f"| sum of run wall times | {m(tot['plain'])} | {m(tot['quod'])} | {tot['quod'] / tot['plain']:.2f} |")
print(f"| median run wall time | {m(st.median(plains))} | {m(st.median(quods))} | {st.median(quods) / st.median(plains):.2f} |")
print(f"| sum of job times (runner minutes) | {m(tot['plain_job'])} | {m(tot['quod_job'])} | {tot['quod_job'] / tot['plain_job']:.2f} |")
print(f"| commits where quod was faster | | {sum(q < p for p, q in zip(plains, quods))} of {n} | |")
print(f"\nJobs: {tot['jobs']}; test step replayed {tot['replayed']}, ran {tot['ran']}, ran twice (offline, then with the network) {tot['ran (network)']}. pytest summaries equal to plain's on the same commit: {tot['eq']}/{tot['jobs']}.")

print("\n### Per job (commits 2+)\n")
print("| job | plain median | quod median | quod when replayed (median) | quod when ran (median) | replayed | ran | ran twice |")
print("|---|---|---|---|---|---|---|---|")
jobs = collections.defaultdict(list)
for r in rows:
    if r["i"] > 1:
        jobs[r["job"]].append(r)
for j in sorted(jobs):
    rs = jobs[j]
    rep = [r["quod_s"] for r in rs if kind(r) == "replayed"]
    ran = [r["quod_s"] for r in rs if kind(r) != "replayed"]
    k = collections.Counter(kind(r) for r in rs)
    f = lambda xs: f"{st.median(xs):.0f} s" if xs else "-"
    print(f"| {j} | {f([r['plain_s'] for r in rs])} | {f([r['quod_s'] for r in rs])} | {f(rep)} | {f(ran)} | {k['replayed']} | {k['ran']} | {k['ran (network)']} |")

print("\n### Quod against a hand-written path filter (commits 2+)\n")
agree = pf_only = quod_only = 0
extra = []
for r in rows:
    if r["i"] < 2 or kind(r) == "ran (network)":
        continue
    qran = kind(r) != "replayed"
    pf = r["path_filter_runs"]
    if qran == pf:
        agree += 1
    elif qran and not pf:
        quod_only += 1; extra.append((r["i"], r["job"], (r["quod_test_detail"] or [""])[0][:140]))
    else:
        pf_only += 1
print(f"Jobs whose tests Quod could replay at all (not network-tainted): agree {agree}, Quod ran and a path filter would have skipped {quod_only}, a path filter ran and Quod replayed {pf_only}.")
for e in extra[:15]:
    print(f"- commit {e[0]}, {e[1]}: {e[2]}")

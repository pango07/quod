#!/usr/bin/env python3
"""For every job, the set of packages `go test ./...` reported under Quod
equals the set plain-warm reported on the same commit, and every line is
`ok` or `?` (no FAIL).   pkgcheck.py HIST_LOGS TRAFFIC_LOGS"""
import glob, os, re, sys
from collections import defaultdict


def pkgs(p):
    s, bad = set(), 0
    for ln in open(p, errors="replace"):
        m = re.match(r"^(ok|\?|FAIL|---)\s+(\S+)", ln)
        if m:
            s.add(m.group(2))
            bad += m.group(1) not in ("ok", "?")
    return s, bad


def check(files, key):
    groups = defaultdict(dict)
    for f in files:
        k, series = key(os.path.basename(f))
        groups[k][series] = pkgs(f)
    same = differ = fails = 0
    for k, g in groups.items():
        fails += sum(b for _, b in g.values())
        if len(g) == 2:
            a, b = (x[0] for x in g.values())
            same += a == b
            differ += a != b
            if a != b:
                print("DIFFER", k, sorted(a ^ b)[:5])
    return same, differ, fails, len(groups)


h = glob.glob(os.path.join(sys.argv[1], "*-s2.out"))
# 00-<commit>-warm-s2.out / 00-<commit>-quod-s2.out
print("history: same/differ/fail-lines/jobs",
      check(h, lambda b: ("-".join(b.split("-")[:2]), b.split("-")[2])))
t = glob.glob(os.path.join(sys.argv[2], "*-s2.out"))
# 00-pr-<commit>-plain-s2.out / 00-pr-<commit>-s2.out
print("traffic: same/differ/fail-lines/jobs",
      check(t, lambda b: ("-".join(b.split("-")[:3]), "plain" if "-plain-" in b else "quod")))

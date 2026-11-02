#!/usr/bin/env python3
"""Lists files under DIR that name a variable Quod 72e3b59 withholds from keyed
steps (quod_key::VOLATILE, the GITHUB_RUN_/ACTIONS_ families, RUNNER_NAME,
RUNNER_TRACKING_ID, GH_TOKEN, INPUT_TOKEN, @actions/github), the way
quod-hooks' scan matches them (identifier boundaries; families match as prefixes).
Skips what the scan skips (.md, .rst, images, .pdf)."""
import os, re, sys
VOL = ["GITHUB_SHA","GITHUB_JOB","GITHUB_TOKEN","GITHUB_ACTOR","GITHUB_ACTOR_ID","GITHUB_TRIGGERING_ACTOR",
       "GITHUB_EVENT_NAME","GITHUB_EVENT_PATH","GITHUB_WORKFLOW_SHA","GITHUB_WORKFLOW_REF","GITHUB_ACTION",
       "GITHUB_ACTION_REF","GITHUB_ACTION_REPOSITORY","GITHUB_ACTION_PATH","GITHUB_RETENTION_DAYS",
       "INVOCATION_ID","QUOD_RUN_ID","RUNNER_NAME","RUNNER_TRACKING_ID","GH_TOKEN","INPUT_TOKEN"]
rx = re.compile(rb"(?<![A-Za-z0-9_])(?:(" + b"|".join(v.encode() for v in VOL) + rb")(?![A-Za-z0-9_])|(GITHUB_RUN_|ACTIONS_)[A-Za-z0-9_]*)|(@actions/github)")
SKIP = (".md",".markdown",".rst",".png",".jpg",".jpeg",".gif",".svg",".ico",".pdf",".webp")
hits = {}
for root, dirs, files in os.walk(sys.argv[1]):
    for f in files:
        p = os.path.join(root, f)
        if p.lower().endswith(SKIP) or os.path.islink(p):
            continue
        try:
            b = open(p, "rb").read()
        except OSError:
            continue
        names = sorted({m.group(0).decode() for m in rx.finditer(b)})
        if names:
            hits[p] = names
for p, n in sorted(hits.items()):
    print(p, " ".join(n[:6]))
print(len(hits), "files", file=sys.stderr)

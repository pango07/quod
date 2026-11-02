#!/usr/bin/env python3
"""A7 and A7b at the unit level, against the prototype's store after the
history (OUT/store, tree OUT/memo/cli at the last commit, uid 1001).

A7-unit: for K proven test units, corrupt one recorded workspace input (or
create a recorded absent path), run `go test ./<pkg>` through the shim in
replay mode and assert the unit did not replay, then run it plain
(-count=1) and assert exit code and the ok/FAIL verdict agree. A replay
with a changed input is a false hit.

A7b-unit: for K2 files no unit manifest names (README, docs), corrupt them,
run the same way, assert the unit replayed and the plain run agrees.

Link fidelity: the whole step twice more on the unchanged tree; every test
unit must replay, which it can only do when the replayed link produced the
byte-identical binary its manifest names.
"""
import json, os, random, subprocess, sys, glob, time, hashlib

OUT = os.environ.get("OUT", "/work/pm/memo")
K = int(os.environ.get("K", "40"))
K2 = int(os.environ.get("K2", "10"))
SHIM = "/work/pm/pmemo"
WS = f"{OUT}/memo/cli"
RUNNER_UID = 1001
random.seed(int(os.environ.get("SEED", "7")))

BASE = {
    "CGO_ENABLED": "0", "GOTOOLCHAIN": "local", "LANG": "C.UTF-8", "TZ": "UTC",
    "PATH": "/usr/local/go/bin:/usr/bin:/bin", "GOMODCACHE": "/work/hist/plain/gomod",
    "GOPATH": "/work/hist/plain/gopath", "GOTELEMETRY": "off", "GOPROXY": "off",
    "GOCACHE": f"{OUT}/memo/gocache", "HOME": f"{OUT}/memo/home", "GODEBUG": "goindex=0",
}
MEMO = dict(BASE, GOFLAGS=f"-mod=mod -count=1 -toolexec={SHIM} -exec={SHIM}", PMEMO_STORE=f"{OUT}/store",
            PMEMO_MODE="replay", PMEMO_PROOFS="2", PMEMO_TMP=f"{OUT}/memo/tmp", PMEMO_WS=WS)
PLAIN = dict(BASE, GOFLAGS="-mod=mod -count=1")


def as_runner():
    os.setgid(RUNNER_UID); os.setgroups([]); os.setuid(RUNNER_UID)


def go(env, args, log=None):
    e = dict(env)
    if log:
        e["PMEMO_LOG"] = log
        if os.path.exists(log):
            os.remove(log)
    p = subprocess.run(["go"] + args, cwd=WS, env=e, capture_output=True, text=True, preexec_fn=as_runner)
    verdict = [ln.split("\t")[0] for ln in p.stdout.splitlines() if ln.startswith(("ok", "FAIL", "---"))]
    return p.returncode, verdict, p.stdout + p.stderr


def outcomes(log, kind="test"):
    if not os.path.exists(log):
        return []
    return [json.loads(l) for l in open(log) if json.loads(l)["kind"] == kind]


def pkg_of(cwd):
    rel = cwd[len(WS):].strip("/")
    return "./" + rel if rel else "."


def main():
    recs = []
    for f in glob.glob(f"{OUT}/store/tests/*/*/*.json"):
        r = json.load(open(f))
        if r["streak"] >= 2 and r["result"]["exit"] == 0 and r["cwd"].startswith(WS):
            inputs = [(p, h) for p, h in r["files"].items() if p.startswith(WS + "/") and not p.endswith(".test") and h not in ("dir",)]
            if inputs:
                recs.append((r, inputs))
    print(f"proven test units with a workspace input: {len(recs)}", flush=True)
    random.shuffle(recs)
    results = {"a7_false_hits": 0, "a7_disagree": 0, "a7_ok": 0, "a7_not_tested": 0, "a7b_ok": 0, "a7b_no_replay": 0, "a7b_disagree": 0}
    log = f"{OUT}/memo/mutate.log.jsonl"   # under a runner-writable dir: the shim appends as uid 1001
    detail = []
    for r, inputs in recs[:K]:
        pkg = pkg_of(r["cwd"])
        path, h = random.choice(inputs)
        created = False
        if h == "absent":
            os.makedirs(os.path.dirname(path), exist_ok=True)
            open(path, "wb").write(b"x\n"); created = True
            os.chown(path, RUNNER_UID, RUNNER_UID)
        else:
            orig = open(path, "rb").read()
            if not orig:
                mutated = b"x"
            else:
                i = random.randrange(len(orig))
                mutated = orig[:i] + bytes([orig[i] ^ 0x5A]) + orig[i + 1:]
            open(path, "wb").write(mutated)
        try:
            code, verdict, _ = go(MEMO, ["test", pkg], log)
            outs = outcomes(log)
            replayed = any(o["outcome"] == "replayed" for o in outs)
            pcode, pverdict, _ = go(PLAIN, ["test", pkg])
            agree = (code == pcode and verdict == pverdict)
            rel = path[len(WS) + 1:]
            if not outs:
                results["a7_not_tested"] += 1; kind = "no-unit"
            elif replayed:
                results["a7_false_hits"] += 1; kind = "FALSE HIT"
            elif not agree:
                results["a7_disagree"] += 1; kind = "disagree"
            else:
                results["a7_ok"] += 1; kind = "ok"
            detail.append(f"A7 {pkg} mutated `{rel}` ({'created' if created else 'corrupted'}): {kind}; shim {[o['outcome'] for o in outs]} exit {code} {verdict}; plain exit {pcode} {pverdict}")
            print(detail[-1], flush=True)
        finally:
            if created:
                os.remove(path)
            else:
                open(path, "wb").write(orig)
    # A7b: files no manifest names.
    named = set()
    for f in glob.glob(f"{OUT}/store/tests/*/*/*.json"):
        named.update(json.load(open(f))["files"].keys())
    candidates = [p for p in glob.glob(f"{WS}/docs/**/*.md", recursive=True) + [f"{WS}/README.md", f"{WS}/LICENSE"] if p not in named]
    random.shuffle(candidates)
    pkgs = [pkg_of(r["cwd"]) for r, _ in recs]
    for p in candidates[:K2]:
        pkg = random.choice(pkgs)
        orig = open(p, "rb").read()
        open(p, "wb").write(orig + b"\nmutated\n")
        try:
            code, verdict, _ = go(MEMO, ["test", pkg], log)
            outs = outcomes(log)
            replayed = any(o["outcome"] == "replayed" for o in outs)
            pcode, pverdict, _ = go(PLAIN, ["test", pkg])
            agree = (code == pcode and verdict == pverdict)
            if not replayed:
                results["a7b_no_replay"] += 1; kind = "did not replay"
            elif not agree:
                results["a7b_disagree"] += 1; kind = "DISAGREE"
            else:
                results["a7b_ok"] += 1; kind = "ok"
            detail.append(f"A7b {pkg} after mutating unread `{p[len(WS)+1:]}`: {kind}; shim {[o['outcome'] for o in outs]}; plain exit {pcode} {pverdict}")
            print(detail[-1], flush=True)
        finally:
            open(p, "wb").write(orig)
    # Link fidelity: the whole step twice on the unchanged tree.
    fid = []
    for n in range(3):
        t = time.monotonic()
        code, verdict, _ = go(MEMO, ["test", "./..."], log)
        wall = time.monotonic() - t
        ts = outcomes(log, "test"); ls = outcomes(log, "link")
        fid.append(dict(wall=wall, exit=code, tests=len(ts), tests_replayed=sum(o["outcome"] == "replayed" for o in ts),
                        lookup_s=sum(o["lookup_ms"] for o in ts + ls) / 1000, materialize_s=sum(o["materialize_ms"] for o in ls) / 1000,
                        opaque_wall_s=sum(o["wall_ms"] for o in ts if o["outcome"] == "opaque") / 1000,
                        ran_wall_s=sum(o["wall_ms"] for o in ts + ls if o["outcome"].startswith("ran")) / 1000,
                        tests_other={o["unit"]: o["outcome"] + (":" + o.get("why", "") if o.get("why") else "") for o in ts if o["outcome"] != "replayed"},
                        links=len(ls), links_replayed=sum(o["outcome"] == "replayed" for o in ls)))
        print("fidelity", json.dumps(fid[-1])[:800], flush=True)
    json.dump(dict(results=results, detail=detail, fidelity=fid), open(f"{OUT}/mutate-results.json", "w"), indent=1)
    print(json.dumps(results))
    print("MUTATE_DONE")


if __name__ == "__main__":
    main()

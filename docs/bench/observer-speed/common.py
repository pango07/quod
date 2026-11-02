import json, os, shutil, subprocess, time

O = "/work/obsspeed"
SRC = "/work/cli"
P = "401854ece8"
H = "1531837d85"
STEPS = ["go build ./...", "go vet ./...", "go test ./..."]
RUNNER = 1001
PLAIN = f"{O}/plain"
WS = f"{PLAIN}/cli"
QREPO = f"{O}/q/cli"
QSNAP = f"{O}/q/snap"
QSTATE = f"{O}/q/state"
os.environ.update(GIT_CONFIG_COUNT="1", GIT_CONFIG_KEY_0="safe.directory", GIT_CONFIG_VALUE_0="*")
ROWS = os.environ.get("ROWS", f"{O}/rows.jsonl")


def env_plain(gocache, home, net=False):
    return {
        "CGO_ENABLED": "0", "GOFLAGS": "-mod=mod", "GOTOOLCHAIN": "local",
        "LANG": "C.UTF-8", "TZ": "UTC", "PATH": "/usr/local/go/bin:/usr/bin:/bin",
        "GOMODCACHE": f"{PLAIN}/gomod", "GOPATH": f"{PLAIN}/gopath",
        "GOTELEMETRY": "off", "GOCACHE": gocache, "HOME": home,
        "GOPROXY": "https://proxy.golang.org,direct" if net else "off",
    }


def git(*a, **kw):
    return subprocess.run(["git", "-c", "safe.directory=*"] + list(a), check=True, **kw)


def full(c):
    return subprocess.run(["git", "-c", "safe.directory=*", "-C", SRC, "rev-parse", c],
                          capture_output=True, text=True, check=True).stdout.strip()


def as_runner():
    os.setgid(RUNNER); os.setgroups([]); os.setuid(RUNNER)


def cpu():
    """System-wide CPU seconds (user+nice+system+irq+softirq, iowait, idle)."""
    f = open("/proc/stat").readline().split()[1:]
    v = [int(x) for x in f]
    t = os.sysconf("SC_CLK_TCK")
    busy = (v[0] + v[1] + v[2] + v[5] + v[6]) / t
    vm = {}
    for l in open("/proc/vmstat"):
        k, x = l.split()
        if k in ("pgpgin", "pgpgout", "pgmajfault"):
            vm[k] = int(x)
    return busy, v[0] / t + v[1] / t, v[2] / t, v[4] / t, vm["pgpgin"] / 1024, vm["pgpgout"] / 1024, vm["pgmajfault"]


def load():
    return open("/proc/loadavg").read().split()[0]


def row(d):
    with open(ROWS, "a") as f:
        f.write(json.dumps(d) + "\n")


def fresh_checkout(commit):
    if os.path.exists(WS):
        shutil.rmtree(WS)
    subprocess.run(["git", "clone", "-q", "--shared", "--no-checkout", f"{O}/src", WS], check=True, preexec_fn=as_runner)
    subprocess.run(["git", "-C", WS, "checkout", "-q", "-B", "ci", commit], check=True, preexec_fn=as_runner)


def plain_step(step, gocache, home, logbase, extra_env=None, net=False):
    env = env_plain(gocache, home, net)
    env.update(extra_env or {})
    c0 = cpu(); t = time.monotonic()
    with open(logbase + ".out", "wb") as o, open(logbase + ".err", "wb") as e:
        p = subprocess.run(["/bin/sh", "-e", "-c", step], cwd=WS, env=env, stdout=o, stderr=e, preexec_fn=as_runner)
    wall = time.monotonic() - t; c1 = cpu()
    return dict(wall=wall, exit=p.returncode, cpu_busy=c1[0] - c0[0], cpu_user=c1[1] - c0[1], cpu_sys=c1[2] - c0[2], cpu_iowait=c1[3] - c0[3], mb_in=c1[4] - c0[4], mb_out=c1[5] - c0[5], majflt=c1[6] - c0[6])


def quod_step(quod, state, commit, step, logbase, extra, env_extra=None):
    ev = f"{state}/events.jsonl"
    before = sum(1 for _ in open(ev)) if os.path.exists(ev) else 0
    args = [quod, "--state", state, "run", "--repo", QREPO, "--commit", commit] + extra + ["--step", step]
    env = dict(os.environ); env.update(env_extra or {})
    c0 = cpu(); t = time.monotonic()
    with open(logbase + ".out", "wb") as o, open(logbase + ".err", "wb") as e:
        p = subprocess.run(args, stdout=o, stderr=e, env=env)
    wall = time.monotonic() - t; c1 = cpu()
    events = [json.loads(l) for l in open(ev).read().splitlines()[before:]] if os.path.exists(ev) else []
    pick = lambda m: [x for x in events if x.get("fields", x).get("message") == m or x.get("message") == m]
    run, obs = pick("run"), pick("observer")
    return dict(wall=wall, exit=p.returncode, cpu_busy=c1[0] - c0[0], cpu_user=c1[1] - c0[1], cpu_sys=c1[2] - c0[2], cpu_iowait=c1[3] - c0[3], mb_in=c1[4] - c0[4], mb_out=c1[5] - c0[5], majflt=c1[6] - c0[6],
                run=run[-1] if run else None, obs=obs[-1] if obs else None)

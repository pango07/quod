"""Plays actions/runner against `quod hooks` (as tests/hooks_acceptance.rs
does) for a container job on a real image, and times each hook call.

usage: drive.py <quod binary> <root dir> <image> <jobs> <step script>...
"""
import json, os, subprocess, sys, time, shutil

quod, root, image, jobs = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4])
steps = sys.argv[5:]
WS = "/__w/o/r"
work = os.path.join(root, "_work")
state = os.path.join(root, "state")
repo = os.path.join(root, "o", "r")

def sh(*a, **k):
    return subprocess.run(a, check=True, capture_output=True, text=True, **k).stdout

if not os.path.isdir(repo):
    os.makedirs(repo)
    with open(os.path.join(repo, "app.py"), "w") as f:
        f.write("def add(a, b):\n    return a + b\n")
    with open(os.path.join(repo, "test_app.py"), "w") as f:
        f.write("from app import add\n\ndef test_add():\n    assert add(2, 3) == 5\n")
    os.makedirs(os.path.join(repo, ".github/workflows"))
    with open(os.path.join(repo, ".github/workflows/ci.yml"), "w") as f:
        f.write("on: push\njobs:\n  test:\n    runs-on: [self-hosted, quod]\n    container: python\n    steps:\n      - uses: actions/checkout@v4\n      - run: python -c 1\n")
    nfiles = int(os.environ.get("NFILES", "0"))
    for i in range(nfiles):
        d = os.path.join(repo, "pkg", f"m{i % 50}")
        os.makedirs(d, exist_ok=True)
        with open(os.path.join(d, f"f{i}.py"), "w") as f:
            f.write(("# filler %d\n" % i) * 400)
    sh("git", "-C", repo, "init", "-q", "-b", "main")
    sh("git", "-C", repo, "add", "-A")
    sh("git", "-C", repo, "-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "-m", "init")
for d in ["_temp/_runner_file_commands", "_temp/_github_workflow", "_actions/actions/checkout/abc/dist", "o/r", "_tool"]:
    os.makedirs(os.path.join(work, d), exist_ok=True)
head = sh("git", "-C", repo, "rev-parse", "HEAD").strip()

def mounts():
    m = lambda s, d: {"sourceVolumePath": s, "targetVolumePath": d, "readOnly": False}
    return [m(work, "/__w"), m(work + "/_temp", "/__w/_temp"), m(work + "/_temp/_github_workflow", "/github/workflow"),
            m(work + "/_actions", "/__w/_actions"), m(work + "/_tool", "/__w/_tool")]

policy = json.dumps({"mode": "replay", "proofs": 1, "audit": 0.0, "keep_jobs": False, "rootfs": "image"})

def hook(inp):
    t = time.time()
    p = subprocess.run([quod, "--state", state, "hooks", "--policy", policy], input=json.dumps(inp),
                       capture_output=True, text=True)
    dt = time.time() - t
    return p.returncode, p.stdout, p.stderr, dt

def step_env(n):
    cmds = "/__w/_temp/_runner_file_commands"
    for f in ["set_env", "add_path", "set_output", "save_state", "step_summary"]:
        open(os.path.join(work, "_temp/_runner_file_commands", f"{f}_{n}"), "w").close()
    return {
        "CI": "true", "GITHUB_ACTIONS": "true", "GITHUB_REPOSITORY": "o/r", "GITHUB_EVENT_NAME": "push",
        "GITHUB_WORKSPACE": WS, "GITHUB_SERVER_URL": "file://" + root, "GITHUB_SHA": head,
        "GITHUB_REF": "refs/heads/main", "GITHUB_WORKFLOW_REF": "o/r/.github/workflows/ci.yml@refs/heads/main",
        "GITHUB_WORKFLOW_SHA": head, "GITHUB_JOB": "test", "GITHUB_RUN_ID": str(1000 + n), "GITHUB_RUN_ATTEMPT": "1",
        "RUNNER_TEMP": "/__w/_temp", "RUNNER_TOOL_CACHE": "/__w/_tool", "RUNNER_OS": "Linux",
        "GITHUB_ENV": f"{cmds}/set_env_{n}", "GITHUB_PATH": f"{cmds}/add_path_{n}", "GITHUB_OUTPUT": f"{cmds}/set_output_{n}",
        "GITHUB_STATE": f"{cmds}/save_state_{n}", "GITHUB_STEP_SUMMARY": f"{cmds}/step_summary_{n}",
    }

for j in range(jobs):
    if j > 0 and os.environ.get("NEWCOMMIT"):
        with open(os.path.join(repo, "CHANGES"), "w") as f:
            f.write(str(j))
        sh("git", "-C", repo, "add", "-A")
        sh("git", "-C", repo, "-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "-m", f"c{j}")
        head = sh("git", "-C", repo, "rev-parse", "HEAD").strip()
    with open(os.path.join(work, "_temp/_github_workflow/event.json"), "w") as f:
        json.dump({"ref": "refs/heads/main", "after": head, "pusher": {"name": "o"},
                   "repository": {"full_name": "o/r", "default_branch": "main"}}, f)
    resp = os.path.join(work, "_temp/response.json")
    open(resp, "w").close()
    code, out, err, dt = hook({"command": "prepare_job", "responseFile": resp, "state": {}, "args": {
        "container": {"image": image, "workingDirectory": WS, "createOptions": "",
                      "environmentVariables": {"HOME": "/github/home"}, "systemMountVolumes": mounts(), "userMountVolumes": []},
        "services": []}})
    assert code == 0, err
    jid = json.load(open(resp))["state"]["job"]
    print(f"job {j}: prepare_job {dt:.2f}s")
    total = dt
    n = 1
    env = step_env(n)
    env.update({"INPUT_REPOSITORY": "o/r", "INPUT_REF": "", "INPUT_TOKEN": "", "INPUT_PATH": "",
                "INPUT_SUBMODULES": "false", "INPUT_LFS": "false"})
    code, out, err, dt = hook({"command": "run_script_step", "responseFile": resp, "state": {"job": jid}, "args": {
        "entryPoint": "/__e/node20/bin/node", "entryPointArgs": ["/__w/_actions/actions/checkout/abc/dist/index.js"],
        "environmentVariables": env, "prependPath": [], "workingDirectory": WS}})
    assert code == 0, err
    print(f"job {j}: checkout {dt:.2f}s")
    total += dt
    for s in steps:
        n += 1
        path = os.path.join(work, f"_temp/step-{n}.sh")
        with open(path, "w") as f:
            f.write(s + "\n")
        code, out, err, dt = hook({"command": "run_script_step", "responseFile": resp, "state": {"job": jid}, "args": {
            "entryPoint": "bash", "entryPointArgs": ["--noprofile", "--norc", "-e", "-o", "pipefail", f"/__w/_temp/step-{n}.sh"],
            "environmentVariables": step_env(n), "prependPath": [], "workingDirectory": WS}})
        what = "replayed" if "QUOD  REPLAYED" in err else "executed"
        lines = [l for l in err.splitlines() if l.startswith("QUOD") or l.startswith("quod")]
        print(f"job {j}: step {n} {what} exit {code} {dt:.2f}s  {lines[:2]}")
        total += dt
    code, out, err, dt = hook({"command": "cleanup_job", "responseFile": None, "state": {"job": jid}, "args": {}})
    print(f"job {j}: cleanup_job {dt:.2f}s; total {total + dt:.2f}s")

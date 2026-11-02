# Getting started: Quod Cloud on GitHub-hosted runners

Quod runs a CI step, records what it read and what it produced, and the next
time the same step meets the same inputs it replays the recorded result
instead of running it again. With Quod Cloud the results live at
`api.quodci.com`, so a fresh GitHub-hosted runner replays what another job
recorded. Your source never leaves the runner: the server keeps hashes,
manifests and the step's outputs.

## The workflow change

Three changes to a job: the OIDC permission, the setup step, and `quod run`
in front of the commands you want Quod to remember.

```yaml
permissions:
  contents: read
  id-token: write          # 1. Quod Cloud signs you in with GitHub OIDC

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: pango07/quod/.github/actions/setup@main   # 2. install quod
        with:
          toolchains: node
      - run: npm ci
      - run: quod run -- npm test                        # 3. wrap the step
```

That is the whole setup: no secrets to copy. The server knows your team from
the job's OIDC token (repository and owner), and only jobs on your writer ref
(`refs/heads/main`) record results the whole team replays. Pull requests
replay main's results and record only for their own branch.

The setup action is `pango07/quod/.github/actions/setup@main` for now; it
becomes `quodci/setup@v1` when that repository is published.

## What `quod run` does

- `quod run -- <command> <args...>` runs one command, its words passed as
  they are (each is quoted, so `&&` or a pipe is an argument, not shell).
  For a shell script use `quod run --step 'npm run build && npm test'`.
  The step starts in the directory you call it from, so
  `working-directory: web` with `quod run -- npm test` tests `web/`.
- The step runs in a sandbox that sees your checkout and `/usr`, with a
  fixed environment for its toolchain. It cannot reach the network (install
  dependencies in an ordinary step before it, as `npm ci` above).
- `--toolchain go|node|python` picks the profile. Quod detects it from the
  repository's top level (`go.mod`, `package.json`, `pyproject.toml`, ...);
  name it when the project is in a subdirectory or there are several.
- The step's exit code is the job's. If the server cannot be reached, the
  step runs as usual and nothing is recorded. If quod itself fails (not
  the step), it exits 2 with a `QUOD  ERROR` banner.

## The setup action

| Input | Default | What |
|---|---|---|
| `toolchains` | none | `go`, `node`, `python` (space-separated), installed at `/usr/local` where the sandbox sees them. A toolchain from `actions/setup-node` and friends lives under `/opt` and is not visible to the step. |
| `node-version`, `go-version`, `python-version` | 24.22.2, 1.27.3, 3.12.16 | |
| `server` | `https://api.quodci.com` | `QUOD_SERVER`; empty keeps results on the runner only |
| `server-key` | Quod Cloud's current key | `QUOD_SERVER_KEY`: the server's Ed25519 public key. Every replay's receipt must carry its signature, so a compromised endpoint cannot make a runner replay. |
| `mode` | `replay` | `observe` executes everything and reports what would have replayed |
| `version`, `sha256` | the current client release | the quod client release and its pinned checksum |

It installs bubblewrap and FUSE, turns on FUSE over io_uring when the kernel
has it, and puts `quod` on `PATH` (it runs as root through `sudo`, which
GitHub-hosted runners allow, because the sandbox and the file observer need
it). The OIDC request token reaches `quod` through a private file, never a
command line, and the step itself never sees it.

## The first runs

1. The first run of a step on main executes it and records the result.
2. The next runs on main whose inputs are identical are re-executed and
   compared (the shadow phase: two by default) before anything is replayed.
3. From then on an identical step replays in about a second; a few percent
   are still re-executed and compared at random (audits).

A replay prints `QUOD  REPLAYED` (with the run that proved it) in the step's log. The results show on
[app.quodci.com](https://app.quodci.com): runs, steps, time saved, and each
step's trust state.

What keeps a step from replaying: any input it read changed; a different
runner image, kernel or CPU (the result is only proved for the machine class
it ran on); a failed step (failures are not replayed by default).

## Limits today

- Linux x86_64 GitHub-hosted runners (`ubuntu-latest`). Toolchain profiles:
  Go, Node, Python.
- The client release is downloaded with the job's `GITHUB_TOKEN`, so for now
  it works in repositories that can read `pango07/quod`.
- A step's OIDC token is requested once, when the step starts; GitHub's tokens
  last about five minutes, so a step that runs much longer than that may
  finish unrecorded. Shorter steps are unaffected.
- Fork pull requests get no OIDC token from GitHub: their steps run and
  nothing is recorded.

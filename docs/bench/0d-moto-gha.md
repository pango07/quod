# 0d: Quod on GitHub Actions, moto's test suite, one job per service group

Before run: Quod main `72e3b59`, `proofs: 1` (a result replays after one recorded
execution), default 5% audit, replay mode, through the official
actions/runner 2.338.2 with Quod's container hooks. The same jobs ran without
Quod on a second runner on the same machine, one workflow at a time. After
run: main `5a4c18b`, default policy, 2 commits, stopped (see below).
Corpus: [getmoto/moto](https://github.com/getmoto/moto) (Apache-2.0), the
15 newest first-parent commits of master on 2026-11-02
(`6bae955b`..`3a9a6f30`; commits 1-10 measured), mirrored to the private repository
`pango07/quod-demo-moto`. Machine: Hetzner server 1 (CCX23: 4 dedicated
AMD EPYC vCPUs, 16 GB, Ubuntu 26.04, kernel 7.0). Measured 2026-11-02.

## Summary

**Short answer:** on Quod 72e3b59 (the "before" run), a moto commit that
touches one service (or no moto code at all) ran its CI in a **median 16.5 min
with Quod against 19.8 min plain (0.83x)**. Quod was faster on 8 of 9
measured commits. **99 of 99 jobs** gave the same pytest result as the plain
run on the same commit, and nothing was ever replayed wrongly. The win is
small because three of the 11 jobs ran their tests twice on every commit
(network-touching tests), and every Quod job pays about 32 s even when its
tests replay. The run stopped at commit 10 of 15, on the product owner's
decision to redo it on current main. The "after" run on main 5a4c18b covered
2 commits before it was stopped too, see below.

What was measured. moto's unit tests (about 13 minutes of tests per commit on
this box) ran as 11 GitHub Actions jobs: one per service upstream runs with
xdist (batch, dynamodb, ec2, s3, sqs), and the other ~170 test directories in
six alphabetical shards. The 15 newest commits of moto master were pushed one
at a time to a private mirror. Each push ran the same workflow twice on the
same machine, once with Quod's runner hooks and once without.

Before run (Quod 72e3b59, proofs 1, no shadow audits), commits 2-10. Commit 1
is the cold warm-up and is excluded.

| | plain | Quod | Quod / plain |
|---|---|---|---|
| sum of run wall times (9 commits) | 180.7 min | 159.2 min | 0.88 |
| median run wall time | 19.8 min | 16.5 min | 0.83 |
| sum of job times (runner minutes) | 177.0 min | 151.0 min | 0.85 |
| commits where Quod was faster | | 8 of 9 | |
| job test steps replayed / ran / ran twice | | 61 / 11 / 27 of 99 | |

- **The story, as measured.** Commit 3 changes only `moto/emr` (EMR's instance
  types data). Quod ran the one shard whose tests read it (`services d-e`) and
  the three network-tainted jobs, and replayed the other 7. The run took
  18.4 min against plain's 19.4 min. On a commit that touches no moto code
  (6 of the 9: `other_langs/` and `.github/` bumps), 8 of 11 jobs replay and
  the run takes 15.7-17.7 min against 19.6-22.4 min (0.78-0.84x).
- **A replayed job is fast, a job is not free.** A job whose tests replay
  takes 32-36 s by the hooks' clock (36-41 s as GitHub times the job): a few seconds of hooks, 20 s for the
  `pip install` (never replayed: it needs the network), 10 s for its doomed
  offline try, and the replay itself (0.0-0.3 s). Plain takes 34-153 s for the
  same jobs. ec2 drops from 83 s to 39 s, services d-e from 153 s to 37 s,
  services s-z from 128 s to 41 s. batch (34 s plain, 36 s Quod) and dynamodb
  (41 s, 38 s) gain nothing.
- **The losses.** Three jobs never replay and run their tests twice on every
  commit: offline (failing or "reaching for the network"), then again with the
  network. These are services c (moto's passthrough tests in `test_core`:
  282 s against plain's 337 s, so still not slower), services f-m (firehose's
  HTTP-destination tests: 219 s against 113 s) and s3 (83 s against 48 s).
  Together they cost about 580 s of Quod's ~990 s on a no-code commit. A
  commit that touches shared data invalidates nearly everything: commit 5
  (`moto/ec2` instance offerings, read by 8 of 11 jobs' tests) took 23.1 min
  with Quod against 20.1 min plain (1.15x). Commit 1 (cold) took 24.3 min
  against 19.9 min (1.22x).
- **Correctness.** Every one of the 99 jobs (61 of them replayed) printed
  the same pytest summary under Quod as plain printed on the same commit, the
  replays included (each replay reprints the recorded output of an earlier
  commit, so a stale result would show here as a different count). No audit
  fired (5%, per key, with 8 or fewer hits per key) and nothing diverged.
- **Quod against a path filter.** On the 72 job-commits Quod could decide
  (excluding the network-tainted jobs), Quod and a hand-written per-service
  path filter agreed 63 times. Quod ran 9 jobs that the path filter would have
  skipped: the tests of other services really read `moto/ec2/resources/...`
  and `moto/config/resources/aws_managed_rules.json` (the first changed path
  each receipt names), so a path filter would have been wrong there. The
  reverse never happened.
- **Reference: GitHub-hosted `ubuntu-latest`**, the same 11 jobs in
  parallel (run 41426446917, on the previous window's first commit): 3.1 min
  wall, 23 job-minutes (1,382 s). Parallelism, not Quod, wins wall time there.
  A self-hosted single runner serializes the jobs.

After run (current main 5a4c18b, the default policy, the natural workflow
with every earlier workaround removed): 2 commits, stopped by the product
owner. **Gone:**
- the double run of network tests: services c, f-m and s3 ran once, straight
  on the runner's surface ("needed the network on an earlier run ... without
  trying offline first");
- all of the workflow workarounds: GPG_KEY, `name:tag@digest`, a step before
  checkout, the image's stdlib bytecode, install-time `.pyc`;
- the LangChain blocker, the scan redesign (not re-measured).

**Not gone, or new:**
- `pip install` still tries offline first on every job (its inputs, the
  pip cache in `.quod/tool`, change from job to job, so the network memory
  never matches);
- the default shadow audits (2) made all five pytest-xdist jobs diverge on
  their first audit (`-n 4` prints test lines in a different order each run)
  and quarantined them. Under this policy they would never replay;
- the fixed cost per replayed job was not measured: no job replayed in the 2
  commits, because every would-be hit was a shadow audit.

Plain ran on a second CCX23 there and was about 20% faster than plain on
server 1 (964-998 s against 1,166-1,345 s for the same commits), so the after
run's plain-vs-Quod numbers compare two hosts and are reported only for the
record.

## The demo repository and its workflows

- `pango07/quod-demo-moto` (private). Each commit is upstream's tree at the
  commit named in its message (`Upstream: getmoto/moto@<sha>`), with upstream's
  `.github/workflows/` replaced by this benchmark's three workflows,
  `.github/dependabot.yml` removed, and a `QUOD_DEMO.md` that credits upstream.
  `LICENSE` is upstream's. Upstream authors and dates are kept.
- Upstream runs moto's unit tests as one job (`make test-only`): five services
  with xdist (batch, dynamodb, ec2, s3, sqs) and every other test directory
  serially. Here the same tests run as 11 jobs: the five xdist services each
  in its own job, with upstream's flags (`-n 4 --dist loadscope`,
  `MOTO_CALL_RESET_API=false`, clean-slate tests in a separate serial run),
  and the ~170 other test directories in six alphabetical shards of roughly
  equal duration (`tests/test_[ab]*`, `test_c*`, `test_[de]*`, `test_[f-m]*`,
  `test_[n-r]*`, `test_[s-z]*`). That is the shape of the story: one service
  changes, and only the jobs whose tests read it should run.
- Every job: `actions/checkout` (pinned v6), a summary heading,
  `python -m venv .venv && .venv/bin/pip install -e ".[all,server]" -r requirements-tests.txt`
  with a persistent pip cache (`$RUNNER_TOOL_CACHE/pip`), then pytest with
  `-sv -rs` as the Makefile runs it, minus coverage (`--cov` writes a
  timestamped data file and slows the tests; it is reporting, not testing).
  Job container `ghcr.io/astral-sh/uv@sha256:101820b9...` (python:3.12-trixie
  with uv 0.13.6), Python 3.12 only (upstream also runs 3.10-3.14).
  `TESTS_SKIP_REQUIRES_DOCKER=true`, upstream's own switch for running without
  Docker (the hooks do not support service or Docker-in-job containers).
- `quod.yml` runs on every push to `main` (runner label `quod`);
  `plain.yml` is dispatched on `main` right after the quod run finishes (label
  `plain`); `hosted.yml` (GitHub's `ubuntu-latest`, same container) was
  dispatched once for reference. The three files differ only in name, trigger
  and `runs-on`.
- The driver (`0d-moto-gha/scripts/driver.py`) pushed one commit at a time, waited for
  the quod run, dispatched plain, waited, and saved both runs, all jobs and all
  job logs (`0d-moto-gha/before/NN/`, `after/NN/`). Only one runner had a job at any moment.
  The server's heavy lock (`flock /work/heavy.lock`) was held throughout.

### Runners

Two official actions/runner 2.338.2 installs on the same host, as systemd
services, registered to the demo repository only (tokens from
`gh api -X POST .../registration-token`, passed over ssh stdin, never written
to disk; deregistered at the end):

- `plain` (`/opt/gha-plain`): no hooks; job containers through Docker
  (docker.io 29.1.3, installed for this).
- `quod` (`/opt/gha-quod`): `ACTIONS_RUNNER_CONTAINER_HOOKS` pointing at
  `scripts/runner/hooks/index.js` with its `quod.json`, as
  `scripts/runner/install.sh` writes them (under `/opt/quod-gha/hooks`, since
  `/opt/quod` is another bench's checkout on this box), state in
  `/var/lib/quod-gha`, `ACTIONS_RUNNER_REQUIRE_JOB_CONTAINER=true`. Policy:
  `{"mode": "replay", "proofs": 1, "audit": 0.05, "net_steps": [the defaults, plus "uv sync"]}`.
  Pushes to `main` are writer jobs (trusted scope): they persist the
  workspace's `.quod/tool` (the pip cache) and record into the scope's ledger.

Why `proofs: 1` on main and not a combined exp/trust-v2 + exp/never-worse
build: both branches had a failing full gate (dispatch runs on 112e269 and
93324a0), so the benchmark uses main, with the hooks' own `proofs` setting
standing in for trust v2's "replay after the first record" (without its
shadow audits).

Both runners use the same uv/pip cache location, image and steps; plain's
pip cache lives in its `_work/_tool`, Quod's in the persisted `.quod/tool`.
Commit 1 starts both from empty caches and is the warm-up: it is excluded
from totals and medians.

## Screenshots: the run pages to use

All in `pango07/quod-demo-moto` (private). Each Quod job's summary section
has a line per step saying whether it replayed or ran, and why. A replay line
reads like this: `[replayed] ...: saved 1m41s (recorded 1m41s, replay 0.0s, 10,758 inputs matched; trust: proven by 1 identical execution, ...)`.
The job's log shows the receipt (`QUOD  REPLAYED`, proved by run ... on
<time>, step and manifest ids, recorded / replay / saved).

| What it shows | plain | Quod |
|---|---|---|
| One service changed (`moto/emr`, commit 3): 7 of 11 jobs replay, d-e runs | [41447127613](https://github.com/pango07/quod-demo-moto/actions/runs/41447127613) (19.4 min) | [41444579043](https://github.com/pango07/quod-demo-moto/actions/runs/41444579043) (18.4 min) |
| No moto code changed (`other_langs`, commit 9): 8 of 11 replay | [41482826083](https://github.com/pango07/quod-demo-moto/actions/runs/41482826083) (20.2 min) | [41480184008](https://github.com/pango07/quod-demo-moto/actions/runs/41480184008) (15.7 min) |
| Shared data changed (`moto/ec2` resources, commit 5): nothing replays, Quod is slower | [41458934067](https://github.com/pango07/quod-demo-moto/actions/runs/41458934067) (20.1 min) | [41455487198](https://github.com/pango07/quod-demo-moto/actions/runs/41455487198) (23.1 min) |

For a single job side by side: `services s-z` on commit 9 (plain 128 s; Quod
about 41 s with `saved 1m41s`), or `services d-e` (153 s against 37 s). Honest
framing for a post: the whole run is 0.78-0.84x on these commits, not 3x,
because three network-touching jobs and the install step are not replayed.
The per-commit table below has every run's URLs.

## Before run (Quod 72e3b59), commits 1-10

`tables-before.md` holds the same tables with the run links; `before/NN/`
holds every run's JSON, job JSON and job log (gzip); `before/rows.jsonl` holds
one row per commit and job (`scripts/analyze.py`). "ran twice" means the test
step ran offline, failed or reached for the network, and ran again with it.
"results equal" compares each job's pytest summary lines, with durations
removed, to plain's on the same commit.

#### Per commit

| # | upstream | subject | files touched | plain | quod | quod/plain | jobs replayed | ran | ran twice (network) | results equal | runs |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | `6bae955b` | Core: skip CloudWatch protocol detection tests on older boto | tests/test_cloudwatch, tests/test_core | 19.9 min | 24.3 min | 1.22 | 0/11 | 8 | 3 | 11/11 | [plain](https://github.com/pango07/quod-demo-moto/actions/runs/41437127676) [quod](https://github.com/pango07/quod-demo-moto/actions/runs/41433744480) |
| 2 | `dbe25569` | Core: Add CBOR test for the Java SDK | other_langs | 19.8 min | 16.0 min | 0.81 | 8/11 | 0 | 3 | 11/11 | [plain](https://github.com/pango07/quod-demo-moto/actions/runs/41441823920) [quod](https://github.com/pango07/quod-demo-moto/actions/runs/41439739951) |
| 3 | `43907e80` | chore: update EMR Instance Types (#10511) | moto/emr | 19.4 min | 18.4 min | 0.94 | 7/11 | 1 | 3 | 11/11 | [plain](https://github.com/pango07/quod-demo-moto/actions/runs/41447127613) [quod](https://github.com/pango07/quod-demo-moto/actions/runs/41444579043) |
| 4 | `341c6281` | chore: update Config Managed Rules (#10512) | moto/config | 19.6 min | 19.2 min | 0.98 | 6/11 | 2 | 3 | 11/11 | [plain](https://github.com/pango07/quod-demo-moto/actions/runs/41452600178) [quod](https://github.com/pango07/quod-demo-moto/actions/runs/41449788357) |
| 5 | `ad9f5fb7` | chore: update EC2 Instance Offerings (#10513) | moto/ec2 | 20.1 min | 23.1 min | 1.15 | 0/11 | 8 | 3 | 11/11 | [plain](https://github.com/pango07/quod-demo-moto/actions/runs/41458934067) [quod](https://github.com/pango07/quod-demo-moto/actions/runs/41455487198) |
| 6 | `c9d08885` | Bump github/codeql-action from 4.38.2 to 4.39.1 (#10520) | .github | 19.6 min | 16.5 min | 0.84 | 8/11 | 0 | 3 | 11/11 | [plain](https://github.com/pango07/quod-demo-moto/actions/runs/41464860802) [quod](https://github.com/pango07/quod-demo-moto/actions/runs/41462182841) |
| 7 | `a088f77b` | Bump actions/setup-java from 6.0.1 to 6.1.0 (#10521) | .github | 19.9 min | 16.4 min | 0.83 | 8/11 | 0 | 3 | 11/11 | [plain](https://github.com/pango07/quod-demo-moto/actions/runs/41470783318) [quod](https://github.com/pango07/quod-demo-moto/actions/runs/41468020960) |
| 8 | `bde8cd8d` | Bump ruby/setup-ruby from 1.327.0 to 1.335.0 (#10522) | .github | 19.8 min | 16.3 min | 0.82 | 8/11 | 0 | 3 | 11/11 | [plain](https://github.com/pango07/quod-demo-moto/actions/runs/41476851445) [quod](https://github.com/pango07/quod-demo-moto/actions/runs/41474101461) |
| 9 | `a2a22d5d` | Bump the go-deps group in /other_langs/tests_go with 8 updat | other_langs | 20.2 min | 15.7 min | 0.78 | 8/11 | 0 | 3 | 11/11 | [plain](https://github.com/pango07/quod-demo-moto/actions/runs/41482826083) [quod](https://github.com/pango07/quod-demo-moto/actions/runs/41480184008) |
| 10 | `4b1a9e50` | Bump the java-deps group in /other_langs/tests_java with 3 u | other_langs | 22.4 min | 17.7 min | 0.79 | 8/11 | 0 | 3 | 11/11 | [plain](https://github.com/pango07/quod-demo-moto/actions/runs/41491045454) [quod](https://github.com/pango07/quod-demo-moto/actions/runs/41486322014) |

#### Which jobs replayed and which ran, per commit

| # | ran | ran twice (network) | replayed |
|---|---|---|---|
| 1 | batch, dynamodb, ec2, a-b, d-e, n-r, s-z, sqs | s3, c, f-m | - |
| 2 | - | s3, c, f-m | batch, dynamodb, ec2, a-b, d-e, n-r, s-z, sqs |
| 3 | d-e | s3, c, f-m | batch, dynamodb, ec2, a-b, n-r, s-z, sqs |
| 4 | d-e, s-z | s3, c, f-m | batch, dynamodb, ec2, a-b, n-r, sqs |
| 5 | batch, dynamodb, ec2, a-b, d-e, n-r, s-z, sqs | s3, c, f-m | - |
| 6 | - | s3, c, f-m | batch, dynamodb, ec2, a-b, d-e, n-r, s-z, sqs |
| 7 | - | s3, c, f-m | batch, dynamodb, ec2, a-b, d-e, n-r, s-z, sqs |
| 8 | - | s3, c, f-m | batch, dynamodb, ec2, a-b, d-e, n-r, s-z, sqs |
| 9 | - | s3, c, f-m | batch, dynamodb, ec2, a-b, d-e, n-r, s-z, sqs |
| 10 | - | s3, c, f-m | batch, dynamodb, ec2, a-b, d-e, n-r, s-z, sqs |

#### Totals, commits 2-10 (9 commits)

| | plain | quod | quod/plain |
|---|---|---|---|
| sum of run wall times | 180.7 min | 159.2 min | 0.88 |
| median run wall time | 19.8 min | 16.5 min | 0.83 |
| sum of job times (runner minutes) | 177.0 min | 151.0 min | 0.85 |
| commits where quod was faster | | 8 of 9 | |

Jobs: 99; test step replayed 61, ran 11, ran twice (offline, then with the network) 27. pytest summaries equal to plain's on the same commit: 99/99.

#### Per job (commits 2+)

| job | plain median | quod median | quod when replayed (median) | quod when ran (median) | replayed | ran | ran twice |
|---|---|---|---|---|---|---|---|
| batch | 34 s | 37 s | 36 s | 47 s | 8 | 1 | 0 |
| dynamodb | 41 s | 39 s | 38 s | 55 s | 8 | 1 | 0 |
| ec2 | 83 s | 39 s | 39 s | 93 s | 8 | 1 | 0 |
| s3 | 48 s | 83 s | - | 83 s | 0 | 0 | 9 |
| services a-b | 84 s | 36 s | 36 s | 98 s | 8 | 1 | 0 |
| services c | 337 s | 282 s | - | 282 s | 0 | 0 | 9 |
| services d-e | 153 s | 41 s | 37 s | 166 s | 6 | 3 | 0 |
| services f-m | 113 s | 219 s | - | 219 s | 0 | 0 | 9 |
| services n-r | 85 s | 39 s | 38 s | 110 s | 8 | 1 | 0 |
| services s-z | 128 s | 42 s | 41 s | 144 s | 7 | 2 | 0 |
| sqs | 51 s | 38 s | 38 s | 67 s | 8 | 1 | 0 |

#### Quod against a hand-written path filter (commits 2+)

Jobs whose tests Quod could replay at all (not network-tainted): agree 63, Quod ran and a path filter would have skipped 9, a path filter ran and Quod replayed 0.
- commit 4, services d-e: 2m08s; would replay: no, closest manifest differed at moto/config/resources/aws_managed_rules.json [file content changed (file b3:bbccca65 m
- commit 4, services s-z: 1m47s; would replay: no, closest manifest differed at moto/config/resources/aws_managed_rules.json [file content changed (file b3:bbccca65 m
- commit 5, batch: 10.1s; would replay: no, closest manifest differed at moto/ec2/resources/instance_type_offerings/availability-zone-id/af-south-1.json [file 
- commit 5, dynamodb: 17.6s; would replay: no, closest manifest differed at moto/ec2/resources/instance_type_offerings/availability-zone-id/af-south-1.json [file 
- commit 5, services a-b: 1m00s; would replay: no, closest manifest differed at moto/ec2/resources/instance_type_offerings/availability-zone-id/af-south-1.json [file 
- commit 5, services d-e: 2m07s; would replay: no, closest manifest differed at moto/ec2/resources/instance_type_offerings/availability-zone-id/af-south-1.json [file 
- commit 5, services n-r: 1m00s; would replay: no, closest manifest differed at moto/ec2/resources/instance_type_offerings/availability-zone-id/af-south-1.json [file 
- commit 5, services s-z: 1m45s; would replay: no, closest manifest differed at moto/ec2/resources/instance_type_offerings/availability-zone-id/af-south-1.json [file 
- commit 5, sqs: 27.4s; would replay: no, closest manifest differed at moto/ec2/resources/instance_type_offerings/availability-zone-id/af-south-1.json [file 


## After run (main 5a4c18b), commits 1-2, stopped

Default policy (`{"mode": "replay"}`: proofs 1, shadow 2, audit 5%). The
workflow has no workarounds left (`workflows-after/`). Plain ran on a second
CCX23 (quod-bench-4) in parallel. That host was about 20% faster for the same
plain workflow, so compare Quod's own before/after, not these two columns.
Here "ran twice (network)" counts every network-tainted test step: in this
run those steps ran **once** (the network memory), straight on the runner's
surface, except on commit 1, which had no memory yet.

#### Per commit

| # | upstream | subject | files touched | plain | quod | quod/plain | jobs replayed | ran | ran twice (network) | results equal | runs |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | `6bae955b` | Core: skip CloudWatch protocol detection tests on older boto | tests/test_cloudwatch, tests/test_core | 16.1 min | 23.4 min | 1.46 | 0/11 | 8 | 3 | 10/11 | [plain](https://github.com/pango07/quod-demo-moto/actions/runs/41499548026) [quod](https://github.com/pango07/quod-demo-moto/actions/runs/41499531937) |
| 2 | `dbe25569` | Core: Add CBOR test for the Java SDK | other_langs | 16.6 min | 20.0 min | 1.20 | 0/11 | 7 | 4 | 11/11 | [plain](https://github.com/pango07/quod-demo-moto/actions/runs/41503362109) [quod](https://github.com/pango07/quod-demo-moto/actions/runs/41503343681) |

#### Which jobs replayed and which ran, per commit

| # | ran | ran twice (network) | replayed |
|---|---|---|---|
| 1 | batch, dynamodb, ec2, a-b, d-e, n-r, s-z, sqs | s3, c, f-m | - |
| 2 | batch, dynamodb, ec2, a-b, d-e, n-r, sqs | s3, c, f-m, s-z | - |



| | before (72e3b59) | after (5a4c18b) |
|---|---|---|
| network-touching test steps (c, f-m, s3) | offline, then again with the network, every commit | commit 1 twice; then once, without the offline try |
| `pip install` | offline try (~10 s) then online, every job | the same: its inputs (the pip cache) differ every job, so the network memory never matches |
| xdist jobs (batch, dynamodb, ec2, s3, sqs) | replayed whenever their inputs matched | the first shadow audit diverged on test-line order (`-n 4`), key quarantined; would never replay |
| serial shards | replayed from the 2nd commit | shadow audits agreed; replays would start at the 4th commit (2 audited hits first) |
| fixed cost of a replayed job | 32-36 s (hooks' clock) | not measured (no replay in 2 commits) |
| workflow workarounds needed | 5 (GPG_KEY, digest-only image, pyc prefix, install pyc aside, heading after checkout) | none |

## Why moto, and what was tried first

1. **LangChain** (`langchain-ai/langchain`, MIT, a uv monorepo with one test
   job per package: the first choice). On 72e3b59 no test step could ever
   replay: `langsmith`, which every package imports through `langchain-core`,
   names `GITHUB_SHA` in `langsmith/env/_runtime_env.py`, and the hooks' scan
   then ran any step that read it again on the runner's surface, never
   recorded. The same scan blocked vitest (its `cli-api` chunk names
   `GITHUB_SHA`) and jest (`ci-info`'s `vendors.json` names
   `GITHUB_EVENT_NAME`), and with them most Node monorepos (checked by
   scanning installed trees with `remote/volscan.py`). Job logs from the probe
   run: `langchain-probe/`. The scan was redesigned on main (DECISIONS
   2026-11-02, exp/py-node-ready finding 1); LangChain was not re-run.
2. **opentelemetry-python** was considered: a real per-package monorepo, but
   its suites take 10-30 s, so the per-job fixed cost would dominate, and its
   `tox-uv` puts a `uv` binary in the tree that names `ACTIONS_ID_TOKEN_*`.
3. **moto** (`getmoto/moto`, Apache-2.0, 8.7k stars): its test-time
   dependencies name no withheld variable, its tests take minutes, and its
   commits are mostly one service each. It is one package, not a monorepo;
   splitting its single test job by service is this benchmark's choice,
   disclosed above. Its services are less independent than packages are:
   every test imports every service's `urls`/`responses`/`models`/`exceptions`
   (`moto-coupling-networkfirewall.txt`: a dynamodb test reads
   `moto/networkfirewall/*.py`), so a change to a service's models misses
   every job. Commits that change a service's data files (the EMR instance
   types, the Config rules) or its tests only, as most commits in this window
   do, miss only the jobs that read them.

The window is the 15 newest first-parent commits of moto master when the
final run started (`6bae955b`..`3a9a6f30`), by that rule alone. It was cut
from 30 to 15 for time (each commit pair took ~40 min sequentially). The
before run measured commits 1-10; the product owner stopped it at commit 11
to re-run on main.

## Methodology notes and caveats

- **Setup attempts.** Five earlier attempts at the before run were thrown
  away: their Quod state was wiped and their runs deleted from the demo repo.
  Each was stopped by a product finding that the next attempt worked around in
  the workflow (DECISIONS lists them; all are fixed on main): the image
  reference form; `GPG_KEY` classified as a secret; stdlib `.pyc` written
  under `/usr/local`; install-time `.pyc` stamped with install mtimes (twice:
  `PYTHONDONTWRITEBYTECODE` does not reach pip's build subprocesses, and the
  first prefix was still read by the tests); a step before checkout making
  the first job of each commit opaque. The before run's workflow is in
  `workflows-before/`.
- **Python 3.12 only**, no coverage, no Docker-requiring tests (upstream's
  `TESTS_SKIP_REQUIRES_DOCKER`), `pip install -e ".[all,server]" -r requirements-tests.txt`
  (upstream installs `requirements-dev.txt`, which adds the lint tools).
- **Sequential, one host** (before run): only one runner had a job at a time,
  driven by `scripts/driver.py`, under the box's heavy lock. Plain's run wall
  times were stable at 19.4-20.2 min, except commit 10 (22.4 min, dispatched
  right after a network outage on the driver's side).
- **The `proofs: 1` policy** (before run) is the hooks' own setting standing
  in for trust v2 without its shadow audits. exp/trust-v2 and exp/never-worse
  could not be combined: both had failing full gates.
- **Run wall time** is GitHub's `run_started_at` to `updated_at`. Job times
  are each job's `started_at` to `completed_at`.
- **Not measured:** a re-run of the same commit (would replay 8 of 11 under
  the before policy, like a no-code commit); pull requests (non-writer
  scopes); more than one Python version.
- **Disk:** Quod's state reached about 6 GB (CAS, rootfs) for 10 commits.

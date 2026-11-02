## history v2

mode `history`, policy `(defaults)`, 40 commits, 40 jobs, 120 keyed step runs, quod `quod 0.0.0`

| step | phase | runs | miss | replayed | audit | proving | diverged | other | replayed / unchanged-input runs | wall total | wall of replays | wall of executions |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `go build ./...` | merge | 40 | 40 | **0** | 0 | 0 | 0 | 0 | 0/0 | 336s | 0s | 336s |
| `go build ./...` | all | 40 | 40 | **0** | 0 | 0 | 0 | 0 | 0/0 | 336s | 0s | 336s |
| `go vet ./...` | merge | 40 | 32 | **4** | 4 | 0 | 0 | 0 | 4/8 (50%) | 244s | 8s | 236s |
| `go vet ./...` | all | 40 | 32 | **4** | 4 | 0 | 0 | 0 | 4/8 (50%) | 244s | 8s | 236s |
| `go test ./...` | merge | 40 | 34 | **3** | 3 | 0 | 0 | 0 | 3/6 (50%) | 1815s | 6s | 1810s |
| `go test ./...` | all | 40 | 34 | **3** | 3 | 0 | 0 | 0 | 3/6 (50%) | 1815s | 6s | 1810s |

All keyed steps: 120 runs, 106 miss, **7 replayed**, 7 audited, 0 proving, 0 diverged; replayed 7 of 14 unchanged-input runs (50%); wall 2395s.
Jobs: 40, total wall 2427s (40.5 min), median 53.5s.

`go test` misses by first changed input (top-level path): `pkg` 9, `internal` 7, `.quod` 6, `api` 4, `.github` 2, `git` 2, `-` 1, `go.mod` 1

Ledger: miss 106; net 40; replayed 7; audit 7

| key | cmd | state | eligible | replays | audits | divergences | volatile | shadow_from |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `b3:f19dd29a7` | `go build ./...` | proven | 0 | 0 | 0 | 0 | 0 | 0 |
| `b3:d75210952` | `go test ./...` | proven | 6 | 3 | 3 | 0 | 3 | 0 |
| `b3:a839227fe` | `go vet ./...` | proven | 8 | 4 | 4 | 0 | 0 | 0 |

Audits: audit ok 3; shadow ok 4
Suspect replays: 0

## traffic v2

mode `traffic`, policy `(defaults)`, 40 commits, 87 jobs, 261 keyed step runs, quod `quod 0.0.0`

| step | phase | runs | miss | replayed | audit | proving | diverged | other | replayed / unchanged-input runs | wall total | wall of replays | wall of executions |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `go build ./...` | pr | 40 | 40 | **0** | 0 | 0 | 0 | 0 | 0/0 | 365s | 0s | 365s |
| `go build ./...` | merge | 40 | 40 | **0** | 0 | 0 | 0 | 0 | 0/0 | 223s | 0s | 223s |
| `go build ./...` | rerun | 7 | 0 | **3** | 4 | 0 | 0 | 0 | 3/7 (43%) | 26s | 7s | 19s |
| `go build ./...` | all | 87 | 80 | **3** | 4 | 0 | 0 | 0 | 3/7 (43%) | 615s | 7s | 608s |
| `go vet ./...` | pr | 40 | 32 | **7** | 1 | 0 | 0 | 0 | 7/8 (88%) | 249s | 14s | 235s |
| `go vet ./...` | merge | 40 | 6 | **28** | 6 | 0 | 0 | 0 | 28/34 (82%) | 106s | 50s | 56s |
| `go vet ./...` | rerun | 7 | 0 | **6** | 1 | 0 | 0 | 0 | 6/7 (86%) | 14s | 10s | 4s |
| `go vet ./...` | all | 87 | 38 | **41** | 8 | 0 | 0 | 0 | 41/49 (84%) | 369s | 74s | 295s |
| `go test ./...` | pr | 40 | 35 | **3** | 2 | 0 | 0 | 0 | 3/5 (60%) | 2407s | 7s | 2400s |
| `go test ./...` | merge | 40 | 34 | **4** | 2 | 0 | 0 | 0 | 4/6 (67%) | 1557s | 9s | 1548s |
| `go test ./...` | rerun | 7 | 0 | **6** | 1 | 0 | 0 | 0 | 6/7 (86%) | 54s | 12s | 42s |
| `go test ./...` | all | 87 | 69 | **13** | 5 | 0 | 0 | 0 | 13/18 (72%) | 4018s | 28s | 3990s |

All keyed steps: 261 runs, 187 miss, **57 replayed**, 17 audited, 0 proving, 0 diverged; replayed 57 of 74 unchanged-input runs (77%); wall 5002s.
Jobs: 87, total wall 5068s (84.5 min), median 51.5s.

`go test` misses by first changed input (top-level path): `.git` 66, `-` 1, `.quod` 1, `pkg` 1

Ledger: miss 187; net 87; replayed 57; audit 17

| key | cmd | state | eligible | replays | audits | divergences | volatile | shadow_from |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `b3:f19dd29a7` | `go build ./...` | proven | 7 | 3 | 4 | 0 | 0 | 0 |
| `b3:d75210952` | `go test ./...` | proven | 18 | 13 | 5 | 0 | 5 | 0 |
| `b3:a839227fe` | `go vet ./...` | proven | 49 | 41 | 8 | 0 | 0 | 0 |

Audits: audit ok 11; shadow ok 6
Suspect replays: 0

## traffic v1

mode `traffic`, policy `--proofs 2 --shadow 0`, 40 commits, 87 jobs, 261 keyed step runs, quod `quod 0.0.0`

| step | phase | runs | miss | replayed | audit | proving | diverged | other | replayed / unchanged-input runs | wall total | wall of replays | wall of executions |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `go build ./...` | pr | 40 | 40 | **0** | 0 | 0 | 0 | 0 | 0/0 | 364s | 0s | 364s |
| `go build ./...` | merge | 40 | 40 | **0** | 0 | 0 | 0 | 0 | 0/0 | 222s | 0s | 222s |
| `go build ./...` | rerun | 7 | 0 | **0** | 0 | 7 | 0 | 0 | 0/7 (0%) | 35s | 0s | 35s |
| `go build ./...` | all | 87 | 80 | **0** | 0 | 7 | 0 | 0 | 0/7 (0%) | 620s | 0s | 620s |
| `go vet ./...` | pr | 40 | 32 | **6** | 1 | 1 | 0 | 0 | 6/8 (75%) | 249s | 11s | 238s |
| `go vet ./...` | merge | 40 | 6 | **7** | 0 | 27 | 0 | 0 | 7/34 (21%) | 155s | 13s | 143s |
| `go vet ./...` | rerun | 7 | 0 | **7** | 0 | 0 | 0 | 0 | 7/7 (100%) | 12s | 12s | 0s |
| `go vet ./...` | all | 87 | 38 | **20** | 1 | 28 | 0 | 0 | 20/49 (41%) | 416s | 35s | 381s |
| `go test ./...` | pr | 40 | 35 | **0** | 0 | 5 | 0 | 0 | 0/5 (0%) | 1979s | 0s | 1979s |
| `go test ./...` | merge | 40 | 34 | **0** | 0 | 6 | 0 | 0 | 0/6 (0%) | 1720s | 0s | 1720s |
| `go test ./...` | rerun | 7 | 0 | **1** | 0 | 6 | 0 | 0 | 1/7 (14%) | 254s | 1s | 252s |
| `go test ./...` | all | 87 | 69 | **1** | 0 | 17 | 0 | 0 | 1/18 (6%) | 3952s | 1s | 3951s |

All keyed steps: 261 runs, 187 miss, **21 replayed**, 1 audited, 52 proving, 0 diverged; replayed 21 of 74 unchanged-input runs (28%); wall 4989s.
Jobs: 87, total wall 5055s (84.2 min), median 53.2s.

`go test` misses by first changed input (top-level path): `.git` 68, `-` 1

Ledger: miss 187; net 87; proving 52; replayed 21; audit 1

| key | cmd | state | eligible | replays | audits | divergences | volatile | shadow_from |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `b3:a4a82eaec` | `go build ./...` | proven | 0 | 0 | 7 | 0 | 0 | 0 |
| `b3:aa88b0b5a` | `go test ./...` | proven | 1 | 1 | 17 | 0 | 17 | 0 |
| `b3:1368055b3` | `go vet ./...` | proven | 21 | 20 | 29 | 0 | 0 | 0 |

Audits: audit ok 1; proof ok 52
Suspect replays: 0

## traffic v1 (apt upgrade mid-run)

mode `traffic`, policy `--proofs 2 --shadow 0`, 40 commits, 87 jobs, 261 keyed step runs, quod `quod 0.0.0`

| step | phase | runs | miss | replayed | audit | proving | diverged | other | replayed / unchanged-input runs | wall total | wall of replays | wall of executions |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `go build ./...` | pr | 40 | 40 | **0** | 0 | 0 | 0 | 0 | 0/0 | 408s | 0s | 408s |
| `go build ./...` | merge | 40 | 40 | **0** | 0 | 0 | 0 | 0 | 0/0 | 208s | 0s | 208s |
| `go build ./...` | rerun | 7 | 1 | **0** | 0 | 6 | 0 | 0 | 0/6 (0%) | 69s | 0s | 69s |
| `go build ./...` | all | 87 | 81 | **0** | 0 | 6 | 0 | 0 | 0/6 (0%) | 685s | 0s | 685s |
| `go vet ./...` | pr | 40 | 34 | **5** | 0 | 1 | 0 | 0 | 5/6 (83%) | 242s | 7s | 235s |
| `go vet ./...` | merge | 40 | 7 | **7** | 0 | 26 | 0 | 0 | 7/33 (21%) | 147s | 11s | 136s |
| `go vet ./...` | rerun | 7 | 1 | **6** | 0 | 0 | 0 | 0 | 6/6 (100%) | 29s | 10s | 18s |
| `go vet ./...` | all | 87 | 42 | **18** | 0 | 27 | 0 | 0 | 18/45 (40%) | 418s | 28s | 390s |
| `go test ./...` | pr | 40 | 36 | **0** | 0 | 4 | 0 | 0 | 0/4 (0%) | 1942s | 0s | 1942s |
| `go test ./...` | merge | 40 | 34 | **0** | 0 | 6 | 0 | 0 | 0/6 (0%) | 1689s | 0s | 1689s |
| `go test ./...` | rerun | 7 | 1 | **1** | 0 | 5 | 0 | 0 | 1/6 (17%) | 301s | 1s | 299s |
| `go test ./...` | all | 87 | 71 | **1** | 0 | 15 | 0 | 0 | 1/16 (6%) | 3932s | 1s | 3930s |

All keyed steps: 261 runs, 194 miss, **19 replayed**, 0 audited, 48 proving, 0 diverged; replayed 19 of 67 unchanged-input runs (28%); wall 5035s.
Jobs: 87, total wall 5110s (85.2 min), median 52.7s.

`go test` misses by first changed input (top-level path): `.git` 67, `-` 3, `.` 1

Ledger: miss 194; net 87; proving 48; replayed 19

| key | cmd | state | eligible | replays | audits | divergences | volatile | shadow_from |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `b3:f19dd29a7` | `go build ./...` | proven | 0 | 0 | 6 | 0 | 0 | 0 |
| `b3:50b140785` | `go build ./...` | observing | 0 | 0 | 0 | 0 | 0 | 0 |
| `b3:560d233a7` | `go build ./...` | observing | 0 | 0 | 0 | 0 | 0 | 0 |
| `b3:d75210952` | `go test ./...` | proven | 1 | 1 | 12 | 0 | 12 | 0 |
| `b3:b23e3b89e` | `go test ./...` | observing | 0 | 0 | 0 | 0 | 0 | 0 |
| `b3:1f587673c` | `go test ./...` | proven | 0 | 0 | 3 | 0 | 3 | 0 |
| `b3:a839227fe` | `go vet ./...` | proven | 13 | 13 | 20 | 0 | 0 | 0 |
| `b3:902396ed7` | `go vet ./...` | observing | 0 | 0 | 0 | 0 | 0 | 0 |
| `b3:660f09394` | `go vet ./...` | proven | 5 | 5 | 7 | 0 | 0 | 0 |

Audits: proof ok 48
Suspect replays: 0

## plain warm

mode `plain`, policy `(defaults)`, 40 commits, 40 jobs, 120 keyed step runs, quod ``

| step | phase | runs | miss | replayed | audit | proving | diverged | other | replayed / unchanged-input runs | wall total | wall of replays | wall of executions |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `go build ./...` | plain | 40 | 0 | **0** | 0 | 0 | 0 | 40 | 0/40 (0%) | 217s | 0s | 217s |
| `go build ./...` | all | 40 | 0 | **0** | 0 | 0 | 0 | 40 | 0/40 (0%) | 217s | 0s | 217s |
| `go vet ./...` | plain | 40 | 0 | **0** | 0 | 0 | 0 | 40 | 0/40 (0%) | 127s | 0s | 127s |
| `go vet ./...` | all | 40 | 0 | **0** | 0 | 0 | 0 | 40 | 0/40 (0%) | 127s | 0s | 127s |
| `go test ./...` | plain | 40 | 0 | **0** | 0 | 0 | 0 | 40 | 0/40 (0%) | 1040s | 0s | 1040s |
| `go test ./...` | all | 40 | 0 | **0** | 0 | 0 | 0 | 40 | 0/40 (0%) | 1040s | 0s | 1040s |

All keyed steps: 120 runs, 0 miss, **0 replayed**, 0 audited, 0 proving, 0 diverged; replayed 0 of 120 unchanged-input runs (0%); wall 1384s.
Jobs: 40, total wall 1395s (23.3 min), median 19.6s.


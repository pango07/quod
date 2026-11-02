# Per-process upper bound on the 40-commit cli/cli history (key: ckey)

Every process the Go command ran (compile, asm, vet, link, each package's test binary) on every commit, under Quod's warm-miss conditions (persistent GOCACHE, `-count=1`, `goindex=0`, fresh HOME, fresh checkout), each keyed on exactly what it read. A **hit** is a process whose key was produced on an earlier commit; **proven** needs two earlier commits (the shipped trust rule). Commit 1 is excluded from totals as in the bench. The wall estimate removes a hit's wall time divided by the step's measured parallelism (process-seconds per wall second), i.e. assumes replay is free and the parent's critical path shrinks proportionally; replay cost per process in this run: 0 ms.

## Totals, commits 2-40

| Step | Kind | Processes | Serial wall | Hit (key seen before) | Hit wall | Proven (seen twice) | Proven wall |
| --- | --- | --- | --- | --- | --- | --- | --- |
| go build ./... | compile | 2871 | 316 s | 0 (0%) | 0 s (0%) | 0 | 0 s |
| go build ./... | link | 156 | 146 s | 0 (0%) | 0 s (0%) | 0 | 0 s |
| go vet ./... | compile | 42 | 9 s | 0 (0%) | 0 s (0%) | 0 | 0 s |
| go vet ./... | vet | 5104 | 282 s | 0 (0%) | 0 s (0%) | 0 | 0 s |
| go test ./... | compile | 4654 | 491 s | 0 (0%) | 0 s (0%) | 0 | 0 s |
| go test ./... | vet | 5105 | 276 s | 0 (0%) | 0 s (0%) | 0 | 0 s |
| go test ./... | link | 10156 | 4848 s | 5915 (58%) | 2617 s (54%) | 4932 | 2115 s |
| go test ./... | test | 10082 | 888 s | 7698 (76%) | 657 s (74%) | 5895 | 496 s |

## Step wall: measured, and estimated with hits replayed free

| Step | Instrumented wall (sum) | Bench plain-warm (sum) | Bench Quod miss (sum) | Est. with hits free | Est. with proven free |
| --- | --- | --- | --- | --- | --- |
| go build ./... | 213 s | 170 s | 282 s | 213 s (0% less) | 213 s (0% less) |
| go vet ./... | 146 s | 108 s | 226 s | 146 s (0% less) | 146 s (0% less) |
| go test ./... | 2315 s | 943 s | 1804 s | 1140 s (51% less) | 1378 s (40% less) |

## `go test ./...` units against Go's own test cache (commits 2-40)

| Class | Units | Test-binary wall (serial) |
| --- | --- | --- |
| memo-hit/go-cached | 6616 | 514.6 s |
| memo-hit/go-ran | 1082 | 142.4 s |
| memo-miss/go-cached | 241 | 23.0 s |
| memo-miss/go-ran | 2143 | 207.5 s |

`memo-hit/go-ran`: what a content-keyed process memo skips that Go's cache re-ran (the win over Go). `memo-miss/go-cached`: what Go skipped but the content key did not match (a key leak or Go's own unsoundness).

Packages Go cached but the memo missed (count over commits): `internal/docs` x13, `internal/ghcmd` x11, `internal/codespaces` x6, `internal/codespaces/connection` x6, `internal/codespaces/portforwarder` x6, `internal/codespaces/rpc` x6, `pkg/cmd/project/shared/format` x6, `pkg/cmd/secret/shared` x6, `pkg/cmd/skills/list` x6, `pkg/cmd/completion` x5, `pkg/cmd/config/clear-cache` x5, `pkg/cmd/version` x5, `pkg/cmd/auth/shared/gitcredentials` x5, `pkg/cmd/discussion/shared` x4, `context` x4

Packages the memo hit while Go re-ran them (count, serial ms over commits): `internal/safepaths` x39 (0.6 s), `acceptance` x38 (0.9 s), `pkg/cmd/run/download` x31 (0.3 s), `pkg/cmd/attestation/artifact` x30 (0.4 s), `pkg/cmd/attestation/verification` x30 (0.6 s), `pkg/cmd/repo/read-file` x30 (0.4 s), `pkg/cmd/ruleset/check` x30 (0.3 s), `pkg/cmd/ruleset/list` x30 (0.2 s), `pkg/cmd/attestation/download` x29 (0.7 s), `pkg/cmd/release/download` x29 (1.1 s), `pkg/cmd/release/verify-asset` x29 (0.9 s), `pkg/cmd/repo/create` x29 (88.0 s)

Why test units missed when the same package ran on the previous commit: binary: 1592, argv: 790, files:/work/pm/hist/cli/acceptance/testdata/pr/pr-checkout-with-url-from-fork.txtar,/work/pm/hist/cli/acceptance/testdata/pr/p: 1

Test binaries that exec a subprocess (strace survey at the last commit): 11: `git`, `internal/prompter`, `internal/skills/installer`, `pkg/cmd/auth/shared/gitcredentials`, `pkg/cmd/codespace`, `pkg/cmd/extension`, `pkg/cmd/issue/create`, `pkg/cmd/pr/merge`, `pkg/cmd/repo/sync`, `pkg/cmd/skills/search`, `pkg/surveyext`

Test binaries whose testlog stats an executable path: 12.

Failing or killed test units: 0

## Key stability: the last commit run twice on the same tree

| Step | Processes in the second run | Same key as the first | Unstable |
| --- | --- | --- | --- |
| go build ./... | 4 | 0 | 4 link /tmp/go-build/b1118/exe/a.out, link /tmp/go-build/b1125/exe/a.out, link /tmp/go-build/b1109/exe/a.out, link /tmp/go-build/b251/exe/a.out |
| go vet ./... | 0 | 0 | 0  |
| go test ./... | 518 | 517 | 1 link /tmp/go-build/b1127/git.test |


## Per commit

| # | Commit | Subject | build wall / hit% / est | vet wall / hit% / est | test wall / hit% / est | test: link hits | test: unit hits | Go cached | plain-warm test | Quod test |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | `b1c8c0ae5a` | Merge pull request #14750 from cli/williammar | 44.8s / 0% / 44.8s | 19.7s / 0% / 19.7s | 86.8s / 0% / 86.8s | 0/260 | 0/258 | 0/258 | 66.6s | 69.6s |
| 2 | `5b0cc54248` | chore(deps): bump the codeql-actions group wi | 2.7s / 0% / 2.7s | 0.9s / 0% / 0.9s | 50.3s / 16% / 42.4s | 0/260 | 258/258 | 220/258 | 9.6s | 39.8s |
| 3 | `ed40e856ff` | Merge pull request #14719 from cli/bagtoad/fi | 2.8s / 0% / 2.8s | 1.1s / 0% / 1.1s | 50.5s / 94% / 3.0s | 251/260 | 251/258 | 216/258 | 11.0s | 40.1s |
| 4 | `5cfe30624f` | Bump Go to 1.27.3 (#14767) | 2.7s / 0% / 2.7s | 1.0s / 0% / 1.0s | 50.7s / 99% / 0.6s | 255/260 | 258/258 | 220/258 | 10.3s | 39.8s |
| 5 | `ad27e286ab` | Merge pull request #14770 from cli/bagtoad/me | 2.8s / 0% / 2.8s | 1.0s / 0% / 1.0s | 50.6s / 100% / 0.1s | 258/260 | 258/258 | 220/258 | 10.3s | 40.1s |
| 6 | `401854ece8` | chore(deps): bump github.com/gabriel-vasile/m | 3.5s / 0% / 3.5s | 1.4s / 0% / 1.4s | 51.4s / 89% / 5.6s | 240/260 | 240/258 | 206/258 | 12.9s | 41.0s |
| 7 | `1531837d85` | Merge pull request #14148 from heaths/issue37 | 12.5s / 0% / 12.5s | 10.9s / 0% / 10.9s | 77.3s / 2% / 75.9s | 17/260 | 17/258 | 15/258 | 59.3s | 62.6s |
| 8 | `ad4a6456e1` | Merge pull request #14450 from cli/babakks/re | 11.7s / 0% / 11.7s | 10.3s / 0% / 10.3s | 76.7s / 3% / 74.1s | 16/260 | 29/258 | 38/258 | 56.5s | 61.9s |
| 9 | `4f168ce66d` | Merge pull request #14451 from cli/babakks/re | 20.5s / 0% / 20.5s | 17.3s / 0% / 17.3s | 108.8s / 4% / 104.5s | 29/260 | 31/258 | 66/258 | 56.0s | 61.8s |
| 10 | `a81663f7d8` | Merge pull request #14452 from cli/babakks/re | 3.5s / 0% / 3.5s | 1.9s / 0% / 1.9s | 52.5s / 19% / 42.6s | 31/260 | 241/258 | 208/258 | 16.2s | 41.9s |
| 11 | `b7887068cb` | Merge pull request #14453 from cli/babakks/re | 2.9s / 0% / 2.9s | 1.3s / 0% / 1.3s | 64.8s / 91% / 6.0s | 240/260 | 253/258 | 217/258 | 14.1s | 40.1s |
| 12 | `bd0b0e085f` | Merge pull request #14454 from cli/babakks/re | 3.0s / 0% / 3.0s | 1.4s / 0% / 1.4s | 58.4s / 94% / 3.2s | 252/260 | 253/258 | 217/258 | 14.5s | 40.4s |
| 13 | `d8474903cf` | Merge pull request #14455 from cli/babakks/re | 4.9s / 0% / 4.9s | 3.3s / 0% / 3.3s | 60.9s / 86% / 8.5s | 247/260 | 248/258 | 213/258 | 15.2s | 41.1s |
| 14 | `316e7a80fb` | Merge pull request #14456 from cli/babakks/re | 2.8s / 0% / 2.8s | 1.0s / 0% / 1.0s | 50.8s / 95% / 2.7s | 248/260 | 258/258 | 220/258 | 13.2s | 40.2s |
| 15 | `4a289a9e19` | Merge pull request #14663 from cli/babakks/re | 11.7s / 0% / 11.7s | 10.3s / 0% / 10.3s | 77.3s / 5% / 73.3s | 31/260 | 31/258 | 37/258 | 56.5s | 62.1s |
| 16 | `e62e84ca88` | Merge pull request #14665 from cli/babakks/re | 3.7s / 0% / 3.7s | 1.9s / 0% / 1.9s | 53.1s / 19% / 42.9s | 31/260 | 245/258 | 212/258 | 14.2s | 42.3s |
| 17 | `56682b6faf` | Merge pull request #14672 from cli/babakks/re | 11.5s / 0% / 11.5s | 10.1s / 0% / 10.1s | 76.2s / 5% / 72.2s | 31/260 | 31/258 | 72/258 | 55.6s | 62.0s |
| 18 | `abf0e0c25a` | Merge pull request #14673 from cli/babakks/re | 3.2s / 0% / 3.2s | 1.5s / 0% / 1.5s | 52.0s / 22% / 40.7s | 37/260 | 247/258 | 209/258 | 15.8s | 41.4s |
| 19 | `6314d776e1` | Merge pull request #14717 from cli/babakks/re | 11.6s / 0% / 11.6s | 10.1s / 0% / 10.1s | 76.5s / 5% / 72.6s | 31/260 | 31/258 | 73/258 | 55.2s | 61.9s |
| 20 | `547bf81aa5` | Merge pull request #14745 from cli/babakks/re | 7.4s / 0% / 7.4s | 6.1s / 0% / 6.1s | 64.5s / 13% / 56.0s | 34/260 | 138/258 | 135/258 | 36.9s | 51.9s |
| 21 | `ae7b971597` | Merge pull request #14752 from cli/babakks/re | 4.1s / 0% / 4.1s | 4.3s / 0% / 4.3s | 62.7s / 29% / 44.7s | 99/261 | 156/259 | 145/259 | 33.4s | 50.2s |
| 22 | `99750e1576` | Merge pull request #14768 from cli/babakks/re | 3.6s / 0% / 3.6s | 1.8s / 0% / 1.8s | 52.9s / 61% / 20.8s | 155/261 | 244/259 | 207/259 | 16.7s | 42.0s |
| 23 | `7948e8056e` | Merge pull request #14782 from cli/babakks/sk | 2.7s / 0% / 2.7s | 1.0s / 0% / 1.0s | 51.2s / 93% / 3.6s | 244/261 | 259/259 | 221/259 | 12.7s | 40.7s |
| 24 | `5056c0a8ef` | Merge pull request #14784 from cli/tommaso-mo | 2.9s / 0% / 2.9s | 1.1s / 0% / 1.1s | 51.3s / 94% / 3.2s | 252/261 | 252/259 | 216/259 | 14.4s | 40.8s |
| 25 | `ed84a97a55` | Merge pull request #14791 from cli/bdehamer-d | 4.8s / 0% / 4.8s | 2.4s / 0% / 2.4s | 53.7s / 78% / 11.8s | 235/261 | 237/259 | 207/259 | 18.0s | 42.8s |
| 26 | `e87ed4c751` | Document extrepo installation on Debian (#147 | 2.7s / 0% / 2.7s | 1.0s / 0% / 1.0s | 50.9s / 86% / 7.1s | 237/261 | 259/259 | 221/259 | 12.3s | 40.6s |
| 27 | `64d175234b` | Merge pull request #14798 from cli/babakks/po | 3.0s / 0% / 3.0s | 1.4s / 0% / 1.4s | 51.2s / 95% / 2.6s | 254/261 | 254/259 | 218/259 | 13.6s | 41.4s |
| 28 | `059f4975b5` | Merge pull request #14804 from martinajir/att | 4.5s / 0% / 4.5s | 2.7s / 0% / 2.7s | 55.7s / 70% / 16.9s | 208/261 | 209/259 | 213/259 | 21.3s | 45.0s |
| 29 | `4f2cfe92e3` | Merge pull request #14805 from cli/bagtoad/at | 5.6s / 0% / 5.6s | 5.4s / 0% / 5.4s | 66.2s / 38% / 41.1s | 136/261 | 136/259 | 128/259 | 37.0s | 52.7s |
| 30 | `63837fb778` | Merge pull request #14815 from cli/babakks/au | 3.0s / 0% / 3.0s | 1.4s / 0% / 1.4s | 52.3s / 57% / 22.7s | 137/261 | 254/259 | 218/259 | 13.5s | 41.1s |
| 31 | `0186b810be` | Merge pull request #14801 from tj-smith47/rem | 11.8s / 0% / 11.8s | 10.3s / 0% / 10.3s | 76.7s / 6% / 72.5s | 34/261 | 34/259 | 60/259 | 56.1s | 62.5s |
| 32 | `9be5608274` | Merge pull request #14824 from cli/dependabot | 2.8s / 0% / 2.8s | 1.0s / 0% / 1.0s | 50.8s / 21% / 40.1s | 34/261 | 259/259 | 221/259 | 12.4s | 41.0s |
| 33 | `5687d4ffe8` | Merge pull request #14837 from cli/williammar | 2.8s / 0% / 2.8s | 1.0s / 0% / 1.0s | 50.7s / 100% / 0.1s | 259/261 | 259/259 | 221/259 | 12.7s | 40.5s |
| 34 | `93d24ce9ff` | Only save Go caches from trunk (#14840) | 2.7s / 0% / 2.7s | 0.9s / 0% / 0.9s | 50.8s / 100% / 0.1s | 259/261 | 259/259 | 221/259 | 12.5s | 40.4s |
| 35 | `674516ff39` | Merge pull request #14854 from bobrik/ivan/no | 2.7s / 0% / 2.7s | 1.1s / 0% / 1.1s | 50.9s / 99% / 0.7s | 258/261 | 258/259 | 221/259 | 12.8s | 41.0s |
| 36 | `f7a06a18a9` | Merge pull request #14825 from cli/bagtoad/gh | 2.8s / 0% / 2.8s | 0.9s / 0% / 0.9s | 50.5s / 99% / 0.3s | 258/261 | 259/259 | 221/259 | 12.5s | 40.9s |
| 37 | `8553724d15` | Merge pull request #14858 from cli/bagtoad/pr | 3.0s / 0% / 3.0s | 1.4s / 0% / 1.4s | 51.4s / 94% / 2.8s | 254/261 | 253/259 | 218/259 | 13.8s | 41.3s |
| 38 | `30eb70ca63` | Merge pull request #14878 from cli/dependabot | 7.9s / 0% / 7.9s | 3.5s / 0% / 3.5s | 55.0s / 75% / 13.6s | 233/261 | 234/259 | 204/259 | 19.5s | 44.2s |
| 39 | `443baeba05` | Merge pull request #14355 from cli/williammar | 10.9s / 0% / 10.9s | 9.6s / 0% / 9.6s | 75.4s / 8% / 69.6s | 43/259 | 47/259 | 42/259 | 53.7s | 61.0s |
| 40 | `9490d204a2` | Merge pull request #14879 from cli/bagtoad/sh | 2.8s / 0% / 2.8s | 1.0s / 0% / 1.0s | 51.2s / 25% / 38.6s | 47/259 | 258/259 | 220/259 | 10.6s | 41.5s |


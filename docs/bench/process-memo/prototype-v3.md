## Prototype on the history: `go test ./...` per commit

| # | Commit | Subject | plain-warm (paired) | memo | bench plain-warm | bench Quod miss | links replayed | tests replayed | opaque | ran (first/proving/changed) | materialize | saved (recorded) |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | `b1c8c0ae5a` | Merge pull request #14750 from cli/willi | 0.0s | 116.7s (exit 0) | 66.6s | 69.6s | 0/260 | 0/258 | 11 | 507/0/0 | 0.0s | 0s |
| 2 | `5b0cc54248` | chore(deps): bump the codeql-actions gro | 0.0s | 42.0s (exit 0) | 9.6s | 39.8s | 0/260 | 0/258 | 11 | 261/246/0 | 0.0s | 0s |
| 3 | `ed40e856ff` | Merge pull request #14719 from cli/bagto | 0.0s | 39.2s (exit 0) | 11.0s | 40.1s | 0/260 | 240/258 | 11 | 16/251/0 | 0.0s | 17s |
| 4 | `5cfe30624f` | Bump Go to 1.27.3 (#14767) | 0.0s | 7.8s (exit 0) | 10.3s | 39.8s | 251/260 | 240/258 | 11 | 10/6/0 | 2.3s | 139s |
| 5 | `ad27e286ab` | Merge pull request #14770 from cli/bagto | 0.0s | 7.2s (exit 0) | 10.3s | 40.1s | 251/260 | 246/258 | 11 | 2/8/0 | 2.3s | 140s |
| 6 | `401854ece8` | chore(deps): bump github.com/gabriel-vas | 0.0s | 10.9s (exit 0) | 12.9s | 41.0s | 240/260 | 229/258 | 11 | 38/0/0 | 2.3s | 133s |
| 7 | `1531837d85` | Merge pull request #14148 from heaths/is | 0.0s | 75.6s (exit 0) | 59.3s | 62.6s | 1/260 | 16/258 | 11 | 490/0/0 | 0.0s | 0s |
| 8 | `ad4a6456e1` | Merge pull request #14450 from cli/babak | 0.0s | 74.0s (exit 0) | 56.5s | 61.9s | 1/260 | 15/258 | 11 | 465/26/0 | 0.0s | 0s |
| 9 | `4f168ce66d` | Merge pull request #14451 from cli/babak | 0.0s | 72.7s (exit 0) | 56.0s | 61.8s | 16/260 | 26/258 | 11 | 450/15/0 | 0.1s | 4s |
| 10 | `a81663f7d8` | Merge pull request #14452 from cli/babak | 0.0s | 42.0s (exit 0) | 16.2s | 41.9s | 29/260 | 28/258 | 11 | 245/205/0 | 0.1s | 8s |
| 11 | `b7887068cb` | Merge pull request #14453 from cli/babak | 0.0s | 36.8s (exit 0) | 14.1s | 40.1s | 31/260 | 230/258 | 11 | 25/221/0 | 0.2s | 25s |
| 12 | `bd0b0e085f` | Merge pull request #14454 from cli/babak | 0.0s | 10.0s (exit 0) | 14.5s | 40.4s | 240/260 | 241/258 | 11 | 13/13/0 | 2.2s | 132s |
| 13 | `d8474903cf` | Merge pull request #14455 from cli/babak | 0.0s | 10.5s (exit 0) | 15.2s | 41.1s | 246/260 | 236/258 | 11 | 23/2/0 | 2.2s | 134s |
| 14 | `316e7a80fb` | Merge pull request #14456 from cli/babak | 0.0s | 8.7s (exit 0) | 13.2s | 40.2s | 247/260 | 237/258 | 11 | 12/11/0 | 2.2s | 134s |
| 15 | `4a289a9e19` | Merge pull request #14663 from cli/babak | 0.0s | 73.6s (exit 0) | 56.5s | 62.1s | 1/260 | 28/258 | 11 | 478/0/0 | 0.0s | 1s |
| 16 | `e62e84ca88` | Merge pull request #14665 from cli/babak | 0.0s | 44.5s (exit 0) | 14.2s | 42.3s | 1/260 | 28/258 | 11 | 241/237/0 | 0.0s | 1s |
| 17 | `56682b6faf` | Merge pull request #14672 from cli/babak | 0.0s | 71.5s (exit 0) | 55.6s | 62.0s | 31/260 | 28/258 | 11 | 448/0/0 | 0.2s | 8s |
| 18 | `abf0e0c25a` | Merge pull request #14673 from cli/babak | 0.0s | 41.2s (exit 0) | 15.8s | 41.4s | 31/260 | 28/258 | 11 | 239/209/0 | 0.1s | 8s |
| 19 | `6314d776e1` | Merge pull request #14717 from cli/babak | 0.0s | 70.9s (exit 0) | 55.2s | 61.9s | 31/260 | 28/258 | 11 | 448/0/0 | 0.2s | 8s |
| 20 | `547bf81aa5` | Merge pull request #14745 from cli/babak | 0.0s | 56.6s (exit 0) | 36.9s | 51.9s | 29/260 | 26/258 | 11 | 346/106/0 | 0.1s | 7s |
| 21 | `ae7b971597` | Merge pull request #14752 from cli/babak | 0.0s | 52.5s (exit 0) | 33.4s | 50.2s | 5/261 | 93/259 | 11 | 353/58/0 | 0.0s | 5s |
| 22 | `99750e1576` | Merge pull request #14768 from cli/babak | 0.0s | 42.2s (exit 0) | 16.7s | 42.0s | 6/261 | 149/259 | 11 | 120/234/0 | 0.0s | 8s |
| 23 | `7948e8056e` | Merge pull request #14782 from cli/babak | 0.0s | 20.4s (exit 0) | 12.7s | 40.7s | 155/261 | 234/259 | 11 | 18/102/0 | 3.0s | 89s |
| 24 | `5056c0a8ef` | Merge pull request #14784 from cli/tomma | 0.0s | 9.7s (exit 0) | 14.4s | 40.8s | 241/261 | 242/259 | 11 | 15/11/0 | 3.4s | 132s |
| 25 | `ed84a97a55` | Merge pull request #14791 from cli/bdeha | 0.0s | 16.5s (exit 0) | 18.0s | 42.8s | 234/261 | 224/259 | 11 | 49/2/0 | 3.2s | 118s |
| 26 | `e87ed4c751` | Document extrepo installation on Debian  | 0.0s | 12.4s (exit 0) | 12.3s | 40.6s | 234/261 | 226/259 | 11 | 25/24/0 | 3.1s | 118s |
| 27 | `64d175234b` | Merge pull request #14798 from cli/babak | 0.0s | 12.5s (exit 0) | 13.6s | 41.4s | 236/261 | 243/259 | 11 | 12/18/0 | 3.3s | 121s |
| 28 | `059f4975b5` | Merge pull request #14804 from martinaji | 0.0s | 21.0s (exit 0) | 21.3s | 45.0s | 208/261 | 199/259 | 11 | 101/1/0 | 2.5s | 109s |
| 29 | `4f2cfe92e3` | Merge pull request #14805 from cli/bagto | 0.0s | 40.1s (exit 0) | 37.0s | 52.7s | 136/261 | 131/259 | 11 | 242/0/0 | 1.6s | 66s |
| 30 | `63837fb778` | Merge pull request #14815 from cli/babak | 0.0s | 25.2s (exit 0) | 13.5s | 41.1s | 136/261 | 131/259 | 11 | 130/112/0 | 1.4s | 66s |
| 31 | `0186b810be` | Merge pull request #14801 from tj-smith4 | 0.0s | 72.4s (exit 0) | 56.1s | 62.5s | 13/261 | 31/259 | 11 | 465/0/0 | 0.0s | 4s |
| 32 | `9be5608274` | Merge pull request #14824 from cli/depen | 0.0s | 671.2s (exit 1) | 12.4s | 41.0s | 13/261 | 31/258 | 11 | 228/236/0 | 0.0s | 4s |
| 33 | `5687d4ffe8` | Merge pull request #14837 from cli/willi | 0.0s | 35.8s (exit 0) | 12.7s | 40.5s | 34/261 | 246/259 | 11 | 2/227/0 | 0.2s | 25s |
| 34 | `93d24ce9ff` | Only save Go caches from trunk (#14840) | 0.0s | 5.4s (exit 0) | 12.5s | 40.4s | 259/261 | 247/259 | 11 | 2/1/0 | 2.3s | 142s |
| 35 | `674516ff39` | Merge pull request #14854 from bobrik/iv | 0.0s | 5.8s (exit 0) | 12.8s | 41.0s | 258/261 | 247/259 | 11 | 3/1/0 | 2.3s | 141s |
| 36 | `f7a06a18a9` | Merge pull request #14825 from cli/bagto | 0.0s | 5.6s (exit 0) | 12.5s | 40.9s | 258/261 | 247/259 | 11 | 3/1/0 | 2.4s | 141s |
| 37 | `8553724d15` | Merge pull request #14858 from cli/bagto | 0.0s | 8.6s (exit 0) | 13.8s | 41.3s | 253/261 | 242/259 | 11 | 13/1/0 | 2.2s | 135s |
| 38 | `30eb70ca63` | Merge pull request #14878 from cli/depen | 0.0s | 21.0s (exit 0) | 19.5s | 44.2s | 233/261 | 222/259 | 11 | 52/2/0 | 2.0s | 115s |
| 39 | `443baeba05` | Merge pull request #14355 from cli/willi | 0.0s | 67.4s (exit 0) | 53.7s | 61.0s | 43/259 | 41/259 | 11 | 419/4/0 | 0.2s | 12s |
| 40 | `9490d204a2` | Merge pull request #14879 from cli/bagto | 0.0s | 38.8s (exit 0) | 10.6s | 41.5s | 43/259 | 45/259 | 11 | 212/207/0 | 0.2s | 14s |

**Totals, commits 2-40:** plain-warm (paired where run, else the 0b bench's) 943 s, memo 1980 s (+110%); bench plain-warm 943 s, bench Quod miss 1804 s. Memo faster than plain-warm on 14/39 commits.

Outcomes over commits 2-40: compile passthrough: 7411, link ran-first: 4418, link ran-proving: 1063, link replayed: 4675, test opaque: 429, test ran-first: 2296, test ran-proving: 1735, test replayed: 5621, vet passthrough: 5105. Materialization (copying replayed link outputs into $WORK): 48 s serial in total.

Opaque units (count over commits): `internal/skills/installer: spawned 1 subprocess(es)` x40; `internal/prompter: non-regular input /usr/bin/echo` x40; `pkg/cmd/auth/shared/gitcredentials: spawned 44 subprocess(es)` x40; `pkg/cmd/codespace: spawned 15 subprocess(es)` x40; `pkg/cmd/extension: spawned 11 subprocess(es)` x40; `pkg/cmd/issue/create: spawned 2 subprocess(es)` x40; `pkg/cmd/pr/merge: spawned 1 subprocess(es)` x40; `pkg/cmd/repo/sync: spawned 108 subprocess(es)` x40; `pkg/cmd/skills/search: spawned 1 subprocess(es)` x40; `pkg/surveyext: spawned 2 subprocess(es)` x40; `git: spawned 109 subprocess(es)` x38; `git: spawned 657 subprocess(es)` x1; `git: spawned 664 subprocess(es)` x1

## Unit-level mutation tests

Results: {"a7_false_hits": 0, "a7_disagree": 0, "a7_ok": 40, "a7_not_tested": 0, "a7b_ok": 10, "a7b_no_replay": 0, "a7b_disagree": 0}

- A7 ./pkg/cmd/issue/edit mutated `pkg/cmd/issue/edit/nope.png` (created): ok; shim ['ran-first'] exit 1 ['--- FAIL: TestNewCmdEdit (0.00s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: TestNewCmdEdit (0.00s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/pr/comment mutated `pkg/cmd/pr/comment/nope.png` (created): ok; shim ['ran-first'] exit 1 ['--- FAIL: TestNewCmdComment (0.00s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: TestNewCmdComment (0.00s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/issue/list mutated `pkg/cmd/issue/list/fixtures/issueList.json` (corrupted): ok; shim ['ran-first'] exit 1 ['--- FAIL: TestIssueList_nontty (0.00s)', '--- FAIL: TestIssueList_tty (0.00s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: TestIssueList_nontty (0.00s)', '--- FAIL: TestIssueList_tty (0.00s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/pr/view mutated `pkg/cmd/pr/view/fixtures/prViewPreviewDraftState.json` (corrupted): ok; shim ['ran-first'] exit 1 ['--- FAIL: TestPRView_Preview_nontty (0.00s)', '--- FAIL: TestPRView_Preview (0.00s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: TestPRView_Preview_nontty (0.00s)', '--- FAIL: TestPRView_Preview (0.00s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/pr/edit mutated `pkg/cmd/pr/edit/nope.png` (created): ok; shim ['ran-first'] exit 1 ['--- FAIL: TestNewCmdEdit (0.00s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: TestNewCmdEdit (0.00s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/attestation/download mutated `pkg/cmd/attestation/test/data/sigstore-js-2.1.0.tgz` (corrupted): ok; shim ['ran-first'] exit 0 ['ok  ']; plain exit 0 ['ok  ']
- A7 ./pkg/cmd/attestation/trustedroot mutated `pkg/cmd/attestation/verification/embed/tuf-repo.github.com/root.json` (corrupted): ok; shim ['ran-first'] exit 0 ['ok  ']; plain exit 0 ['ok  ']
- A7 ./pkg/cmd/attestation/inspect mutated `pkg/cmd/attestation/test/data/non-existent-sigstoreBundle.json` (created): ok; shim ['ran-first'] exit 0 ['ok  ']; plain exit 0 ['ok  ']
- A7 ./pkg/cmd/pr/list mutated `pkg/cmd/pr/list/fixtures/prList.json` (corrupted): ok; shim ['ran-first'] exit 1 ['--- FAIL: TestPRList (0.00s)', '--- FAIL: TestPRList_nontty (0.00s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: TestPRList (0.00s)', '--- FAIL: TestPRList_nontty (0.00s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/issue/list mutated `pkg/cmd/issue/list/fixtures/issueList.json` (corrupted): ok; shim ['ran-first'] exit 1 ['--- FAIL: TestIssueList_nontty (0.00s)', '--- FAIL: TestIssueList_tty (0.00s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: TestIssueList_nontty (0.00s)', '--- FAIL: TestIssueList_tty (0.00s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/ruleset/view mutated `pkg/cmd/ruleset/view/fixtures/rulesetViewRepo.json` (corrupted): ok; shim ['ran-first'] exit 1 ['--- FAIL: Test_viewRun (0.00s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: Test_viewRun (0.00s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/attestation/verify mutated `pkg/cmd/attestation/test/data/gh_2.60.1_windows_arm64.zip` (corrupted): ok; shim ['ran-first'] exit 0 ['ok  ']; plain exit 0 ['ok  ']
- A7 ./pkg/cmd/release/verify-asset mutated `pkg/cmd/attestation/test/data/github_release_artifact_invalid.zip` (corrupted): ok; shim ['ran-first'] exit 0 ['ok  ']; plain exit 0 ['ok  ']
- A7 ./pkg/cmd/issue/status mutated `pkg/cmd/issue/status/fixtures/issueStatus.json` (corrupted): ok; shim ['ran-first'] exit 1 ['--- FAIL: TestIssueStatus (0.00s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: TestIssueStatus (0.00s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/issue/comment mutated `pkg/cmd/issue/comment/nope.png` (created): ok; shim ['ran-first'] exit 1 ['--- FAIL: TestNewCmdComment (0.00s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: TestNewCmdComment (0.00s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/ruleset/list mutated `pkg/cmd/ruleset/list/fixtures/rulesetList.json` (corrupted): ok; shim ['ran-first'] exit 1 ['--- FAIL: Test_listRun (0.00s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: Test_listRun (0.00s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/pr/checks mutated `pkg/cmd/pr/checks/fixtures/someFailing.json` (corrupted): ok; shim ['ran-first'] exit 1 ['--- FAIL: Test_checksRun (0.00s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: Test_checksRun (0.00s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/release/verify-asset mutated `pkg/cmd/release/verify-asset/artifact.zip` (created): ok; shim ['ran-first'] exit 1 ['--- FAIL: Test_verifyAssetRun_NoSuchAsset (0.00s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: Test_verifyAssetRun_NoSuchAsset (0.00s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/ruleset/check mutated `pkg/cmd/ruleset/check/fixtures/rulesetCheck.json` (corrupted): ok; shim ['ran-first'] exit 1 ['--- FAIL: Test_checkRun (0.00s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: Test_checkRun (0.00s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/pr/list mutated `pkg/cmd/pr/list/fixtures/prList.json` (corrupted): ok; shim ['ran-first'] exit 1 ['--- FAIL: TestPRList (0.00s)', '--- FAIL: TestPRList_nontty (0.00s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: TestPRList (0.00s)', '--- FAIL: TestPRList_nontty (0.00s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/attestation/download mutated `pkg/cmd/attestation/test/data/not-real.zip` (created): ok; shim ['ran-first'] exit 1 ['--- FAIL: TestRunDownload (0.01s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: TestRunDownload (0.01s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/repo/list mutated `pkg/cmd/repo/list/fixtures/repoList.json` (corrupted): ok; shim ['ran-first'] exit 0 ['ok  ']; plain exit 0 ['ok  ']
- A7 ./pkg/cmd/pr/edit mutated `pkg/cmd/pr/edit/nope.png` (created): ok; shim ['ran-proving'] exit 1 ['--- FAIL: TestNewCmdEdit (0.00s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: TestNewCmdEdit (0.00s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/release/verify-asset mutated `pkg/cmd/attestation/test/data/github_release_artifact_invalid.zip` (corrupted): ok; shim ['ran-first'] exit 0 ['ok  ']; plain exit 0 ['ok  ']
- A7 ./pkg/cmd/issue/comment mutated `pkg/cmd/issue/comment/nope.png` (created): ok; shim ['ran-proving'] exit 1 ['--- FAIL: TestNewCmdComment (0.00s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: TestNewCmdComment (0.00s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/status mutated `pkg/cmd/status/fixtures/events.json` (corrupted): ok; shim ['ran-first'] exit 0 ['ok  ']; plain exit 0 ['ok  ']
- A7 ./internal/attachments mutated `internal/attachments/testdata/references_input.md` (corrupted): ok; shim ['ran-first'] exit 1 ['--- FAIL: TestAttachAssetsToMarkdownFixture (0.00s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: TestAttachAssetsToMarkdownFixture (0.00s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/agent-task/shared mutated `pkg/cmd/agent-task/shared/testdata/log-1-want.txt` (corrupted): ok; shim ['ran-first'] exit 1 ['--- FAIL: TestFollow (0.01s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: TestFollow (0.01s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/attestation/artifact mutated `pkg/cmd/attestation/test/data/github_release_artifact.zip` (corrupted): ok; shim ['ran-first'] exit 1 ['--- FAIL: Test_digestLocalFileArtifact_withRealZip (0.00s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: Test_digestLocalFileArtifact_withRealZip (0.00s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/attestation/verify mutated `pkg/cmd/attestation/test/data/non-existent-sigstoreBundle.json` (created): ok; shim ['ran-first'] exit 0 ['ok  ']; plain exit 0 ['ok  ']
- A7 ./pkg/cmd/pr/checks mutated `pkg/cmd/pr/checks/fixtures/someSkipping.json` (corrupted): ok; shim ['ran-first'] exit 1 ['--- FAIL: Test_checksRun (0.00s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: Test_checksRun (0.00s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/pr/view mutated `pkg/cmd/pr/view/fixtures/prViewPreviewSingleComment.json` (corrupted): ok; shim ['ran-first'] exit 1 ['--- FAIL: TestPRView_tty_Comments (0.00s)', '--- FAIL: TestPRView_nontty_Comments (0.00s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: TestPRView_tty_Comments (0.00s)', '--- FAIL: TestPRView_nontty_Comments (0.00s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/issue/comment mutated `pkg/cmd/issue/comment/nope.png` (created): ok; shim ['ran-proving'] exit 1 ['--- FAIL: TestNewCmdComment (0.00s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: TestNewCmdComment (0.00s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/attestation/artifact mutated `pkg/cmd/attestation/test/data/github_release_artifact.zip` (corrupted): ok; shim ['ran-first'] exit 1 ['--- FAIL: Test_digestLocalFileArtifact_withRealZip (0.00s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: Test_digestLocalFileArtifact_withRealZip (0.00s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/pr/create mutated `pkg/cmd/pr/create/docs` (created): ok; shim ['ran-first'] exit 0 ['ok  ']; plain exit 0 ['ok  ']
- A7 ./pkg/cmd/ruleset/view mutated `pkg/cmd/ruleset/view/fixtures/rulesetViewMultiple.json` (corrupted): ok; shim ['ran-first'] exit 1 ['--- FAIL: Test_viewRun (0.00s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: Test_viewRun (0.00s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/attestation/verify mutated `pkg/cmd/attestation/test/data/sigstore-js-2.1.0.tgz` (corrupted): ok; shim ['ran-first'] exit 0 ['ok  ']; plain exit 0 ['ok  ']
- A7 ./pkg/cmd/attestation/trustedroot mutated `pkg/cmd/attestation/verification/embed/tuf-repo.github.com/root.json` (corrupted): ok; shim ['ran-first'] exit 0 ['ok  ']; plain exit 0 ['ok  ']
- A7 ./pkg/cmd/ruleset/list mutated `pkg/cmd/ruleset/list/fixtures/rulesetList.json` (corrupted): ok; shim ['ran-first'] exit 1 ['--- FAIL: Test_listRun (0.00s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: Test_listRun (0.00s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/status mutated `pkg/cmd/status/fixtures/events.json` (corrupted): ok; shim ['ran-first'] exit 1 ['--- FAIL: TestStatusRun (0.00s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: TestStatusRun (0.00s)', 'FAIL', 'FAIL', 'FAIL']
- A7b ./pkg/cmd/status after mutating unread `docs/api-host-test-harness.md`: ok; shim ['replayed']; plain exit 0 ['ok  ']
- A7b ./pkg/cmd/pr/create after mutating unread `docs/primer/foundations/README.md`: ok; shim ['replayed']; plain exit 0 ['ok  ']
- A7b ./pkg/cmd/pr/create after mutating unread `docs/codespaces.md`: ok; shim ['replayed']; plain exit 0 ['ok  ']
- A7b ./pkg/cmd/attestation/artifact after mutating unread `docs/gh-vs-hub.md`: ok; shim ['replayed']; plain exit 0 ['ok  ']
- A7b ./pkg/cmd/attestation/download after mutating unread `docs/install_source.md`: ok; shim ['replayed']; plain exit 0 ['ok  ']
- A7b ./pkg/cmd/pr/view after mutating unread `LICENSE`: ok; shim ['replayed']; plain exit 0 ['ok  ']
- A7b ./pkg/cmd/repo/create after mutating unread `docs/api-host.md`: ok; shim ['replayed']; plain exit 0 ['ok  ']
- A7b ./pkg/cmd/status after mutating unread `docs/README.md`: ok; shim ['replayed']; plain exit 0 ['ok  ']
- A7b ./pkg/cmd/agent-task/shared after mutating unread `docs/multiple-accounts.md`: ok; shim ['replayed']; plain exit 0 ['ok  ']
- A7b ./internal/attachments after mutating unread `docs/release-process-deep-dive.md`: ok; shim ['replayed']; plain exit 0 ['ok  ']

Link fidelity (the whole step twice more on the unchanged tree):
- wall 34.5s exit 0: tests replayed 247/259, links replayed 47/259; lookups 5.2s, materialize 0.3s, opaque units 5.8s, ran 111.5s (all serial); not replayed: {"internal/ghcmd": "ran-first", "internal/skills/installer": "opaque:spawned 1 subprocess(es)", "git": "opaque:spawned 664 subprocess(es)", "internal/prompter": "opaque:non-regular input /usr/bin/echo", "pkg/cmd/auth/shared/gitcredentials": "opaque:spawned 44 subprocess(es)", "pkg/cmd/codespace": "opaque:spawned 13 subprocess(es)", "pkg/cmd/extension": "opaque:spawned 11 subprocess(es)", "pkg/cmd/issue/create": "opaque:spawned 2 subprocess(es)", "pkg/cmd/pr/merge": "opaque:spawned 1 subprocess(es)", "pkg/cmd/repo/sync": "opaque:spawned 108 subprocess(es)", "pkg/cmd/skills/search": "opaque:spawned 1 subprocess(es)", "pkg/surveyext": "opaque:spawned 2 subprocess(es)"}
- wall 6.1s exit 0: tests replayed 247/259, links replayed 258/259; lookups 4.8s, materialize 3.7s, opaque units 4.9s, ran 0.3s (all serial); not replayed: {"internal/ghcmd": "ran-proving", "internal/skills/installer": "opaque:spawned 1 subprocess(es)", "pkg/cmd/auth/shared/gitcredentials": "opaque:spawned 44 subprocess(es)", "git": "opaque:spawned 664 subprocess(es)", "pkg/cmd/codespace": "opaque:spawned 13 subprocess(es)", "pkg/cmd/extension": "opaque:spawned 11 subprocess(es)", "internal/prompter": "opaque:non-regular input /usr/bin/echo", "pkg/cmd/issue/create": "opaque:spawned 2 subprocess(es)", "pkg/cmd/pr/merge": "opaque:spawned 1 subprocess(es)", "pkg/cmd/repo/sync": "opaque:spawned 108 subprocess(es)", "pkg/cmd/skills/search": "opaque:spawned 1 subprocess(es)", "pkg/surveyext": "opaque:spawned 2 subprocess(es)"}
- wall 6.2s exit 0: tests replayed 247/259, links replayed 259/259; lookups 4.8s, materialize 3.7s, opaque units 4.8s, ran 0.1s (all serial); not replayed: {"internal/ghcmd": "ran-proving", "internal/skills/installer": "opaque:spawned 1 subprocess(es)", "pkg/cmd/auth/shared/gitcredentials": "opaque:spawned 44 subprocess(es)", "git": "opaque:spawned 664 subprocess(es)", "pkg/cmd/codespace": "opaque:spawned 13 subprocess(es)", "pkg/cmd/extension": "opaque:spawned 11 subprocess(es)", "internal/prompter": "opaque:non-regular input /usr/bin/echo", "pkg/cmd/issue/create": "opaque:spawned 2 subprocess(es)", "pkg/cmd/pr/merge": "opaque:spawned 1 subprocess(es)", "pkg/cmd/repo/sync": "opaque:spawned 108 subprocess(es)", "pkg/cmd/skills/search": "opaque:spawned 1 subprocess(es)", "pkg/surveyext": "opaque:spawned 2 subprocess(es)"}

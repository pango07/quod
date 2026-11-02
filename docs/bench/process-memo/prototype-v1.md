## Prototype on the history: `go test ./...` per commit

| # | Commit | Subject | plain-warm (paired) | memo | bench plain-warm | bench Quod miss | links replayed | tests replayed | opaque | ran (first/proving/changed) | materialize | saved (recorded) |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | `b1c8c0ae5a` | Merge pull request #14750 from cli/willi | 108.0s | 126.1s (exit 0) | 66.6s | 69.6s | 0/260 | 0/258 | 12 | 506/0/0 | 0.0s | 0s |
| 2 | `5b0cc54248` | chore(deps): bump the codeql-actions gro | 10.1s | 51.5s (exit 0) | 9.6s | 39.8s | 0/260 | 0/258 | 12 | 260/246/0 | 0.0s | 0s |
| 3 | `ed40e856ff` | Merge pull request #14719 from cli/bagto | 11.1s | 48.4s (exit 0) | 11.0s | 40.1s | 0/260 | 239/258 | 12 | 16/251/0 | 0.0s | 16s |
| 4 | `5cfe30624f` | Bump Go to 1.27.3 (#14767) | 10.8s | 18.9s (exit 0) | 10.3s | 39.8s | 251/260 | 239/258 | 12 | 9/7/0 | 3.7s | 132s |
| 5 | `ad27e286ab` | Merge pull request #14770 from cli/bagto | 10.4s | 18.8s (exit 0) | 10.3s | 40.1s | 251/260 | 246/258 | 12 | 2/7/0 | 3.6s | 133s |
| 6 | `401854ece8` | chore(deps): bump github.com/gabriel-vas | 13.9s | 22.0s (exit 0) | 12.9s | 41.0s | 240/260 | 228/258 | 12 | 38/0/0 | 3.3s | 126s |
| 7 | `1531837d85` | Merge pull request #14148 from heaths/is | 68.9s | 84.3s (exit 0) | 59.3s | 62.6s | 1/260 | 17/258 | 12 | 488/0/0 | 0.0s | 0s |
| 8 | `ad4a6456e1` | Merge pull request #14450 from cli/babak | 66.0s | 82.5s (exit 0) | 56.5s | 61.9s | 1/260 | 16/258 | 12 | 463/26/0 | 0.0s | 0s |
| 9 | `4f168ce66d` | Merge pull request #14451 from cli/babak | 65.5s | 81.7s (exit 0) | 56.0s | 61.8s | 16/260 | 27/258 | 12 | 448/15/0 | 0.2s | 4s |
| 10 | `a81663f7d8` | Merge pull request #14452 from cli/babak | 17.7s | 51.4s (exit 0) | 16.2s | 41.9s | 29/260 | 29/258 | 12 | 243/205/0 | 0.3s | 8s |
| 11 | `b7887068cb` | Merge pull request #14453 from cli/babak | 14.5s | 46.7s (exit 0) | 14.1s | 40.1s | 31/260 | 231/258 | 12 | 25/219/0 | 0.3s | 24s |
| 12 | `bd0b0e085f` | Merge pull request #14454 from cli/babak | 15.1s | 20.8s (exit 0) | 14.5s | 40.4s | 240/260 | 240/258 | 12 | 13/13/0 | 3.7s | 129s |
| 13 | `d8474903cf` | Merge pull request #14455 from cli/babak | 16.2s | 21.1s (exit 0) | 15.2s | 41.1s | 246/260 | 235/258 | 12 | 23/2/0 | 3.9s | 131s |
| 14 | `316e7a80fb` | Merge pull request #14456 from cli/babak | 13.6s | 19.5s (exit 0) | 13.2s | 40.2s | 247/260 | 236/258 | 12 | 12/11/0 | 3.5s | 131s |
| 15 | `4a289a9e19` | Merge pull request #14663 from cli/babak | 65.5s | 82.2s (exit 0) | 56.5s | 62.1s | 1/260 | 29/258 | 12 | 476/0/0 | 0.0s | 0s |
| 16 | `e62e84ca88` | Merge pull request #14665 from cli/babak | 15.1s | 53.7s (exit 0) | 14.2s | 42.3s | 1/260 | 29/258 | 12 | 239/237/0 | 0.0s | 0s |
| 17 | `56682b6faf` | Merge pull request #14672 from cli/babak | 63.9s | 80.3s (exit 0) | 55.6s | 62.0s | 31/260 | 29/258 | 12 | 446/0/0 | 0.4s | 8s |
| 18 | `abf0e0c25a` | Merge pull request #14673 from cli/babak | 17.1s | 50.7s (exit 0) | 15.8s | 41.4s | 31/260 | 29/258 | 12 | 237/209/0 | 0.2s | 8s |
| 19 | `6314d776e1` | Merge pull request #14717 from cli/babak | 64.4s | 79.8s (exit 0) | 55.2s | 61.9s | 31/260 | 29/258 | 12 | 446/0/0 | 0.1s | 8s |
| 20 | `547bf81aa5` | Merge pull request #14745 from cli/babak | 42.2s | 65.5s (exit 0) | 36.9s | 51.9s | 29/260 | 27/258 | 12 | 347/103/0 | 0.2s | 7s |
| 21 | `ae7b971597` | Merge pull request #14752 from cli/babak | 35.7s | 62.0s (exit 0) | 33.4s | 50.2s | 5/261 | 93/259 | 12 | 351/59/0 | 0.0s | 5s |
| 22 | `99750e1576` | Merge pull request #14768 from cli/babak | 17.9s | 51.9s (exit 0) | 16.7s | 42.0s | 6/261 | 150/259 | 12 | 118/234/0 | 0.1s | 7s |
| 23 | `7948e8056e` | Merge pull request #14782 from cli/babak | 13.1s | 30.5s (exit 0) | 12.7s | 40.7s | 155/261 | 235/259 | 12 | 17/101/0 | 2.6s | 87s |
| 24 | `5056c0a8ef` | Merge pull request #14784 from cli/tomma | 14.9s | 20.3s (exit 0) | 14.4s | 40.8s | 241/261 | 240/259 | 12 | 16/11/0 | 2.6s | 128s |
| 25 | `ed84a97a55` | Merge pull request #14791 from cli/bdeha | 19.8s | 26.5s (exit 0) | 18.0s | 42.8s | 234/261 | 222/259 | 12 | 49/3/0 | 2.1s | 114s |
| 26 | `e87ed4c751` | Document extrepo installation on Debian  | 12.4s | 22.7s (exit 0) | 12.3s | 40.6s | 234/261 | 225/259 | 12 | 24/25/0 | 2.0s | 114s |
| 27 | `64d175234b` | Merge pull request #14798 from cli/babak | 14.2s | 22.4s (exit 0) | 13.6s | 41.4s | 236/261 | 243/259 | 12 | 11/18/0 | 2.0s | 117s |
| 28 | `059f4975b5` | Merge pull request #14804 from martinaji | 22.9s | 30.4s (exit 0) | 21.3s | 45.0s | 208/261 | 199/259 | 12 | 101/0/0 | 1.8s | 106s |
| 29 | `4f2cfe92e3` | Merge pull request #14805 from cli/bagto | 40.4s | 49.7s (exit 0) | 37.0s | 52.7s | 136/261 | 131/259 | 12 | 241/0/0 | 1.2s | 64s |
| 30 | `63837fb778` | Merge pull request #14815 from cli/babak | 14.0s | 35.2s (exit 0) | 13.5s | 41.1s | 136/261 | 131/259 | 12 | 130/111/0 | 1.1s | 64s |
| 31 | `0186b810be` | Merge pull request #14801 from tj-smith4 | 64.9s | 81.7s (exit 0) | 56.1s | 62.5s | 13/261 | 32/259 | 12 | 463/0/0 | 0.1s | 4s |
| 32 | `9be5608274` | Merge pull request #14824 from cli/depen | 12.6s | 50.1s (exit 0) | 12.4s | 41.0s | 13/261 | 32/259 | 12 | 227/236/0 | 0.1s | 4s |
| 33 | `5687d4ffe8` | Merge pull request #14837 from cli/willi | 12.7s | 46.0s (exit 0) | 12.7s | 40.5s | 34/261 | 247/259 | 12 | 2/225/0 | 0.2s | 26s |
| 34 | `93d24ce9ff` | Only save Go caches from trunk (#14840) | 12.7s | 16.7s (exit 0) | 12.5s | 40.4s | 259/261 | 247/259 | 12 | 2/0/0 | 2.3s | 142s |
| 35 | `674516ff39` | Merge pull request #14854 from bobrik/iv | 13.0s | 17.1s (exit 0) | 12.8s | 41.0s | 258/261 | 247/259 | 12 | 3/0/0 | 2.4s | 141s |
| 36 | `f7a06a18a9` | Merge pull request #14825 from cli/bagto | 12.7s | 16.8s (exit 0) | 12.5s | 40.9s | 258/261 | 247/259 | 12 | 3/0/0 | 2.4s | 141s |
| 37 | `8553724d15` | Merge pull request #14858 from cli/bagto | 14.3s | 19.0s (exit 0) | 13.8s | 41.3s | 253/261 | 241/259 | 12 | 13/1/0 | 2.3s | 135s |
| 38 | `30eb70ca63` | Merge pull request #14878 from cli/depen | 24.6s | 30.9s (exit 0) | 19.5s | 44.2s | 233/261 | 221/259 | 12 | 52/2/0 | 2.0s | 115s |
| 39 | `443baeba05` | Merge pull request #14355 from cli/willi | 62.3s | 76.9s (exit 0) | 53.7s | 61.0s | 43/259 | 42/259 | 12 | 417/4/0 | 0.2s | 12s |
| 40 | `9490d204a2` | Merge pull request #14879 from cli/bagto | 10.7s | 48.1s (exit 0) | 10.6s | 41.5s | 43/259 | 46/259 | 12 | 212/205/0 | 0.2s | 14s |

**Totals, commits 2-40:** paired plain-warm 1048 s, memo 1735 s (+66%); bench plain-warm 943 s, bench Quod miss 1804 s. Memo faster than paired plain-warm on 0/39 commits.

Outcomes over commits 2-40: compile passthrough: 7411, link ran-first: 4418, link ran-proving: 1063, link replayed: 4675, test opaque: 468, test ran-first: 2265, test ran-proving: 1723, test replayed: 5626, vet passthrough: 5105. Materialization (copying replayed link outputs into $WORK): 51 s serial in total.

Opaque units (count over commits): `internal/prompter: stats an executable: /usr/local/go/bin/echo` x40; `pkg/cmd/auth/shared/gitcredentials: stats an executable: /usr/bin/git` x38; `pkg/cmd/repo/setdefault: stats an executable: /usr/bin/git` x38; `internal/skills/installer: stats an executable: /usr/bin/git` x37; `pkg/cmd/auth/shared: stats an executable: /usr/bin/ssh-keygen` x36; `pkg/cmd/auth/login: stats an executable: /usr/bin/ssh-keygen` x35; `pkg/cmd/repo/sync: stats an executable: /usr/bin/git` x35; `pkg/cmd/extension: stats an executable: /usr/bin/git` x34; `pkg/cmd/codespace: stats an executable: /usr/bin/ssh-keygen` x27; `git: stats an executable: /usr/local/go/bin/go` x26; `pkg/cmd/issue/create: stats an executable: /usr/bin/git` x25; `pkg/cmd/pr/create: stats an executable: /usr/bin/git` x24; `pkg/cmd/pr/create: stats an executable: /usr/local/go/bin/git` x16; `pkg/cmd/issue/create: stats an executable: /usr/local/go/bin/git` x15; `git: stats an executable: /usr/bin/git` x9; `pkg/cmd/extension: stats an executable: /usr/local/go/bin/git` x6; `pkg/cmd/codespace: stats an executable: /usr/local/go/bin/ssh-keygen` x6; `pkg/cmd/auth/login: stats an executable: /usr/local/go/bin/ssh-keygen` x5; `pkg/cmd/repo/sync: stats an executable: /usr/local/go/bin/git` x5; `git: stats an executable: /usr/local/go/bin/git` x5

## Unit-level mutation tests

Results: {"a7_false_hits": 0, "a7_disagree": 0, "a7_ok": 0, "a7_not_tested": 40, "a7b_ok": 0, "a7b_no_replay": 10, "a7b_disagree": 0}

- A7 ./pkg/cmd/attestation/download mutated `pkg/cmd/attestation/test/data/not-real.zip` (created): no-unit; shim [] exit 1 ['--- FAIL: TestRunDownload (0.01s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: TestRunDownload (0.01s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/attestation/verify mutated `pkg/cmd/attestation/test/data/sigstore-js-2.1.0-bundle.json` (corrupted): no-unit; shim [] exit 1 ['--- FAIL: TestGetAttestations_OCIRegistry_PredicateTypeFiltering (0.01s)', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: TestGetAttestations_OCIRegistry_PredicateTypeFiltering (0.01s)', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/status mutated `pkg/cmd/status/fixtures/search.json` (corrupted): no-unit; shim [] exit 1 ['--- FAIL: TestStatusRun (0.00s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: TestStatusRun (0.01s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/agent-task/shared mutated `pkg/cmd/agent-task/shared/testdata/log-1-want.txt` (corrupted): no-unit; shim [] exit 1 ['--- FAIL: TestFollow (0.01s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: TestFollow (0.01s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/ruleset/list mutated `pkg/cmd/ruleset/list/fixtures/rulesetList.json` (corrupted): no-unit; shim [] exit 0 ['ok  ']; plain exit 0 ['ok  ']
- A7 ./pkg/cmd/agent-task/shared mutated `pkg/cmd/agent-task/shared/testdata/log-2-input.txt` (corrupted): no-unit; shim [] exit 0 ['ok  ']; plain exit 0 ['ok  ']
- A7 ./internal/attachments mutated `internal/attachments/testdata/references_expected.md` (corrupted): no-unit; shim [] exit 1 ['--- FAIL: TestAttachAssetsToMarkdownFixture (0.00s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: TestAttachAssetsToMarkdownFixture (0.00s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/attestation/artifact mutated `pkg/cmd/attestation/test/data/github_release_artifact.zip` (corrupted): no-unit; shim [] exit 1 ['--- FAIL: Test_digestLocalFileArtifact_withRealZip (0.00s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: Test_digestLocalFileArtifact_withRealZip (0.00s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/release/verify-asset mutated `pkg/cmd/release/verify-asset/artifact.zip` (created): no-unit; shim [] exit 1 ['--- FAIL: Test_verifyAssetRun_NoSuchAsset (0.00s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: Test_verifyAssetRun_NoSuchAsset (0.00s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/pr/edit mutated `pkg/cmd/pr/edit/nope.png` (created): no-unit; shim [] exit 1 ['--- FAIL: TestNewCmdEdit (0.00s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: TestNewCmdEdit (0.00s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/pr/merge mutated `pkg/cmd/pr/merge/some/path/git` (created): no-unit; shim [] exit 0 ['ok  ']; plain exit 0 ['ok  ']
- A7 ./pkg/cmd/attestation/artifact mutated `pkg/cmd/attestation/test/data/github_release_artifact.zip` (corrupted): no-unit; shim [] exit 1 ['--- FAIL: Test_digestLocalFileArtifact_withRealZip (0.00s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: Test_digestLocalFileArtifact_withRealZip (0.00s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/ruleset/check mutated `pkg/cmd/ruleset/check/fixtures/rulesetCheck.json` (corrupted): no-unit; shim [] exit 1 ['--- FAIL: Test_checkRun (0.00s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: Test_checkRun (0.00s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/pr/checks mutated `pkg/cmd/pr/checks/fixtures/somePending.json` (corrupted): no-unit; shim [] exit 1 ['--- FAIL: Test_checksRun (0.00s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: Test_checksRun (0.00s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/attestation/trustedroot mutated `pkg/cmd/attestation/verification/embed/tuf-repo.github.com/root.json` (corrupted): no-unit; shim [] exit 0 ['ok  ']; plain exit 0 ['ok  ']
- A7 ./pkg/cmd/pr/status mutated `pkg/cmd/pr/status/fixtures/prStatusChecksWithStatesByCount.json` (corrupted): no-unit; shim [] exit 1 ['--- FAIL: TestPRStatus_reviewsAndChecksWithStatesByCount (0.00s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: TestPRStatus_reviewsAndChecksWithStatesByCount (0.00s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/issue/status mutated `pkg/cmd/issue/status/fixtures/issueStatus.json` (corrupted): no-unit; shim [] exit 1 ['--- FAIL: TestIssueStatus (0.00s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: TestIssueStatus (0.00s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/attestation/download mutated `pkg/cmd/attestation/test/data/sigstore-js-2.1.0.tgz` (corrupted): no-unit; shim [] exit 0 ['ok  ']; plain exit 0 ['ok  ']
- A7 ./pkg/cmd/pr/checks mutated `pkg/cmd/pr/checks/fixtures/someSkipping.json` (corrupted): no-unit; shim [] exit 1 ['--- FAIL: Test_checksRun (0.00s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: Test_checksRun (0.00s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/repo/list mutated `pkg/cmd/repo/list/fixtures/repoList.json` (corrupted): no-unit; shim [] exit 1 ['--- FAIL: TestRepoList_nontty (0.00s)', '--- FAIL: TestRepoList_tty (0.00s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: TestRepoList_nontty (0.00s)', '--- FAIL: TestRepoList_tty (0.00s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/attestation/inspect mutated `pkg/cmd/attestation/test/data/non-existent-sigstoreBundle.json` (created): no-unit; shim [] exit 0 ['ok  ']; plain exit 0 ['ok  ']
- A7 ./pkg/cmd/issue/list mutated `pkg/cmd/issue/list/fixtures/issueList.json` (corrupted): no-unit; shim [] exit 1 ['--- FAIL: TestIssueList_nontty (0.00s)', '--- FAIL: TestIssueList_tty (0.00s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: TestIssueList_nontty (0.00s)', '--- FAIL: TestIssueList_tty (0.00s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/issue/status mutated `pkg/cmd/issue/status/fixtures/issueStatus.json` (corrupted): no-unit; shim [] exit 1 ['--- FAIL: TestIssueStatus (0.00s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: TestIssueStatus (0.00s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/pr/checks mutated `pkg/cmd/pr/checks/fixtures/withEvents.json` (corrupted): no-unit; shim [] exit 1 ['--- FAIL: Test_checksRun (0.00s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: Test_checksRun (0.00s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/attestation/inspect mutated `pkg/cmd/attestation/test/data/non-existent-sigstoreBundle.json` (created): no-unit; shim [] exit 0 ['ok  ']; plain exit 0 ['ok  ']
- A7 ./pkg/cmd/issue/edit mutated `pkg/cmd/issue/edit/nope.png` (created): no-unit; shim [] exit 1 ['--- FAIL: TestNewCmdEdit (0.00s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: TestNewCmdEdit (0.00s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/pr/checks mutated `pkg/cmd/pr/checks/fixtures/somePending.json` (corrupted): no-unit; shim [] exit 1 ['--- FAIL: Test_checksRun (0.00s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: Test_checksRun (0.00s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/pr/list mutated `pkg/cmd/pr/list/fixtures/prListWithDuplicates.json` (corrupted): no-unit; shim [] exit 0 ['ok  ']; plain exit 0 ['ok  ']
- A7 ./pkg/cmd/attestation/verify mutated `pkg/cmd/attestation/test/data/sigstore-js-2.1.0.tgz` (corrupted): no-unit; shim [] exit 0 ['ok  ']; plain exit 0 ['ok  ']
- A7 ./pkg/cmd/pr/comment mutated `pkg/cmd/pr/comment/nope.png` (created): no-unit; shim [] exit 1 ['--- FAIL: TestNewCmdComment (0.00s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: TestNewCmdComment (0.00s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/attestation/verification mutated `pkg/cmd/attestation/test/data/sigstore-js-2.1.0_with_2_bundles.jsonl` (corrupted): no-unit; shim [] exit 1 ['--- FAIL: TestLoadBundlesFromJSONLinesFile (0.00s)', '--- FAIL: TestGetLocalAttestations (0.00s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: TestLoadBundlesFromJSONLinesFile (0.00s)', '--- FAIL: TestGetLocalAttestations (0.00s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/ruleset/check mutated `pkg/cmd/ruleset/check/fixtures/rulesetCheck.json` (corrupted): no-unit; shim [] exit 1 ['--- FAIL: Test_checkRun (0.00s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: Test_checkRun (0.00s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/pr/edit mutated `pkg/cmd/pr/edit/nope.png` (created): no-unit; shim [] exit 1 ['--- FAIL: TestNewCmdEdit (0.00s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: TestNewCmdEdit (0.00s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/pr/edit mutated `pkg/cmd/pr/edit/nope.png` (created): no-unit; shim [] exit 1 ['--- FAIL: TestNewCmdEdit (0.00s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: TestNewCmdEdit (0.00s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/agent-task/shared mutated `pkg/cmd/agent-task/shared/testdata/log-1-want.txt` (corrupted): no-unit; shim [] exit 1 ['--- FAIL: TestFollow (0.01s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: TestFollow (0.01s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/repo/create mutated `pkg/cmd/repo/create/fixtures/repoTempList.json` (corrupted): no-unit; shim [] exit 1 ['--- FAIL: Test_createRun (3.01s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: Test_createRun (3.01s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/attestation/inspect mutated `pkg/cmd/attestation/test/data/non-existent-sigstoreBundle.json` (created): no-unit; shim [] exit 0 ['ok  ']; plain exit 0 ['ok  ']
- A7 ./pkg/cmd/ruleset/list mutated `pkg/cmd/ruleset/list/fixtures/rulesetList.json` (corrupted): no-unit; shim [] exit 1 ['--- FAIL: Test_listRun (0.00s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: Test_listRun (0.00s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/ruleset/view mutated `pkg/cmd/ruleset/view/fixtures/rulesetViewRepo.json` (corrupted): no-unit; shim [] exit 1 ['--- FAIL: Test_viewRun (0.00s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: Test_viewRun (0.00s)', 'FAIL', 'FAIL', 'FAIL']
- A7 ./pkg/cmd/ruleset/view mutated `pkg/cmd/ruleset/view/fixtures/rulesetViewOrg.json` (corrupted): no-unit; shim [] exit 1 ['--- FAIL: Test_viewRun (0.00s)', 'FAIL', 'FAIL', 'FAIL']; plain exit 1 ['--- FAIL: Test_viewRun (0.00s)', 'FAIL', 'FAIL', 'FAIL']
- A7b ./pkg/cmd/issue/status after mutating unread `docs/install_windows.md`: did not replay; shim []; plain exit 0 ['ok  ']
- A7b ./pkg/cmd/issue/view after mutating unread `docs/release-process-deep-dive.md`: did not replay; shim []; plain exit 0 ['ok  ']
- A7b ./pkg/cmd/ruleset/view after mutating unread `docs/api-host-test-harness.md`: did not replay; shim []; plain exit 0 ['ok  ']
- A7b ./pkg/cmd/repo/create after mutating unread `docs/api-and-hosts.md`: did not replay; shim []; plain exit 0 ['ok  ']
- A7b ./pkg/cmd/repo/create after mutating unread `docs/primer/foundations/README.md`: did not replay; shim []; plain exit 0 ['ok  ']
- A7b ./pkg/cmd/pr/edit after mutating unread `docs/api-host.md`: did not replay; shim []; plain exit 0 ['ok  ']
- A7b ./pkg/cmd/agent-task/shared after mutating unread `README.md`: did not replay; shim []; plain exit 0 ['ok  ']
- A7b ./pkg/cmd/agent-task/shared after mutating unread `docs/multiple-accounts.md`: did not replay; shim []; plain exit 0 ['ok  ']
- A7b ./pkg/cmd/attestation/trustedroot after mutating unread `docs/README.md`: did not replay; shim []; plain exit 0 ['ok  ']
- A7b ./pkg/cmd/pr/comment after mutating unread `docs/releasing.md`: did not replay; shim []; plain exit 0 ['ok  ']

Link fidelity (the whole step twice more on the unchanged tree):
- wall 44.3s exit 0: tests replayed 0/0, links replayed 0/0; not replayed: {}
- wall 17.3s exit 0: tests replayed 0/0, links replayed 0/0; not replayed: {}

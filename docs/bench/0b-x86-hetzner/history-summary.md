| # | Commit | Subject | plain-warm job | plain-cold job | quod job | build | vet | test | test first changed input |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | `b1c8c0ae5a` | Merge pull request #14750 from cli/williammartin | 125.9s |  | 139.9s | miss 47.5s | miss 18.1s | miss 69.6s | `-` |
| 2 | `5b0cc54248` | chore(deps): bump the codeql-actions group with  | 12.9s |  | 48.4s | miss 4.3s | proving 3.6s | proving 39.8s | `-` |
| 3 | `ed40e856ff` | Merge pull request #14719 from cli/bagtoad/fix-h | 14.5s |  | 48.9s | miss 4.5s | miss 3.6s | miss 40.1s | `pkg/cmd/ruleset/view/view.go` |
| 4 | `5cfe30624f` | Bump Go to 1.27.3 (#14767) | 13.6s |  | 48.4s | miss 4.4s | miss 3.5s | miss 39.8s | `go.mod` |
| 5 | `ad27e286ab` | Merge pull request #14770 from cli/bagtoad/merge | 13.6s | 123.1s | 48.7s | miss 4.4s | proving 3.5s | proving 40.1s | `-` |
| 6 | `401854ece8` | chore(deps): bump github.com/gabriel-vasile/mime | 17.0s |  | 51.3s | miss 5.7s | miss 3.9s | miss 41.0s | `.quod/gomod/cache/download/github.com/gabriel-vasile/mimetype/@v` |
| 7 | `1531837d85` | Merge pull request #14148 from heaths/issue3718 | 79.4s |  | 89.0s | miss 14.1s | miss 11.3s | miss 62.6s | `.quod/gomod/cache/download/charm.land` |
| 8 | `ad4a6456e1` | Merge pull request #14450 from cli/babakks/refre | 74.7s |  | 87.0s | miss 13.4s | miss 10.8s | miss 61.9s | `.quod/gomod/cache/download/github.com/cli/oauth/@v` |
| 9 | `4f168ce66d` | Merge pull request #14451 from cli/babakks/refre | 74.3s |  | 85.8s | miss 12.5s | miss 10.7s | miss 61.8s | `api/http_client.go` |
| 10 | `a81663f7d8` | Merge pull request #14452 from cli/babakks/refre | 20.8s | 122.7s | 52.1s | miss 5.1s | miss 4.3s | miss 41.9s | `internal/authflow/flow.go` |
| 11 | `b7887068cb` | Merge pull request #14453 from cli/babakks/refre | 17.7s |  | 49.4s | miss 4.8s | miss 3.9s | miss 40.1s | `pkg/cmd/auth/token/token.go` |
| 12 | `bd0b0e085f` | Merge pull request #14454 from cli/babakks/refre | 18.3s |  | 50.0s | miss 4.8s | miss 4.0s | miss 40.4s | `pkg/cmd/auth/status/status.go` |
| 13 | `d8474903cf` | Merge pull request #14455 from cli/babakks/refre | 19.3s |  | 51.0s | miss 5.1s | miss 4.2s | miss 41.1s | `pkg/cmd/agent-task/capi/client.go` |
| 14 | `316e7a80fb` | Merge pull request #14456 from cli/babakks/refre | 16.4s |  | 49.3s | miss 4.7s | miss 3.7s | miss 40.2s | `script` |
| 15 | `4a289a9e19` | Merge pull request #14663 from cli/babakks/refre | 74.8s | 123.3s | 86.5s | miss 12.7s | miss 11.0s | miss 62.1s | `api/client_test.go` |
| 16 | `e62e84ca88` | Merge pull request #14665 from cli/babakks/refre | 19.0s |  | 53.1s | miss 5.7s | miss 4.4s | miss 42.3s | `api/http_client.go` |
| 17 | `56682b6faf` | Merge pull request #14672 from cli/babakks/refre | 73.6s |  | 86.4s | miss 12.6s | miss 11.1s | miss 62.0s | `internal/config/config.go` |
| 18 | `abf0e0c25a` | Merge pull request #14673 from cli/babakks/refre | 20.0s |  | 51.6s | miss 5.2s | miss 4.3s | miss 41.4s | `pkg/cmd/auth/auth.go` |
| 19 | `6314d776e1` | Merge pull request #14717 from cli/babakks/refre | 73.1s |  | 86.2s | miss 12.6s | miss 11.0s | miss 61.9s | `internal/config/config.go` |
| 20 | `547bf81aa5` | Merge pull request #14745 from cli/babakks/refre | 48.1s | 123.0s | 69.5s | miss 9.1s | miss 7.8s | miss 51.9s | `internal/text/text.go` |
| 21 | `ae7b971597` | Merge pull request #14752 from cli/babakks/refre | 40.4s |  | 63.6s | miss 6.1s | miss 6.6s | miss 50.2s | `internal` |
| 22 | `99750e1576` | Merge pull request #14768 from cli/babakks/refre | 21.7s |  | 54.3s | miss 6.8s | miss 4.6s | miss 42.0s | `.quod/gomod/cache/download/github.com/cli/oauth/@v` |
| 23 | `7948e8056e` | Merge pull request #14782 from cli/babakks/skill | 16.0s |  | 50.3s | miss 4.9s | proving 4.0s | proving 40.7s | `-` |
| 24 | `5056c0a8ef` | Merge pull request #14784 from cli/tommaso-moro- | 18.0s |  | 50.8s | miss 5.2s | miss 4.1s | miss 40.8s | `pkg/cmd/skills/install/install.go` |
| 25 | `ed84a97a55` | Merge pull request #14791 from cli/bdehamer-depe | 24.2s | 123.4s | 56.6s | miss 8.1s | miss 5.1s | miss 42.8s | `.quod/gomod/cache/download/github.com/digitorus/pkcs7/@v` |
| 26 | `e87ed4c751` | Document extrepo installation on Debian (#14796) | 15.7s |  | 50.6s | miss 5.2s | proving 4.1s | proving 40.6s | `-` |
| 27 | `64d175234b` | Merge pull request #14798 from cli/babakks/polis | 17.2s |  | 51.7s | miss 5.3s | miss 4.3s | miss 41.4s | `pkg/cmd/auth/login/login.go` |
| 28 | `059f4975b5` | Merge pull request #14804 from martinajir/attach | 27.5s |  | 57.9s | miss 6.7s | miss 5.5s | miss 45.0s | `internal/attachments/client.go` |
| 29 | `4f2cfe92e3` | Merge pull request #14805 from cli/bagtoad/attac | 46.1s |  | 68.8s | miss 7.8s | miss 7.7s | miss 52.7s | `internal/attachments/client.go` |
| 30 | `63837fb778` | Merge pull request #14815 from cli/babakks/auth- | 17.1s | 124.5s | 51.7s | miss 5.4s | miss 4.5s | miss 41.1s | `pkg/cmd/auth/status/status.go` |
| 31 | `0186b810be` | Merge pull request #14801 from tj-smith47/remove | 74.2s |  | 87.8s | miss 13.2s | miss 11.5s | miss 62.5s | `api` |
| 32 | `9be5608274` | Merge pull request #14824 from cli/dependabot/gi | 15.8s |  | 51.2s | miss 5.2s | proving 4.2s | proving 41.0s | `-` |
| 33 | `5687d4ffe8` | Merge pull request #14837 from cli/williammartin | 15.9s |  | 48.4s | miss 5.3s | HIT 1.9s | miss 40.5s | `.github` |
| 34 | `93d24ce9ff` | Only save Go caches from trunk (#14840) | 15.8s |  | 48.3s | miss 5.3s | HIT 1.9s | proving 40.4s | `-` |
| 35 | `674516ff39` | Merge pull request #14854 from bobrik/ivan/norma | 16.1s | 123.5s | 51.3s | miss 5.3s | miss 4.3s | miss 41.0s | `pkg/cmd/extension/command_test.go` |
| 36 | `f7a06a18a9` | Merge pull request #14825 from cli/bagtoad/ghsa- | 15.8s |  | 51.1s | miss 5.4s | proving 4.2s | miss 40.9s | `.github` |
| 37 | `8553724d15` | Merge pull request #14858 from cli/bagtoad/priva | 17.7s |  | 52.3s | miss 5.6s | miss 4.7s | miss 41.3s | `acceptance/testdata/pr/pr-checkout-with-url-from-fork.txtar` |
| 38 | `30eb70ca63` | Merge pull request #14878 from cli/dependabot/go | 29.6s |  | 63.4s | miss 11.4s | miss 6.3s | miss 44.2s | `.quod/gomod/cache/download/go.opentelemetry.io/otel/@v` |
| 39 | `443baeba05` | Merge pull request #14355 from cli/williammartin | 70.6s |  | 85.6s | miss 12.7s | miss 11.2s | miss 61.0s | `git` |
| 40 | `9490d204a2` | Merge pull request #14879 from cli/bagtoad/show- | 13.9s | 123.7s | 52.4s | miss 5.6s | miss 4.5s | miss 41.5s | `git/client_test.go` |

commits: 40
hits go build ./...: 0/40
hits go vet ./...: 2/40
hits go test ./...: 0/40
total plain-warm 1356s, quod 2481s, saved vs warm -82.9%
median job plain-warm 18.7s quod 51.9s
cold sample n=8: median cold 123.4s, warm on same commits 19.0s, quod on same 52.2s
  extrapolated cold total = median cold x n = 4935s
step median warm  go build ./...   n=40 median    2.7s total   211.3s
step median warm  go vet ./...     n=40 median    1.2s total   123.4s
step median warm  go test ./...    n=40 median   14.5s total  1009.3s
step median warm  go mod download  n=40 median    0.0s total     5.4s
step median warm  checkout         n=40 median    0.2s total     6.8s
step median cold  go build ./...   n= 8 median   40.4s total   323.5s
step median cold  go vet ./...     n= 8 median   15.4s total   123.1s
step median cold  go test ./...    n= 8 median   67.4s total   539.0s
step median cold  go mod download  n= 8 median    0.0s total     0.2s
step median cold  checkout         n= 8 median    0.2s total     1.4s
step median quod  go build ./...   n=40 median    5.5s total   329.8s
step median quod  go vet ./...     n=40 median    4.4s total   243.7s
step median quod  go test ./...    n=40 median   41.4s total  1873.6s
step median quod  go mod download  n=40 median    0.7s total    33.7s
outcomes: {'go build ./...': {'miss': 40}, 'go vet ./...': {'miss': 32, 'proving': 6, 'replayed': 2}, 'go test ./...': {'miss': 34, 'proving': 6}}
quod process CPU s per step: median 2.13 max 5.11 total 259
quod process peak RSS MB: median 160 max 286
observer serving CPU (mounted) s: median 0.38 max 1.04
size state_bytes: after first commit 1594 MB, after last 7099 MB
size cas_bytes: after first commit 389 MB, after last 1066 MB
size warm_bytes: after first commit 837 MB, after last 5059 MB
size deps_bytes: after first commit 353 MB, after last 407 MB
size ledger_bytes: after first commit 16 MB, after last 566 MB
size plain_gocache_bytes: after first commit 843 MB, after last 5095 MB

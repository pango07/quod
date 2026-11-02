### Per commit

| # | upstream | subject | files touched | plain | quod | quod/plain | jobs replayed | ran | ran twice (network) | results equal | runs |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | `6bae955b` | Core: skip CloudWatch protocol detection tests on older boto | tests/test_cloudwatch, tests/test_core | 16.1 min | 23.4 min | 1.46 | 0/11 | 8 | 3 | 10/11 | [plain](https://github.com/pango07/quod-demo-moto/actions/runs/41499548026) [quod](https://github.com/pango07/quod-demo-moto/actions/runs/41499531937) |
| 2 | `dbe25569` | Core: Add CBOR test for the Java SDK | other_langs | 16.6 min | 20.0 min | 1.20 | 0/11 | 7 | 4 | 11/11 | [plain](https://github.com/pango07/quod-demo-moto/actions/runs/41503362109) [quod](https://github.com/pango07/quod-demo-moto/actions/runs/41503343681) |

### Which jobs replayed and which ran, per commit

| # | ran | ran twice (network) | replayed |
|---|---|---|---|
| 1 | batch, dynamodb, ec2, a-b, d-e, n-r, s-z, sqs | s3, c, f-m | - |
| 2 | batch, dynamodb, ec2, a-b, d-e, n-r, sqs | s3, c, f-m, s-z | - |

### Totals, commits 2-2 (1 commits)

| | plain | quod | quod/plain |
|---|---|---|---|
| sum of run wall times | 16.6 min | 20.0 min | 1.20 |
| median run wall time | 16.6 min | 20.0 min | 1.20 |
| sum of job times (runner minutes) | 16.4 min | 19.9 min | 1.21 |
| commits where quod was faster | | 0 of 1 | |

Jobs: 11; test step replayed 0, ran 7, ran twice (offline, then with the network) 4. pytest summaries equal to plain's on the same commit: 11/11.

### Per job (commits 2+)

| job | plain median | quod median | quod when replayed (median) | quod when ran (median) | replayed | ran | ran twice |
|---|---|---|---|---|---|---|---|
| batch | 37 s | 48 s | - | 48 s | 0 | 1 | 0 |
| dynamodb | 40 s | 55 s | - | 55 s | 0 | 1 | 0 |
| ec2 | 106 s | 94 s | - | 94 s | 0 | 1 | 0 |
| s3 | 47 s | 59 s | - | 59 s | 0 | 0 | 1 |
| services a-b | 84 s | 97 s | - | 97 s | 0 | 1 | 0 |
| services c | 124 s | 139 s | - | 139 s | 0 | 0 | 1 |
| services d-e | 151 s | 167 s | - | 167 s | 0 | 1 | 0 |
| services f-m | 115 s | 126 s | - | 126 s | 0 | 0 | 1 |
| services n-r | 94 s | 99 s | - | 99 s | 0 | 1 | 0 |
| services s-z | 136 s | 242 s | - | 242 s | 0 | 0 | 1 |
| sqs | 51 s | 65 s | - | 65 s | 0 | 1 | 0 |

### Quod against a hand-written path filter (commits 2+)

Jobs whose tests Quod could replay at all (not network-tainted): agree 0, Quod ran and a path filter would have skipped 7, a path filter ran and Quod replayed 0.
- commit 2, batch: 9.0s; would replay: no, quarantined by this run's divergence; **audit DIVERGED** from run 01M6ER7T: key quarantined (stdout differs from lin
- commit 2, dynamodb: 16.7s; would replay: no, quarantined by this run's divergence; **audit DIVERGED** from run 01M6ER61: key quarantined (stdout differs from li
- commit 2, ec2: 55.5s; would replay: no, quarantined by this run's divergence; **audit DIVERGED** from run 01M6ES3F: key quarantined (stdout differs from li
- commit 2, services a-b: 59.1s; would replay: yes, saving 0.0s (1 proofs); shadow audit: agrees with run 01M6EQW5
- commit 2, services d-e: 2m06s; would replay: yes, saving 0.0s (1 proofs); shadow audit: agrees with run 01M6ER9M
- commit 2, services n-r: 1m00s; would replay: yes, saving 0.0s (1 proofs); shadow audit: agrees with run 01M6EREQ
- commit 2, sqs: 26.8s; would replay: no, quarantined by this run's divergence; **audit DIVERGED** from run 01M6ES1C: key quarantined (stdout differs from li

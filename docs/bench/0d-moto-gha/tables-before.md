### Per commit

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

### Which jobs replayed and which ran, per commit

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

### Totals, commits 2-10 (9 commits)

| | plain | quod | quod/plain |
|---|---|---|---|
| sum of run wall times | 180.7 min | 159.2 min | 0.88 |
| median run wall time | 19.8 min | 16.5 min | 0.83 |
| sum of job times (runner minutes) | 177.0 min | 151.0 min | 0.85 |
| commits where quod was faster | | 8 of 9 | |

Jobs: 99; test step replayed 61, ran 11, ran twice (offline, then with the network) 27. pytest summaries equal to plain's on the same commit: 99/99.

### Per job (commits 2+)

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

### Quod against a hand-written path filter (commits 2+)

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

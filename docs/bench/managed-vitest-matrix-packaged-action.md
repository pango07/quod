The shipped optional Action passed the four-job packaged investigation on [run 42426110078](https://github.com/pango07/quod/actions/runs/42426110078), attempt 1. The fresh consumer authenticated and replayed all 108 candidates through the actual Actions cache service, skipped all 108 candidate compilations, and started zero managed test workers. It still ran one preflight compilation/worker, 12 native remainder instances, the original build and all 20 release tests. All 120 project/file/pool instances, API file states and 4,131 logical assertion states matched both independently executed controls and the pinned original native baseline: 4,130 passes and one skip. **`fullJobHit` is false.**

The [independent data audit](managed-vitest-matrix-packaged-action.json) passed with zero issues. Root and a separate reviewer produced byte-identical accepted v2 reports. Published JSON changes only the report schema: 1,175,157 bytes, SHA-256 `cbc5622c7d81d492fdb3f94c650553e1fd1b6f600c83930f6496b480cf612386`. The [standalone reader](../../scripts/bench-vitest/managed/matrix-packaged-action-audit.py) is 108281 bytes, SHA-256 `ea53fd6095c4e34c30beb98c29359ce299add241c4cc00018591050fa0044440`; its public adaptation changes only docstrings and output-schema names. It uses Python's standard library and read-only local Git objects. It performs no network requests or artifact code execution, project imports, installs, compilation or worker execution.

| Immutable identity | Value |
| --- | --- |
| Runtime Q | [`85c96d6c62c9252f3aa464a851f514348353d010`](https://github.com/pango07/quod/commit/85c96d6c62c9252f3aa464a851f514348353d010) |
| Built Action B | [`97caed0c9b9e998b5b1765c8fde21b4336d368f2`](https://github.com/pango07/quod-action/commit/97caed0c9b9e998b5b1765c8fde21b4336d368f2) |
| Workflow W | [`a5e129a55e8b5e3a93def4daf3ce593e27915998`](https://github.com/pango07/quod/commit/a5e129a55e8b5e3a93def4daf3ce593e27915998) |
| Original c11 | [`5eaa7e5160736054d53f8b71e2c522835eb427e0`](https://github.com/get-convex/convex-helpers/commit/5eaa7e5160736054d53f8b71e2c522835eb427e0) |
| Native baseline | Run `42406036816`, source `ff279ea5b59b359d26985f08f22e0749f9eccbde` |

The separate [Action build 42425271128](https://github.com/pango07/quod-action/actions/runs/42425271128) passed **51 unit checks**, with zero failures, skips, cancellations or todos. Its job ran 23:26:45–23:27:51 UTC on November 7, 2026: **66 seconds**. B has the sole parent `d0fc82d7cf071cb588d91e52055e977dc13a52fc` and changes seven generated `dist` files. The packaged proof uses B directly, without building an alternative Action implementation.

Each role checked out the unchanged c11 project at the repository root and ran **`npm ci`, `npm run build`, `npm run test`**. The last command remains `vitest run && npm run test:release`; release remains `node --test scripts/*.test.mjs`. The proof helper stays outside the project in `RUNNER_TEMP`. Only v0 adds default/JSON reporter evidence through an external preload; it adds no test filter or project-source change.

| Role | Exact UTC interval, November 7, 2026 | Complete job seconds | Modeled `ceil(seconds / 60)` minutes |
| --- | --- | ---: | ---: |
| Seed | 23:37:49–23:46:10 | 501 | 9 |
| Uncached managed control | 23:37:49–23:47:24 | 575 | 10 |
| Fresh consumer | 23:47:26–23:48:15 | 49 | 1 |
| Existing v0 compatibility | 23:37:49–23:41:37 | 228 | 4 |
| Standard `changes` helper | 23:37:49–23:37:56 | 7 | 1 |

API intervals include setup, install/build/test, evidence upload, Action post and cleanup; queue time is excluded. These are fresh development diagnostics, **not a paired original 12-commit benchmark**, a 50% result or a floor-plus-8-second result. The 501-second seed and 575-second control expose cold cost. The preceding SDK-only consumer took [78 seconds](https://github.com/pango07/quod/actions/runs/42423962148), slower than the [earlier 71-second SDK-only consumer](https://github.com/pango07/quod/actions/runs/42417359709); those jobs also differ from this complete build/test/release workflow. A historical c11 [v0 managed-47 job took 70 seconds](https://github.com/pango07/ci-bench/actions/runs/42397721165) in the [earlier cold window](managed-vitest-frozen-cold-window.md). That was a different generation and runner, **not a plain/native comparison**. **49 versus 70 seconds is descriptive history, not a controlled speedup claim.** No same-window floor was measured here; the original full-job objective remains open.

Seed and control each completed one **108-request closed compilation batch with the exact 218 named fences**, then 108 fresh managed workers and a full 120-instance native comparison. Recomputed totals are 145,810,161 bundle bytes, 35,081,677 certificate bytes and 29,187,279 identity bytes. Batch times were 161.1201 and 185.4791 seconds; separate-runner component times establish no causal optimization result. Consumer had no batch artifact, candidate build/certificate artifacts or managed test-worker directories. Its dispatcher took 20.8535 seconds, including 9.0651 seconds of current input validation and 3.5298 seconds of native remainder execution; complete job time remained 49 seconds.

The actual SDK seed made one restore miss and one acknowledged save, cache ID **`10177082981`**. Control made no restore/save calls. Consumer made one restore and zero saves; the returned service key exactly equals the acknowledged seed key:

```text
quod-managed-collection-v1-packaged-42426110078-matrix-b5ccc327e52ddab082a0d457210f6001ee9105a27f8f5f6b8730e82471b4ce46-9f975b81ce3f756481b93b1552739d6f1f49f3abfcc0f86e07be7b65aa95d3c7
```

Saved and restored pack bytes match: **12,909,251 bytes**, SHA-256 `9f975b81ce3f756481b93b1552739d6f1f49f3abfcc0f86e07be7b65aa95d3c7`, within unchanged 32 MiB pack and 2 MiB entry limits. The authenticated whole-pack digest binds 108 envelopes, bounded gzip receipts, reconstructed exact certificates, execution keys and raw responses. Fresh preparation, complete source facts, resolutions/globs/options, four pinned tool trees and 5,492 resolver requests bind current opaque proofs to those executions. Candidate keys alone grant no replay authority. Artifact copies are audit-only evidence, never cache input or authentication authority.

The native remainder has 12 instances and 184 passing assertion states. Replayed responses have 3,946 passes and one skip. Full project/file/pool/test-name/state multisets and actual API file states match both full native controls and the original baseline; repeated project instances are preserved rather than overwritten by path. Arbitrary native reporter timing metadata is not reconstructed. Initial Node/verifier and the finite workflow parent remain trusted; this samples a nonadversarial workspace rather than attesting an atomic snapshot or arbitrary host. Earlier install/build steps remain trusted, and the matrix session is consumed before Vitest dispatch; this establishes no general same-UID credential isolation.

The separate v0 role ran the original 47 managed assertions plus exactly 119 native project/file instances, with an exact worker response, passing v1 receipt and positive separate receipt-cache acknowledgement (`10176995641`). Its complete 4,131-state report multiset matches the baseline. Its existing formatter reports 120 file instances as aggregate suites and flattens managed assertion ancestor titles, so native aggregate/timing metadata equivalence is not claimed. The initial reader wrongly equated fresh-path pretty `selected-response.json` bytes with compact worker bytes. The accepted correction keeps **exact receipt/worker byte equality** and compares the selected document's full decoded data, matching the accepted window reader. Original failed reports remain preserved, SHA-256 `5e2d9008af758d8df962730dc4273f9144850d7d0603be4b72eaeedddc968204`; no producer change or gate rerun was needed.

Every role retained B's exact runtime tar: **51,099,830 bytes**, SHA-256 `0cba2f379b52d9159d4a1655b603ecca71ee8811992505ca40f6c64f0a8ae2f9`. Its 175,943,351 expanded bytes contain 76 nonroot members. The reader hashes members as data without extracting or executing them, checks all 39 top-level Q `.mjs`/package files and resolves precisely two internal symlinks and two internal hardlinks, rejecting missing/cyclic/escaping targets. The exact `50.index.js`, `835.index.js`, `index.js` rosters in each of `main`, `post`, `managed-bridge`, `matrix-bridge` cover **all 12 ncc executable file hashes**, compared with B's Git objects. Lazy chunks are not assumed irrelevant. Manifest SHA-256 is `1d0c057695665475012266640383cc4c72f12fc650da6e382542a95a207597d7`; the full graph and all hashes appear in the JSON report.

The explicit **run-only outer ZIP profile** is 134,217,728 compressed bytes, 500,000,000 expanded bytes, 67,108,864 bytes/file and 20,000 total file/directory paths. It accommodates the observed seed ZIP and retained 51 MB tar and does not change earlier SDK caps. Nested tar data has separate caps: 67,108,864 compressed bytes, 268,435,456 expanded bytes, 160,000,000 bytes/file and 1,000 paths. All bounds and API/source/job/archive pins are reader literals; artifact data cannot choose them.

| Proof role | Artifact ID | ZIP bytes | Expanded bytes | ZIP SHA-256 |
| --- | ---: | ---: | ---: | --- |
| Seed | 13595776702 | 103,666,042 | 390,297,858 | `33ba7bb49732b7f1278d8e1142852401899c9d288bc6c9698c776e49728e485e` |
| Control | 13594834631 | 90,224,895 | 336,148,837 | `70a2ed2909693572dd633f04ba4601f46df9317783a78cdc557eb48f12ef1657` |
| Consumer | 13595348242 | 64,809,426 | 120,917,426 | `171692e0f3335424af6f566413502ae83e2570e44632af1deac37fbc1cca752a` |
| v0 | 13595965179 | 51,605,188 | 58,892,342 | `d93a043bac0e2e7bad19f76383c3cbc844be4df0c816c80bd63e0fc7f86aedce` |
| Native baseline | 13587034973 | 36,186,785 | 198,696,464 | `f8e20f3a68cc3fbec7ce01a5ea90e8b50a1d558cbf08146a7fb049e242adc282` |

New roots contain 1,544 / 1,327 / 679 / 38 regular files. The reader checks whole ZIP bytes before parsing evidence, then independently binds every extracted byte and exact file/directory inventory, including implicit directories. It rejects outer symlinks/special entries, duplicate or NFC/casefold aliases, file/directory collisions, escaping/noncanonical paths, encryption, unreviewed compression and exceeded size/count bounds. Three additional Action-post logical-report artifacts are excluded from this four-proof audit.

Each role completed the same original 20 release tests, checked against unchanged c11 source literals and actual log results:

1. local release writes changelog/version/lockfile and pushes main without a tag
2. dirty trees are rejected before versioning
3. failed checks do not bump or commit a version
4. rejected pushes preserve the release commit for recovery and create no tags
5. watch follows this commit's CI run and opens npm after success
6. failed CI never opens the npm approval page
7. watch without local npm authentication prints manual approval instructions
8. alpha releases keep their prerelease identifier
9. CI uses the release commit even when ordinary commits follow it
10. CI rejects multiple version bumps in one push
11. CI does not release an ordinary push
12. release validation requires notes in the current version section
13. CI validates complete SemVer and chooses npm-compatible dist-tags
14. manual dispatch stages the selected branch tip or an earlier release commit
15. tag job creates the version tag at the staged SHA
16. tag retries accept the same commit without rewriting the ref
17. tag job refuses an existing tag pointing elsewhere
18. tag permission failures remain failures
19. release jobs use the planned commit and channel with separate credentials
20. releases are automatic only on main, with explicit dispatch and a fork guard

To reproduce, use trusted local checkouts containing the fixed Q/W/B/c11 Git objects. No dependency install or project command is needed. Fetch exact public source-data objects into existing checkouts:

```sh
QUOD_REPO=/absolute/path/to/quod
ACTION_REPO=/absolute/path/to/quod-action
PROJECT_REPO=/absolute/path/to/convex-helpers
git -C "$QUOD_REPO" fetch origin 85c96d6c62c9252f3aa464a851f514348353d010 a5e129a55e8b5e3a93def4daf3ce593e27915998 ff279ea5b59b359d26985f08f22e0749f9eccbde
git -C "$ACTION_REPO" fetch origin 97caed0c9b9e998b5b1765c8fde21b4336d368f2
git -C "$PROJECT_REPO" fetch origin 5eaa7e5160736054d53f8b71e2c522835eb427e0
```

The following recipe downloads exact raw API metadata and five original ZIPs while artifacts remain available. It reads literal authority from the trusted reader without importing it, verifies metadata and whole ZIP digests before bounded outer extraction, and never extracts the nested tar. Run from the Quod root with Python 3 and authenticated `gh`:

```sh
python3 -I -S - <<'PY'
import ast, concurrent.futures, hashlib, json, pathlib, stat, subprocess, tempfile, unicodedata, zipfile
reader = pathlib.Path('scripts/bench-vitest/managed/matrix-packaged-action-audit.py').resolve()
body = reader.read_bytes()
assert len(body) == 108281 and hashlib.sha256(body).hexdigest() == 'ea53fd6095c4e34c30beb98c29359ce299add241c4cc00018591050fa0044440'
wanted = {'RUN', 'WORKFLOW_SOURCE', 'ARTIFACTS', 'BASELINE', 'BASELINE_RUN', 'BASELINE_SOURCE', 'ARCHIVE_PROFILE'}
constants = {}
for node in ast.parse(body).body:
    if isinstance(node, ast.Assign) and len(node.targets) == 1 and isinstance(node.targets[0], ast.Name) and node.targets[0].id in wanted:
        constants[node.targets[0].id] = ast.literal_eval(node.value)
assert set(constants) == wanted
cap = constants['ARCHIVE_PROFILE']
out = pathlib.Path(tempfile.mkdtemp(prefix='quod-matrix-packaged-audit-')).resolve()
def api(route, *headers):
    return subprocess.check_output(['gh', 'api', *headers, 'repos/pango07/quod/' + route], timeout=60)
run = constants['RUN']
index = {'schema': 'quod-matrix-packaged-action-audit-inputs-v1', 'evidence': {}}
for key, route in [('runJson', f'actions/runs/{run}'), ('jobsJson', f'actions/runs/{run}/jobs?per_page=100'), ('artifactsJson', f'actions/runs/{run}/artifacts?per_page=100')]:
    target = out / (key + '.json')
    with target.open('xb') as stream:
        stream.write(api(route))
    index[key] = str(target)
for role, pin in {**constants['ARTIFACTS'], 'baseline': constants['BASELINE']}.items():
    artifact_id, size, digest = pin[:3]
    metadata_bytes = api(f'actions/artifacts/{artifact_id}')
    assert len(metadata_bytes) < 65536
    metadata = json.loads(metadata_bytes)
    assert metadata['id'] == artifact_id and metadata['size_in_bytes'] == size and metadata['digest'] == 'sha256:' + digest and not metadata['expired']
    expected_run = constants['BASELINE_RUN'] if role == 'baseline' else run
    expected_source = constants['BASELINE_SOURCE'] if role == 'baseline' else constants['WORKFLOW_SOURCE']
    assert metadata['workflow_run']['id'] == expected_run and metadata['workflow_run']['head_sha'] == expected_source
    if role != 'baseline':
        assert metadata['name'] == 'managed-matrix-packaged-' + role
    assert 0 < size < cap['compressedBytes']
    archive, root = out / (role + '.zip'), out / role
    ranges = [(start, min(size - 1, start + 2 * 1024 * 1024 - 1)) for start in range(0, size, 2 * 1024 * 1024)]
    def fetch(pair):
        start, end = pair
        chunk = api(f'actions/artifacts/{artifact_id}/zip', '-H', f'Range: bytes={start}-{end}')
        assert len(chunk) == end - start + 1
        return chunk
    whole = hashlib.sha256()
    with archive.open('xb') as stream, concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
        for chunk in pool.map(fetch, ranges):
            stream.write(chunk); whole.update(chunk)
    assert archive.stat().st_size == size and whole.hexdigest() == digest
    with (out / (role + '-artifact.json')).open('xb') as stream:
        stream.write(metadata_bytes)
    with zipfile.ZipFile(archive) as zipped:
        entries = zipped.infolist()
        assert 0 < len(entries) <= cap['paths'] and sum(row.file_size for row in entries) < cap['expandedBytes']
        seen, aliases, files, directories = set(), {}, set(), set()
        for entry in entries:
            name = entry.filename[:-1] if entry.is_dir() else entry.filename
            path = pathlib.PurePosixPath(name)
            assert name and path.as_posix() == name and name not in seen and not path.is_absolute()
            assert not any(part in ('', '.', '..') for part in name.split('/'))
            assert not any(char in entry.filename for char in ('\\', '\x00', '\r', '\n', ':'))
            assert len(entry.filename.encode()) <= 4096 and len(path.parts) <= 128
            assert not entry.flag_bits & 1 and entry.compress_type in (zipfile.ZIP_STORED, zipfile.ZIP_DEFLATED)
            assert stat.S_IFMT(entry.external_attr >> 16) in ((0, stat.S_IFDIR) if entry.is_dir() else (0, stat.S_IFREG))
            assert 0 <= entry.file_size <= cap['fileBytes']
            if entry.is_dir():
                assert entry.file_size == 0
            seen.add(name); (directories if entry.is_dir() else files).add(name)
            for parent in path.parents:
                if str(parent) != '.':
                    directories.add(str(parent))
            for part in [path, *path.parents]:
                spelling = str(part)
                if spelling == '.':
                    continue
                alias = unicodedata.normalize('NFC', spelling).casefold()
                assert alias not in aliases or aliases[alias] == spelling
                aliases[alias] = spelling
        assert not files & directories and len(files) + len(directories) <= cap['paths']
        root.mkdir(mode=0o700)
        assert not any(root.iterdir())
        zipped.extractall(root)
    index['evidence'][role] = {'root': str(root), 'archive': str(archive)}
target = out / 'index.json'
with target.open('x') as stream:
    stream.write(json.dumps(index, indent=2) + '\n')
print('Index:', target)
PY
```

Run the reader with the printed absolute index path:

```sh
python3 -I -S scripts/bench-vitest/managed/matrix-packaged-action-audit.py \
  --runtime-repository "$QUOD_REPO" --action-repository "$ACTION_REPO" \
  --workflow-repository "$QUOD_REPO" --project-repository "$PROJECT_REPO" \
  /absolute/path/to/index.json > audit.json
```

Expect exit 0, `passed: true`, `issues: []` and the published evidence/measurements. Index locations select no authority and do not enter admitted execution identity. Changed pins, bytes, paths/types/counts, source, receipts, proofs, rows/file states, release results or SDK acknowledgements fail the audit rather than broaden its profile.

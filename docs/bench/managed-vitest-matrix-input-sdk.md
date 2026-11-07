The early input-proof SDK experiment passed on [run 42417359709](https://github.com/pango07/quod/actions/runs/42417359709), attempt 1, at runtime source `668d25f7daadfb7a925568e948db3dae3e0b99ca`. A fresh consumer restored one authenticated collection, validated 108 current compile inputs, skipped all 108 candidate compilations, and replayed all 108 managed results. It executed the 12 native remainder instances. All 120 project/file/pool instances, API file states, and 4,131 logical test states matched both independently executed seed/control jobs and the retained native baseline: 4,130 passing states and one skipped state.

The [complete independent data audit](managed-vitest-matrix-input-sdk.json) passed with zero issues. It binds the original ZIP bytes to every extracted file, the reviewed source and job identities, the actual SDK acknowledgement and returned key, all 108 envelopes and receipts, reconstructed certificates, current input decisions, raw seed worker responses, native conformance, and complete logical/file-state multisets. Its JSON is 1,255,952 bytes with SHA-256 `82fd6d5054593a32a8e7a5ba3991e078edece81df7a2bf907dbdd61a9343758e`.

| Observation | Seed | Uncached managed control | Fresh consumer |
| --- | ---: | ---: | ---: |
| Complete GitHub job seconds | 674 | 421 | 71 |
| Modeled `ceil(job seconds / 60)` minutes | 12 | 8 | 2 |
| Dispatcher seconds | 623.9066 | 381.2328 | 27.62455 |
| Candidate compilation seconds | 123.6407 | 85.6152 | 0 |
| Current shared input validation seconds | no candidates | no candidates | 12.1454 |
| Managed test workers | 108 | 108 | 0 |
| Native instances executed | 120 | 120 | 12 |
| Native execution seconds | 98.9785 | 50.7414 | 4.8559 |
| SDK restore / save calls | 1 / 1 | 0 / 0 | 1 / 0 |

These are uncontrolled development diagnostics from separate fresh runners. The seed and control have substantially different execution times. Neither job is a plain-only baseline. The earlier SDK experiment's 97.1-second dispatcher and 139-second complete consumer job used a different profile; that comparison is also a development diagnostic, not a controlled speedup measurement. This profile has no new original 12-commit cold window. The original 50% window objective and floor-plus-8-second full-job objective remain open.

The prerequisite job took 104 seconds and passed 142 checks across 13 real dispatcher cases, including the outer-cleanup-error case where passing native fallback must preserve the earlier managed failure. It also passed 18 envelope/proof checks and 152 legacy receipt checks. Authentication-free fixture cases make no real SDK claim; the three subsequent jobs exercise the actual service.

The seed made one cold restore miss and one acknowledged save, cache ID `10174799873`. The fresh consumer's one restore returned the exact saved service key:

```text
quod-managed-collection-v1-matrix-input-gate-42417359709-4ff5875a172857aa2371a650ea5bb32755ae17bd100ccf72b5865a73539718df-62293d5aac552f4b7ef5cf6a7b5c156e5703440f7f2384ea5c75f56fc9eb6f33
```

The saved and restored collection bytes are identical: 12,891,279 bytes, SHA-256 `62293d5aac552f4b7ef5cf6a7b5c156e5703440f7f2384ea5c75f56fc9eb6f33`, within the existing 32 MiB pack limit. All 108 entries fit the unchanged 2 MiB entry limit. Bounded gzip envelopes carry 31,428,639 bytes of exact raw receipts; reconstructing certificates from their authenticated compile identities yields 35,084,701 bytes of exact certificates without duplicating those identities in the pack. Candidate routing keys supply no execution authority. The consumer still needs the externally authenticated envelope bytes, fresh current preparation and input validation, an opaque validation proof, and exact ledger execution identity. The audit binds 5,492 current resolver queries to those decisions. Artifact copies are audit evidence and never supply replay authority.

The consumer had no managed test worker directories and no candidate build metadata. One preflight compilation and worker still ran, as did native preparation, fresh resolution, and the 12-instance native remainder. `fullJobHit` is **false**. Native timing and arbitrary reporter metadata are not reconstructed; actual API file states and the complete project/file/pool/test-name/state multiset are preserved. The logical digest is `b8695e3aa26577da2408fed856ff8bd45b2dfca80c5ef7b21c72a9b7284ce312`.

The finite input-proof scope remains the reviewed compiler and four pinned tool package trees, exact API routes, a trusted initial Node/verifier and parent host, and the reviewed configuration profile. Validation samples full source bytes, current resolutions and glob results with the recorded physical geometry. It does not establish an atomic snapshot against an adversarial writer or arbitrary host/configuration hooks. Production enablement and broader source/configuration coverage are not established here.

To reproduce the audit, use a repository checkout containing this report and [the independent reader](../../scripts/bench-vitest/managed/matrix-input-sdk-audit.py). The reader is 65,619 bytes with SHA-256 `c0da19e6da607efd70b3b61335d727650cd166689a7d7b0106b1b08d0afbf5ff`. Its pinned runtime source must be available as a local Git object. No project dependencies, installs, worker execution, private downloader, or cache-service access are needed. GitHub artifact downloads require access while the retained artifacts remain available; the reader itself performs no network requests.

| Role | Original artifact ID | ZIP bytes | ZIP SHA-256 |
| --- | ---: | ---: | --- |
| Fixtures | 13592117437 | 5,776,322 | `13cd5633a1ddc5a40e8c0002bad532b030b3917796c3f28171a612d21b3ed7c4` |
| Seed | 13592248181 | 52,550,323 | `d6c16faeed6797fae8fe7199a955fb31e5502ac55358eff4cfe1137cb4c32987` |
| Control | 13592790300 | 39,160,525 | `5c5bd7af0f8939087b21f9f136e35fd421877b22e6a7db32ed6d0faa44b6dc9c` |
| Consumer | 13592293074 | 13,724,540 | `264bacf9a870b867cd6f43291284954114f770f1fe746e73c682761e30390ab8` |
| Native baseline, run 42406036816 | 13587034973 | 36,186,785 | `f8e20f3a68cc3fbec7ce01a5ea90e8b50a1d558cbf08146a7fb049e242adc282` |

The baseline source is `ff279ea5b59b359d26985f08f22e0749f9eccbde`. The transport Action is pinned to `02f65151206f3a12d6f17e3b3d1831b704f28a0b`; its collection source SHA-256 is `1b4aa76730aee3fd2984da6ed79a15eabe40c21d9977077ce2ff64ab3d5473dc` and auth source SHA-256 is `dab0b9d4a7d8b54d3cde5791d0d0cbc2694bf2b7a00598d1f49a67ffe253de22`. The reader independently pins each artifact ID, job ID/name, exact start/end timestamps, source, run attempt, and distinct execution runners. The four complete job times above come from the API start/end timestamps and include setup, install, execution, evidence upload, and cleanup.

The following recipe downloads the original five ZIPs and three raw API metadata files, verifies the exact ZIP pins, checks finite extraction bounds and path/type aliases before extraction, and constructs the reader's index. Each extracted tree is created empty and receives only its own verified archive. Run it from the repository root with Python 3 and authenticated `gh` available:

```sh
git fetch origin 668d25f7daadfb7a925568e948db3dae3e0b99ca
git cat-file -e 668d25f7daadfb7a925568e948db3dae3e0b99ca^{commit}
python3 - <<'PY'
import ast, hashlib, json, pathlib, stat, subprocess, tempfile, unicodedata, zipfile

reader = pathlib.Path('scripts/bench-vitest/managed/matrix-input-sdk-audit.py').resolve()
reader_bytes = reader.read_bytes()
assert len(reader_bytes) == 65619
assert hashlib.sha256(reader_bytes).hexdigest() == 'c0da19e6da607efd70b3b61335d727650cd166689a7d7b0106b1b08d0afbf5ff'
# Read literal constants from the trusted reader, without importing any code.
constants = {}
wanted = {'RUN', 'ARTIFACTS', 'BASELINE', 'MAX_ARCHIVE', 'MAX_EXPANDED', 'MAX_FILE', 'MAX_PATHS'}
def literal(node):
    if isinstance(node, ast.BinOp) and isinstance(node.op, ast.Mult):
        return literal(node.left) * literal(node.right)
    return ast.literal_eval(node)
for node in ast.parse(reader_bytes).body:
    if isinstance(node, ast.Assign) and len(node.targets) == 1 and isinstance(node.targets[0], ast.Name):
        if node.targets[0].id in wanted:
            constants[node.targets[0].id] = literal(node.value)
assert set(constants) == wanted
out = pathlib.Path(tempfile.mkdtemp(prefix='quod-matrix-input-audit-')).resolve()
api = lambda route: subprocess.check_output(['gh', 'api', 'repos/pango07/quod/' + route])
run = constants['RUN']
index = {'schema': 'quod-managed-matrix-input-sdk-audit-inputs-v1', 'evidence': {}}
for key, route in [('runJson', f'actions/runs/{run}'),
                   ('jobsJson', f'actions/runs/{run}/jobs?per_page=100'),
                   ('artifactsJson', f'actions/runs/{run}/artifacts?per_page=100')]:
    target = out / (key + '.json')
    target.write_bytes(api(route))
    index[key] = str(target)
for role, pin in {**constants['ARTIFACTS'], 'baseline': constants['BASELINE']}.items():
    artifact_id, size, digest = pin[:3]
    metadata = json.loads(api(f'actions/artifacts/{artifact_id}'))
    assert metadata['id'] == artifact_id and metadata['size_in_bytes'] == size
    assert metadata['digest'] == 'sha256:' + digest and not metadata['expired']
    assert 0 < size <= constants['MAX_ARCHIVE']
    archive, root = out / (role + '.zip'), out / role
    body = api(f'actions/artifacts/{artifact_id}/zip')
    assert len(body) == size and hashlib.sha256(body).hexdigest() == digest
    with archive.open('xb') as handle:
        handle.write(body)
    with zipfile.ZipFile(archive) as zipped:
        entries = zipped.infolist()
        assert 0 < len(entries) <= constants['MAX_PATHS']
        assert sum(e.file_size for e in entries) <= constants['MAX_EXPANDED']
        seen, aliases, files, directories = set(), {}, set(), set()
        for entry in entries:
            name = entry.filename.rstrip('/')
            path = pathlib.PurePosixPath(name)
            assert name and name not in seen and not path.is_absolute()
            assert not any(p in ('', '.', '..') for p in name.split('/'))
            assert not any(c in entry.filename for c in ('\\', '\0', '\r', '\n', ':'))
            assert len(entry.filename.encode()) <= 4096 and len(path.parts) <= 128
            assert not entry.flag_bits & 1
            assert entry.compress_type in (zipfile.ZIP_STORED, zipfile.ZIP_DEFLATED)
            kind = stat.S_IFMT(entry.external_attr >> 16)
            assert kind in ((0, stat.S_IFDIR) if entry.is_dir() else (0, stat.S_IFREG))
            assert 0 <= entry.file_size <= constants['MAX_FILE']
            if entry.is_dir():
                assert entry.file_size == 0
            seen.add(name)
            (directories if entry.is_dir() else files).add(name)
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
        assert not files & directories
        assert len(files) + len(directories) <= constants['MAX_PATHS']
        root.mkdir(mode=0o700)
        assert not any(root.iterdir())
        zipped.extractall(root)
    index['evidence'][role] = {'root': str(root), 'archive': str(archive)}
target = out / 'index.json'
target.write_text(json.dumps(index, indent=2) + '\n')
print('Index:', target)
print('Run: python3', reader, target, '> audit.json')
PY
```

Execute the printed reader command. It checks both the original ZIP and the exact extracted tree for every role, including all implicit directories and file bytes; metadata paths in the index supply locations rather than authority. Expect `passed: true` and `issues: []`. The result should reproduce the published audit's verified evidence and measurements. Local index locations do not affect the admitted execution identity. No artifact code is imported or executed.

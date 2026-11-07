The batch-certificate diagnostic passed on [run 42419977028](https://github.com/pango07/quod/actions/runs/42419977028), attempt 1, at source `615fe7303a7566bddb58227827604e65173fb694`. A reserved sequential batch retained full validation fences around every compilation and capture, checked all prior source/glob observations before publication, and released actual build provenance only after its final guard. This API remains unwired to receipts, replay, and the dispatcher.

The [independent data audit](managed-vitest-batch-certificate.json) passed with zero issues. It binds the four original ZIPs to every extracted file, the pinned controller sources, the original check inventories, current native and managed outcomes, and actual bundle, identity, certificate, resolution, and execution-key bytes. The public reader is 73,577 bytes with SHA-256 `85d36431bf4c7045e47965638197715900ebc2162104a4a5edd5c4fb90b7826e`; the published audit JSON is 963,437 bytes with SHA-256 `6807a9637002a1f626c4d2cb835ee8361281e1df97299cca6d1411bf4487d95f`.

The unchanged regression inventory passed: 55 compiler checks; 213 strict input/API check records with 189 distinct names; 142 assertions across 13 dispatcher cases; 18 input-proof checks; and 152 legacy ledger checks. The reader pins the exact check-name multisets, including the deliberate repeated names in the original 213-record probe. The batch probe added 468 passing checks and 18 fresh-process cases covering cold initialization, reservation/reentry, malformed requests, source/resolution/glob changes, tool/API/environment mutation, late failure, quarantine, and poisoning. It also checked that test source remained unevaluated in each compiler process.

All 120 actual project/file/pool instances and 4,131 native test states matched the retained native baseline, including repeated projects and API file states. The 108 passing managed instances preserved their complete names/states: 3,946 passing and one skipped state. Twelve instances remained ordinary. The strict-versus-batch sample matched all 12 raw bundles, full identities, and exact certificates. The full batch matched all 108 current managed bundles and identities, strict recaptures, fresh opaque-proof outcomes, and complete execution keys, with 5,492 current resolver requests. The two batch traces contained exactly 26 and 218 full fences. The compiler remained SHA-256 `a87f1f8fafea2e0f1b4a1e299199f5581172a2fe90f59882d3c64eb50f68acaf`; the batch verifier was `f01cd9a55f6eceb913c5b402481fd6b6dd7266191d8182f97ca60f158765c703`.

| Retained interval | Seconds |
| --- | ---: |
| Complete focused GitHub job, 22:18:23–22:40:51 UTC | 1,348 |
| Complete current native120/managed108 matrix step, 22:24:05–22:31:45 UTC | 460 |
| Complete batch-comparison step, 22:31:45–22:40:26 UTC | 521 |
| Strict 12 compilation/capture timer, including pair writes | 33.623 |
| Batch 12 compilation/capture timer, before pair writes | 20.025 |
| Batch 108 compilation/capture timer | 187.076 |
| Subsequent shared validation of 108 certificates | 14.974 |

These are fixed-order development diagnostics. The strict-12 timer includes retained pair writes; the batch timers stop before subsequent recapture, proof comparison, and retention. Earlier work also warms caches. Their ratio is not a paired wall-time speedup measurement. No general, complete-job, or original 12-commit window speedup follows from these timers. The focused job models 23 rounded minutes; the existing five-second `changes` helper job models one more minute. Public-runner compute was $0; the modeled 24 minutes are not an actual charge. The original 50% cost objective and floor-plus-8-second full-job objective remain open.

Private WeakMap provenance, reservation, and quarantine are properties of the reviewed controller and its retained rejection cases; a data reader cannot reconstruct those private states. The scope remains a trusted initial Node/verifier/parent host, the reviewed finite configuration and four pinned tool trees, and a sampled nonadversarial workspace. This establishes no atomic snapshot against an adversarial writer, arbitrary host/configuration-hook coverage, new replay authority, or production enablement. The successful runtime gate is unchanged; reader diagnostics remain separate from runtime outcomes.

| Evidence role | Artifact ID | ZIP bytes | ZIP SHA-256 |
| --- | ---: | ---: | --- |
| Controls | 13593448454 | 25,190,521 | `f830701dd6bef44de359fb0dd437b05bc94e7f402a52f8ba299cb7989c67907e` |
| Current matrix | 13593229892 | 34,835,623 | `d7170f6c96527c90a4b6f35a53b737f60048b7b9547ab1a0669d82e646b593bb` |
| Batch pairs | 13594245657 | 49,115,068 | `94b3ac392d6d4e451555f7144184c352792ffd38c6aa75df6e93efc912a4cb46` |
| Native baseline, run 42406036816 | 13587034973 | 36,186,785 | `f8e20f3a68cc3fbec7ce01a5ea90e8b50a1d558cbf08146a7fb049e242adc282` |

The baseline source is `ff279ea5b59b359d26985f08f22e0749f9eccbde`. The original 213-record probe comes from Git object `766ff388f1dcc3305d43a14e99655404dc01b596`, with SHA-256 `3c6cf9be9cf705322ef9768da2dc5dc5a68e0369ba75be4d2817aa27613aa894`. The reader independently pins the successful focused job ID/name, exact timestamps, source, attempt, artifact metadata, and all four ZIP digests. Each role is limited to a 100 MB ZIP, 500 MB expanded data, 32 MiB per file, and 20,000 paths including implicit directories, with path/type/alias checks before extraction.

To reproduce, use a trusted repository checkout containing this report and [the public data-only reader](../../scripts/bench-vitest/managed/batch-certificate-audit.py), Python 3, and authenticated `gh` with artifact access while these retained artifacts remain available. No project dependencies, install, build, worker, cache service, or private downloader are needed. The reader makes no network requests and never imports or executes artifact code. Fetch both source Git objects, then run this complete download/validation/index recipe from the repository root:

```sh
git fetch origin 615fe7303a7566bddb58227827604e65173fb694
git fetch origin 766ff388f1dcc3305d43a14e99655404dc01b596
git cat-file -e 615fe7303a7566bddb58227827604e65173fb694^{commit}
git cat-file -e 766ff388f1dcc3305d43a14e99655404dc01b596^{commit}
python3 - <<'PYTHON'
import ast, hashlib, json, pathlib, stat, subprocess, tempfile, unicodedata, zipfile

repository = pathlib.Path.cwd().resolve()
reader = repository / 'scripts/bench-vitest/managed/batch-certificate-audit.py'
reader_bytes = reader.read_bytes()
assert len(reader_bytes) == 73577
assert hashlib.sha256(reader_bytes).hexdigest() == '85d36431bf4c7045e47965638197715900ebc2162104a4a5edd5c4fb90b7826e'
# Read pinned literal constants without importing the reader or artifact code.
wanted = {'RUN', 'SOURCE', 'ARTIFACTS', 'BASELINE', 'BASELINE_RUN', 'BASELINE_SOURCE',
          'MAX_ARCHIVE', 'MAX_EXPANDED', 'MAX_FILE', 'MAX_PATHS'}
constants = {}
def literal(node):
    if isinstance(node, ast.BinOp) and isinstance(node.op, ast.Mult):
        return literal(node.left) * literal(node.right)
    if isinstance(node, (ast.Tuple, ast.List)):
        return tuple(literal(value) for value in node.elts)
    return ast.literal_eval(node)
for node in ast.parse(reader_bytes).body:
    if not isinstance(node, ast.Assign) or len(node.targets) != 1:
        continue
    target = node.targets[0]
    if isinstance(target, ast.Name) and target.id in wanted:
        constants[target.id] = literal(node.value)
    elif isinstance(target, ast.Tuple):
        values = literal(node.value)
        for name, value in zip(target.elts, values):
            if isinstance(name, ast.Name) and name.id in wanted:
                constants[name.id] = value
assert set(constants) == wanted
out = pathlib.Path(tempfile.mkdtemp(prefix='quod-batch-certificate-audit-')).resolve()
api = lambda route: subprocess.check_output(['gh', 'api', 'repos/pango07/quod/' + route], timeout=60)
run = constants['RUN']
index = {'schema': 'quod-managed-batch-certificate-audit-inputs-v1', 'evidence': {}}
for key, route in [('runJson', f'actions/runs/{run}'),
                   ('jobsJson', f'actions/runs/{run}/jobs?per_page=100'),
                   ('artifactsJson', f'actions/runs/{run}/artifacts?per_page=100')]:
    target = out / (key + '.json')
    with target.open('xb') as handle:
        handle.write(api(route))
    index[key] = str(target)
for role, pin in {**constants['ARTIFACTS'], 'baseline': constants['BASELINE']}.items():
    artifact_id, size, digest = pin
    metadata = json.loads(api(f'actions/artifacts/{artifact_id}'))
    assert metadata['id'] == artifact_id and metadata['size_in_bytes'] == size
    assert metadata['digest'] == 'sha256:' + digest and not metadata['expired']
    expected_run = constants['BASELINE_RUN'] if role == 'baseline' else run
    expected_source = constants['BASELINE_SOURCE'] if role == 'baseline' else constants['SOURCE']
    assert metadata['workflow_run']['id'] == expected_run
    assert metadata['workflow_run']['head_sha'] == expected_source
    if role != 'baseline':
        assert metadata['name'] == 'managed-batch-certificate-' + role
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
            name = entry.filename[:-1] if entry.is_dir() else entry.filename
            path = pathlib.PurePosixPath(name)
            assert name and name not in seen and not path.is_absolute() and path.as_posix() == name
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
        assert len(files) + len(directories) + 1 <= constants['MAX_PATHS']
        root.mkdir(mode=0o700)
        assert not any(root.iterdir())
        zipped.extractall(root)
    index['evidence'][role] = {'root': str(root), 'archive': str(archive)}
target = out / 'index.json'
with target.open('x') as handle:
    handle.write(json.dumps(index, indent=2) + '\n')
print('Index:', target)
print('Run: python3', reader, '--repository', repository, target, '> audit.json')
PYTHON
```

Execute the printed reader command. Its index supplies locations, not authority. Expect `passed: true` and `issues: []`, with the verified evidence and measurements reproduced. The archive and exact extracted tree are both read and hashed for every role; no artifact code is executed. Runtime dependencies and the initial failed reader diagnostic are not part of the published data.

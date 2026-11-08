# Packaged property47 Action proof

[Gate 42433432522](https://github.com/pango07/quod/actions/runs/42433432522)
passed four fresh packaged Action jobs. Two independent data audits then passed
with identical output: 359,241 bytes, SHA256
`fd78a61805821c156669d9682bb5465966e492646648c2eacd86f77690d8657e`.
The [complete report](managed-vitest-property47-packaged-action.json) retains the
ZIP inventories, source and executable hashes, SDK acknowledgment, current-input
proof, receipt, managed/native states and complete job costs.

The consumer restored one SDK-authenticated receipt, validated its current
inputs using 216 fresh resolver requests, and replayed all 47 original property
assertions. It compiled no candidate and started no candidate worker. Its 119
native instances and the replay produced the original 120 instances and 4,131
logical test states, including exact project/file/pool/name/state multiplicity
and API file states. These match both fresh controls and the pinned full-native
baseline. A current preflight compilation/worker, native remainder and 20 release
assertions still ran: `fullJobHit` is false.

| Fresh role | Complete job seconds | Modeled minutes | Work |
|---|---:|---:|---|
| seed |185|4| One47-assertion managed worker, native120, one positive SDK save |
| control |334|6| One47-assertion managed worker, native120, zero receipt SDK operations |
| consumer |55|1| One authenticated replay, zero candidate compiles/workers, native119 |
| old v0 compatibility |228|4| Original47-assertion managed path and exact native119 remainder |
| existing changes helper |5|1| Workflow helper, counted separately |

These are complete GitHub job intervals, including setup, install, build, test,
release tail, evidence uploads and Action post work. The consumer's 21.473s
internal dispatcher interval includes 1.711s input validation and 16.974s native
adapter execution; those intervals have different scope from its 55s complete
job. Seed/control each retained one compilation request and the exact four
adjacent full fences. Both fresh managed responses and the authenticated replay
retain the same passing 47-row response. Every role ran the unchanged
`npm ci`, `npm run build`, `npm run test` command sequence and exact 20 release
test names. The old v0 job also preserved its raw receipt/worker/native119/
merged120 chain.

This is a four-role integration diagnostic. It establishes no original 48-job
window result, controlled causal speedup, 50% objective, or full-hit floor-plus-
eight result. The earlier packaged108 consumer's 49s and these jobs ran on
different runners and are not a controlled comparison. The finite reviewed
configuration, trusted initial Node/verifier/parent and sampled workspace scope
remain explicit; this is not arbitrary-host or active-writer atomic attestation.

## Source and transport pins

| Input | Exact Git commit |
|---|---|
| workflow W |`2893a9c0d222b22f22ff2d4d5a5431618893d7ed`|
| built Action B |`0520117d99379472cd33dbb777f852d8a189a20a`|
| runtime Q |`f67fe14033de76f0b26b436cef4e5d50b0af1ee3`|
| original project c11 |`5eaa7e5160736054d53f8b71e2c522835eb427e0`|
| full-native baseline ff |`ff279ea5b59b359d26985f08f22e0749f9eccbde`|

The finite candidate policy is
`reviewed-original-property47-single-instance-v1-research`, exact instance
`ccda7135f8d1ede102763427b3efabcef8130aaed24822ce92a5ec7ef94a05b3`:
`convex-helpers / packages/convex-helpers/server/stream.property.test.ts / forks`.
The profile is bound in both collection scope and keyed supervisor policy;
it changes no original test selection.

Actual SDK save cache ID `10178910579` authenticated the 103,473-byte collection,
SHA256 `0ddce69d32ebb4794337fbf1ecf0473ee763168f8c3f35ec80f8b39f9437abfc`.
The consumer's returned service key matches that positive seed save exactly.
The reader binds whole pack→entry SHA→compressed envelope→exact certificate/
receipt bytes→fresh current-input decision→current execution key→original raw
worker response/native conformance. A passing payload or embedded checksum does
not supply transport authority. Control performed zero restore/save/entry
operations; consumer published nothing. Credentials were consumed before Vitest
dispatch and absent outside SDK calls under the reviewed finite workflow scope.

B's 51,106,902-byte runtime archive has SHA256
`519c9ad37472ddb50b428b9d2dcd43ef930add1f203a7c440abc5ce7e4a442b8`.
The reader compares all 39 top-level runtime `.mjs` members plus `package.json`
and `package-lock.json` to trusted Q Git bytes, and all 12 ncc executable chunks
across main/post/v0-bridge/matrix-bridge to trusted B Git bytes. Its tar inventory
has 78 named members and 175,972,317 expanded regular-file bytes. Safe internal
hardlinks and symlinks are resolved as data; no archive code or binary executes.

| Proof ZIP | Artifact ID | Bytes | SHA256 |
|---|---:|---:|---|
| seed |13598940155|51795163|`df880336795d43d447878a444784a50abe3956ab33df532e494af8ce24777135`|
| control |13597958625|51686608|`134a7c5e04d5ae4874b8a9194a3f5df17a39026a014af107e6bf530986920910`|
| consumer |13598364334|51467896|`2f1b3421ee38ffcba8e9d769d88ae1445ed3cfacebd4c061e0919569ee755136`|
| v0 |13597989907|51612465|`f48a0c9dfbe6a8699e5f56095a549b776db305ce2ad926a19ffced7270974022`|
| ff baseline |13587034973|36186785|`f8e20f3a68cc3fbec7ce01a5ea90e8b50a1d558cbf08146a7fb049e242adc282`|

The ff baseline comes from [run 42406036816](https://github.com/pango07/quod/actions/runs/42406036816).
Its exact 1,903,888-byte `native-selection.json` has SHA256
`1d3ceb0a7520632c23474769721d608875eb4b9cfed6c6d248f61b89c946ced1`.
All proof ZIPs and every extracted file/directory are independently rebound by
the reader, including implicit directories, byte hashes, path/case/NFC aliases,
entry types and exact inventory equality.

## Reproduce as data only

Use Git, GitHub CLI with artifact-read access, and Python 3.11+ with only its
standard library. Artifact downloads require the original artifacts to remain
available. Run from a checkout containing the published reader. No package
install, build, workload test, server, imported artifact code, or installed
runtime binary is needed. The source repositories are used only for Git object
reads; keep the extracted trees empty before the bounded extraction below.

```sh
set -euo pipefail
property47_audit_dir=$(mktemp -d)
cp scripts/bench-vitest/managed/property47-packaged-action-audit.py "$property47_audit_dir/reader.py"
export PROPERTY47_AUDIT_DIR="$property47_audit_dir"
python3 -I -S - <<'PY'
import hashlib, os, pathlib
p = pathlib.Path(os.environ['PROPERTY47_AUDIT_DIR']) / 'reader.py'
assert hashlib.sha256(p.read_bytes()).hexdigest() == 'a97846343f7bb515207935198050072cf28249b0ef22059063a6c4d4d0fbbad9'
PY
git clone --no-checkout https://github.com/pango07/quod.git "$property47_audit_dir/quod"
git -C "$property47_audit_dir/quod" fetch origin \
  2893a9c0d222b22f22ff2d4d5a5431618893d7ed \
  f67fe14033de76f0b26b436cef4e5d50b0af1ee3 \
  ff279ea5b59b359d26985f08f22e0749f9eccbde
git clone --no-checkout https://github.com/pango07/quod-action.git "$property47_audit_dir/action"
git -C "$property47_audit_dir/action" fetch origin 0520117d99379472cd33dbb777f852d8a189a20a
git clone --no-checkout https://github.com/get-convex/convex-helpers.git "$property47_audit_dir/project"
git -C "$property47_audit_dir/project" fetch origin 5eaa7e5160736054d53f8b71e2c522835eb427e0
python3 -I -S - <<'PY'
# Fetch only pinned data. Verify API ID/source/size/digest before ZIP transfer;
# verify every ZIP bound/name/type before creating its extracted directory.
import concurrent.futures, hashlib, json, os, pathlib, stat, subprocess, unicodedata, zipfile
base = pathlib.Path(os.environ['PROPERTY47_AUDIT_DIR'])
run, source = 42433432522, '2893a9c0d222b22f22ff2d4d5a5431618893d7ed'
proofs = {
 'seed': (13598940155, 51795163, 'df880336795d43d447878a444784a50abe3956ab33df532e494af8ce24777135'),
 'control': (13597958625, 51686608, '134a7c5e04d5ae4874b8a9194a3f5df17a39026a014af107e6bf530986920910'),
 'consumer': (13598364334, 51467896, '2f1b3421ee38ffcba8e9d769d88ae1445ed3cfacebd4c061e0919569ee755136'),
 'v0': (13597989907, 51612465, 'f48a0c9dfbe6a8699e5f56095a549b776db305ce2ad926a19ffced7270974022'),
 'baseline': (13587034973, 36186785, 'f8e20f3a68cc3fbec7ce01a5ea90e8b50a1d558cbf08146a7fb049e242adc282'),
}
def request(endpoint, extra=()):
 return subprocess.run(['gh', 'api', *extra, endpoint], capture_output=True, check=True, timeout=60).stdout
def create(name, body):
 with (base/name).open('xb') as f: f.write(body)
for name, endpoint in {
 'run.json': f'repos/pango07/quod/actions/runs/{run}',
 'jobs.json': f'repos/pango07/quod/actions/runs/{run}/jobs?per_page=100',
 'artifacts.json': f'repos/pango07/quod/actions/runs/{run}/artifacts?per_page=100',
}.items():
 body = request(endpoint); assert 0 < len(body) <= 8*1024*1024; create(name, body)
evidence = {}
for role, (aid, size, digest) in proofs.items():
 metadata_body = request(f'repos/pango07/quod/actions/artifacts/{aid}')
 assert 0 < len(metadata_body) <= 65536
 metadata = json.loads(metadata_body)
 expected_run = 42406036816 if role == 'baseline' else run
 expected_source = 'ff279ea5b59b359d26985f08f22e0749f9eccbde' if role == 'baseline' else source
 expected_name = 'managed-matrix' if role == 'baseline' else 'managed-property47-packaged-'+role
 assert metadata['id'] == aid and metadata['name'] == expected_name and metadata['expired'] is False
 assert metadata['workflow_run']['id'] == expected_run and metadata['workflow_run']['head_sha'] == expected_source
 assert metadata['size_in_bytes'] == size <= 128*1024*1024 and metadata['digest'] == 'sha256:'+digest
 create(role+'-artifact-api.json', metadata_body)
 ranges = [(start, min(size-1, start+2*1024*1024-1)) for start in range(0,size,2*1024*1024)]
 def fetch(pair):
  start,end = pair; body = request(f'repos/pango07/quod/actions/artifacts/{aid}/zip', ('-H',f'Range: bytes={start}-{end}'))
  assert len(body) == end-start+1; return body
 first = fetch(ranges[0])
 with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
  body = first+b''.join(pool.map(fetch, ranges[1:]))
 assert len(body) == size and hashlib.sha256(body).hexdigest() == digest
 create(role+'.zip',body)
 destination = base/role
 assert not destination.exists() and not destination.is_symlink()
 with zipfile.ZipFile(base/(role+'.zip')) as z:
  entries=z.infolist(); assert 0 < len(entries) <= 20000
  seen, aliases, files, dirs, expanded = set(), {}, set(), set(), 0
  for entry in entries:
   name=entry.filename.rstrip('/'); p=pathlib.PurePosixPath(name)
   assert name and not p.is_absolute() and all(part and part not in {'.','..'} for part in name.split('/'))
   assert p.as_posix()==name and len(name.encode()) <= 4096 and len(p.parts) <= 128
   assert not any(char in name for char in ('\\','\0','\r','\n',':')) and name not in seen
   seen.add(name)
   assert not entry.flag_bits&1 and entry.compress_type in (zipfile.ZIP_STORED,zipfile.ZIP_DEFLATED)
   kind=stat.S_IFMT(entry.external_attr>>16)
   assert kind in ((0,stat.S_IFDIR) if entry.is_dir() else (0,stat.S_IFREG))
   assert 0 <= entry.file_size <= 64*1024*1024
   expanded+=entry.file_size; assert expanded <= 500000000
   (dirs if entry.is_dir() else files).add(name)
   for node in [p,*p.parents]:
    spelling=node.as_posix()
    if spelling=='.':continue
    key=unicodedata.normalize('NFC',spelling).casefold()
    assert key not in aliases or aliases[key]==spelling
    aliases[key]=spelling
   dirs.update(parent.as_posix() for parent in p.parents if parent.as_posix()!='.')
  assert not files&dirs and len(files)+len(dirs) <= 20000
  destination.mkdir(mode=0o700)
  for name in sorted(dirs,key=lambda value:(value.count('/'),value)):
   (destination/name).mkdir(exist_ok=True)
  for entry in entries:
   if not entry.is_dir():
    payload=z.read(entry);assert len(payload)==entry.file_size
    with (destination/entry.filename).open('xb') as f:f.write(payload)
 evidence[role]={'root':role,'archive':role+'.zip'}
create('index.json',(json.dumps({'schema':'quod-property47-packaged-action-audit-inputs-v1',
 'runJson':'run.json','jobsJson':'jobs.json','artifactsJson':'artifacts.json','evidence':evidence},indent=2)+'\n').encode())
PY
python3 -I -S "$property47_audit_dir/reader.py" \
  --runtime-repository "$property47_audit_dir/quod" \
  --action-repository "$property47_audit_dir/action" \
  --workflow-repository "$property47_audit_dir/quod" \
  --project-repository "$property47_audit_dir/project" \
  "$property47_audit_dir/index.json" > "$property47_audit_dir/report.json"
```

The reader rechecks the original ZIPs **and** extracted trees; supplying only
extracted JSON is insufficient. Outer profiles are 128MiB compressed,
500,000,000 expanded bytes, 64MiB per file and 20,000 files plus implicit
directories. Nested trusted-B tar caps are 64MiB compressed, 256MiB expanded,
160,000,000 bytes per regular member and 1,000 headers. The exact B tar bytes
must match before its contained hardlink/symlink graphs are parsed. All runtime
and project contents remain data. The index never selects code, Git refs,
bounds, or authority pins; no private `/tmp` helper is required.

## Reader correction and publication normalization

The first independent reader attempts failed on a strict old-v0 report-path
suffix: they expected `/packaged-proof-v0/original-native-report.json` instead
of W's actual `/property47-packaged-proof-v0/original-native-report.json`.
The strict correction changed that one literal. W, B, Q, the completed gate,
assertions and artifact bytes were unchanged. The 133-byte failed reports remain
SHA256 `d59b7e54f358bc0b9439af0bf2cbfe1dac0f9d207b20c927f23e4b63a097edbe`.
The corrected accepted reader is 111,344 bytes, SHA256
`e0b242d8e4f4081183b70ec1afad0700b7932d212a938f5b5147c8835fe22c10`.

The [published standalone reader](../../scripts/bench-vitest/managed/property47-packaged-action-audit.py)
changes only its initial documentation and two emitted output-schema labels
from that accepted source. Restoring those strings/docstring restores its
entire executable AST exactly. Public reader SHA256 is
`a97846343f7bb515207935198050072cf28249b0ef22059063a6c4d4d0fbbad9`.
The complete public report changes only the root schema from the independently
accepted report; all checks, raw provenance and measured data are preserved.
Its 359,228 bytes have SHA256
`1dbc0c59f175da554ec427e29b9c3edd7a7c147dcb98176b51bd268c5fbc70bf`.

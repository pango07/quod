#!/usr/bin/env python3
"""Pinned packaged property47 Action evidence audit for run42433432522.

Reads retained GitHub metadata, complete ZIP/extracted inventories, trusted
Git source objects, SDK acknowledgments/packs and raw runtime/native evidence.
Only Python standard-library modules are imported. Artifact source and binaries
are never imported, evaluated, compiled or executed.

Usage:
  python3 -I -S READER --runtime-repository TRUSTED_Q_CHECKOUT \
    --action-repository TRUSTED_B_CHECKOUT --workflow-repository TRUSTED_W_CHECKOUT \
    --project-repository TRUSTED_C11_CHECKOUT INDEX
Index schema: quod-property47-packaged-action-audit-inputs-v1. The index supplies
raw run/jobs/artifacts API JSON, four exact ZIP+extracted-root pairs, and the
pinned ff baseline ZIP+root. It cannot select Git refs, source, bounds or pins.
The run, source, job, artifact and bound constants below are fixed authority.
"""
import argparse
import collections
import datetime
import hashlib
import io
import json
import math
import os
import pathlib
import posixpath
import re
import stat
import subprocess
import sys
import tarfile
import unicodedata
import zipfile
import base64
import zlib

ROLES = ('seed', 'control', 'consumer', 'v0')
RUN = 42433432522
WORKFLOW_SOURCE = '2893a9c0d222b22f22ff2d4d5a5431618893d7ed'
ACTION_B = '0520117d99379472cd33dbb777f852d8a189a20a'
RUNTIME_Q = 'f67fe14033de76f0b26b436cef4e5d50b0af1ee3'
ARTIFACTS = {'seed': (13598940155, 51795163, 'df880336795d43d447878a444784a50abe3956ab33df532e494af8ce24777135', 128587100575, 'packaged-property47-seed'), 'control': (13597958625, 51686608, '134a7c5e04d5ae4874b8a9194a3f5df17a39026a014af107e6bf530986920910', 128587100262, 'packaged-property47-control'), 'consumer': (13598364334, 51467896, '2f1b3421ee38ffcba8e9d769d88ae1445ed3cfacebd4c061e0919569ee755136', 128588344549, 'packaged-property47-consumer'), 'v0': (13597989907, 51612465, 'f48a0c9dfbe6a8699e5f56095a549b776db305ce2ad926a19ffced7270974022', 128587100526, 'packaged-property47-v0')}
JOB_IDS = {'seed': 128587100575, 'control': 128587100262, 'consumer': 128588344549, 'v0': 128587100526}
JOB_TIMESTAMPS = {'seed': ('2026-11-08T01:15:11Z', '2026-11-08T01:18:16Z'), 'control': ('2026-11-08T01:15:11Z', '2026-11-08T01:20:45Z'), 'consumer': ('2026-11-08T01:20:47Z', '2026-11-08T01:21:42Z'), 'v0': ('2026-11-08T01:15:11Z', '2026-11-08T01:18:59Z')}
STANDARD_HELPER = (128587100616, 'changes', '2026-11-08T01:15:11Z', '2026-11-08T01:15:16Z')
# Bounds are reviewed authority too. The 51,011,909B historical Action runtime
# tar already exceeds the accepted SDK reader's 32MiB per-file bound. Do not
# silently widen that SDK profile or copy its 100MB/500MB limits to these jobs.
ARCHIVE_PROFILE = {'compressedBytes': 134217728, 'expandedBytes': 500000000, 'fileBytes': 67108864, 'paths': 20000}
RUNTIME_TAR_PROFILE = {'compressedBytes': 67108864, 'expandedBytes': 268435456, 'fileBytes': 160000000, 'paths': 1000}
REVIEWED_RUNTIME_TAR_PROFILE = {'compressedBytes': 64 * 1024 * 1024, 'expandedBytes': 256 * 1024 * 1024,
    'fileBytes': 160000000, 'paths': 1000}
IMPLEMENTATION_COMPLETE = True
REVIEW_APPROVED = True

ACCEPTED_PACKAGED108_READER_SHA256 = 'b387df9a4f8ae37d8ae7f7cd712f7ef3fcc6f34dec410dcd067112e93ddb8ded'
SDK_PRIMITIVES_SOURCE_SHA256 = '890af7465cc9c107a31ba08542661e08a3ff3272044858de75b213623e7ba1ad'
REVIEWED_SDK_READER_SHA256 = 'c0da19e6da607efd70b3b61335d727650cd166689a7d7b0106b1b08d0afbf5ff'
PROPERTY_PROFILE = 'reviewed-original-property47-single-instance-v1-research'
PROPERTY_SELECTOR_SHA256 = '840ef60f36368ace3edb40a23d6984d0366c6a9a89a3f4d8d5d34c7a46bc3fce'
PROPERTY_TARGET = {'instanceId': 'ccda7135f8d1ede102763427b3efabcef8130aaed24822ce92a5ec7ef94a05b3',
    'project': 'convex-helpers', 'file': 'packages/convex-helpers/server/stream.property.test.ts', 'pool': 'forks'}
PROPERTY_DESCRIPTOR = {'schema': 'quod-managed-matrix-candidate-profile-v1-research', 'label': PROPERTY_PROFILE,
    'roster': [PROPERTY_TARGET], 'rosterSha256': hashlib.sha256(json.dumps([[PROPERTY_TARGET[k] for k in
        ['instanceId', 'project', 'file', 'pool']]], separators=(',', ':')).encode()).hexdigest()}
BATCH_POLICY = 'closed-adjacent-full-fences-before-any-candidate-worker-v1-diagnostic'
COMPILER = 'a87f1f8fafea2e0f1b4a1e299199f5581172a2fe90f59882d3c64eb50f68acaf'
BATCH_CORE = 'f01cd9a55f6eceb913c5b402481fd6b6dd7266191d8182f97ca60f158765c703'
TOOL_FILES = '7b7ae881ef3186e4b564c26ffa6e8d40b3d3875a15e41b10d93a6bb24cb3a354'
BASELINE_SOURCE = 'ff279ea5b59b359d26985f08f22e0749f9eccbde'
BASELINE_RUN = 42406036816
BASELINE = (13587034973, 36186785,
    'f8e20f3a68cc3fbec7ce01a5ea90e8b50a1d558cbf08146a7fb049e242adc282',
    128503824968, 'benchmark-managed-matrix')
BASELINE_SELECTION_BYTES = 1903888
BASELINE_SELECTION_SHA256 = '1d3ceb0a7520632c23474769721d608875eb4b9cfed6c6d248f61b89c946ced1'
PROJECT_C11 = '5eaa7e5160736054d53f8b71e2c522835eb427e0'
NATIVE_C11 = '09ebb9e3a722220d3a21ddfe9f63992a8a074b0fcaa95764e54c37be90d469cd'
MAX_ITEM = 2 * 1024 * 1024
MAX_PACK = 32 * 1024 * 1024
# Not operational until checked_pins validates the independent profile.
MAX_ARCHIVE = MAX_EXPANDED = MAX_PATHS = None
MAX_FILE = 32 * 1024 * 1024  # copied raw() default; larger reads must be explicit
RUNTIME_REPOSITORY = ACTION_REPOSITORY = WORKFLOW_REPOSITORY = PROJECT_REPOSITORY = None
RUNTIME_MEMBERS = None
PREPARATION_SOURCE_HASHES = PROJECT_DATA_SOURCE_HASHES = None
RELEASE = sorted(['cancelled 0', 'fail 0', 'pass 20', 'skipped 0', 'tests 20', 'todo 0'])
ARTIFACT_NAMES = {role: 'managed-property47-packaged-' + role for role in ROLES}
JOB_NAMES = {role: 'packaged-property47-' + role for role in ROLES}
NCC_BUNDLES = ('main', 'post', 'managed-bridge', 'matrix-bridge')
ACTION_SOURCES = ('src/main.js', 'src/post.js', 'src/matrix-bridge.js', 'src/managed-collection.js',
    'src/managed-auth.js', 'src/matrix-pack-evidence.js', 'src/matrix-reports.js',
    'src/managed-bridge.js', 'src/managed-cache.js',
    'hook.cjs', 'deps.cjs', 'npm-install-profile.cjs', 'managed-source.json', 'package.json')

# Primitives below are copied as source, never imported from a mutable helper.
# Their origin is the independently reviewed SDK draft named by
# SDK_PRIMITIVES_SOURCE_SHA256. The native/logical/envelope/certificate/worker/
# batch algorithms are retained below with explicit new B/project bindings.
# No old SDK controller-output wrapper is fabricated from Action evidence.


def sha(value):
    return hashlib.sha256(value).hexdigest()



def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=False, allow_nan=False)



def unique(pairs):
    value = {}
    for key, entry in pairs:
        assert key not in value, 'duplicate JSON key: ' + key
        value[key] = entry
    return value



def decode(value):
    return json.loads(value, object_pairs_hook=unique,
                      parse_constant=lambda token: (_ for _ in ()).throw(ValueError(token)))



def raw(file, maximum=MAX_FILE):
    file = pathlib.Path(file)
    before = file.lstat()
    assert stat.S_ISREG(before.st_mode) and 0 <= before.st_size <= maximum, 'bounded regular data file required: ' + str(file)
    value = file.read_bytes()
    after = file.lstat()
    assert len(value) == before.st_size and (before.st_ino, before.st_dev, before.st_size, before.st_mtime_ns,
        before.st_ctime_ns) == (after.st_ino, after.st_dev, after.st_size, after.st_mtime_ns, after.st_ctime_ns), 'sampled data file changed'
    return value



def read(file, maximum=MAX_FILE):
    return decode(raw(file, maximum))



def domain(name, value):
    return sha((name + '\n' + canonical(value)).encode())



def exact(value, names):
    assert isinstance(value, dict) and set(value) == set(names), 'JSON fields differ'



def digest(value):
    return isinstance(value, str) and re.fullmatch('[0-9a-f]{64}', value)



def b64(value, maximum):
    assert isinstance(value, str) and len(value) <= ((maximum + 2) // 3) * 4
    body = base64.b64decode(value, validate=True)
    assert 0 < len(body) <= maximum and base64.b64encode(body).decode() == value
    return body



def archive(role, descriptor, expected):
    root, file = pathlib.Path(descriptor['root']), pathlib.Path(descriptor['archive'])
    assert stat.S_ISDIR(root.lstat().st_mode) and stat.S_ISREG(file.lstat().st_mode)
    assert file.stat().st_size == expected[1] <= MAX_ARCHIVE
    h = hashlib.sha256()
    with file.open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            h.update(block)
    assert h.hexdigest() == expected[2], role + ' whole archive byte authority differs'
    names, normalized_names, files, directories, expanded = set(), {}, {}, set(), 0
    with zipfile.ZipFile(file) as zipped:
        assert 0 < len(zipped.infolist()) <= MAX_PATHS
        for entry in zipped.infolist():
            path = pathlib.PurePosixPath(entry.filename)
            name = path.as_posix()
            assert name == entry.filename.rstrip('/') and name and name != '.'
            assert not path.is_absolute() and not any(part in {'', '.', '..'} for part in path.parts)
            assert not any(char in entry.filename for char in ('\\', '\0', '\r', '\n', ':'))
            assert len(entry.filename.encode('utf-8')) <= 4096 and len(path.parts) <= 128, 'ZIP path bound exceeded'
            assert name not in names, 'duplicate ZIP name'
            names.add(name)
            # Include implicit parent directories: Foo/a and foo/b collide on
            # common extraction filesystems even without explicit ZIP dirs.
            for node in [path, *path.parents]:
                spelling = node.as_posix()
                if spelling == '.':
                    continue
                key = unicodedata.normalize('NFC', spelling).casefold()
                assert key not in normalized_names or normalized_names[key] == spelling, 'Unicode/case-colliding ZIP path'
                normalized_names[key] = spelling
            mode = entry.external_attr >> 16
            kind = stat.S_IFMT(mode)
            assert not entry.flag_bits & 1 and entry.compress_type in (zipfile.ZIP_STORED, zipfile.ZIP_DEFLATED)
            assert kind in ((0, stat.S_IFDIR) if entry.is_dir() else (0, stat.S_IFREG)), 'special ZIP entry'
            assert 0 <= entry.file_size <= MAX_FILE
            expanded += entry.file_size
            assert expanded <= MAX_EXPANDED
            for parent in path.parents:
                if parent.as_posix() != '.':
                    directories.add(parent.as_posix())
            if entry.is_dir():
                assert entry.file_size == 0
                directories.add(name)
                continue
            target = root.joinpath(*path.parts)
            info = target.lstat()
            assert stat.S_ISREG(info.st_mode) and info.st_size == entry.file_size
            archived, extracted = hashlib.sha256(), hashlib.sha256()
            size = 0
            with zipped.open(entry) as a, target.open('rb') as b:
                while True:
                    aa, bb = a.read(1024 * 1024), b.read(1024 * 1024)
                    assert aa == bb, role + ' extracted bytes differ: ' + name
                    if not aa:
                        break
                    size += len(aa); archived.update(aa); extracted.update(bb)
            assert size == entry.file_size and archived.digest() == extracted.digest()
            files[name] = {'bytes': size, 'sha256': archived.hexdigest()}
    actual_files, actual_directories = set(), set()
    for directory, dirs, leaves in os.walk(root, followlinks=False):
        assert stat.S_ISDIR(pathlib.Path(directory).lstat().st_mode)
        for name in dirs + leaves:
            target = pathlib.Path(directory) / name
            info = target.lstat(); relative = target.relative_to(root).as_posix()
            assert stat.S_ISDIR(info.st_mode) or stat.S_ISREG(info.st_mode), 'special extracted entry'
            (actual_directories if stat.S_ISDIR(info.st_mode) else actual_files).add(relative)
            assert len(actual_directories) + len(actual_files) <= MAX_PATHS, 'extracted path bound exceeded'
    assert actual_files == set(files) and actual_directories == directories, role + ' exact extracted inventory differs'
    assert not set(files) & directories
    return root, {'artifactId': expected[0], 'archiveSizeBytes': expected[1], 'archiveSha256': expected[2],
        'filesByteVerified': len(files), 'directoriesVerified': len(directories), 'expandedBytes': expanded, 'files': files}



def locate(root, suffix):
    matches = [p for p in root.rglob(suffix[-1]) if p.relative_to(root).parts[-len(suffix):] == tuple(suffix)]
    assert len(matches) == 1, 'one bounded data path required: ' + '/'.join(suffix)
    return matches[0]




# Reviewed SDK admission logic; narrow project-source and shipped-binary
# bindings are added explicitly below. Packaged wrapper facts are read directly.
CERTIFICATE_LIMITS = {'paths': 100000, 'resolverBytes': 16 * 1024 * 1024, 'resolverTimeoutMs': 30000,
    'toolBytes': 128 * 1024 * 1024, 'sourceBytes': 256 * 1024 * 1024,
    'fileBytes': 16 * 1024 * 1024, 'certificateBytes': 16 * 1024 * 1024,
    'certificateSetBytes': 128 * 1024 * 1024, 'certificates': 512}
CONTRACT = 'quod-managed-v0-protectors-frozen-slots-investigation'
INTRINSICS = 'quod-managed-frozen-array-promise-slots-v1-probe'
EXECUTION = 'quod-managed-matrix-execution-v2-research'
CONFORMANCE = 'quod-managed-matrix-conformance-v2-research'
RECEIPT = 'quod-managed-matrix-receipt-v2-research'
ENVELOPE = 'quod-managed-matrix-input-candidate-v1-research'
CERTIFICATE = 'quod-managed-resolved-compile-certificate-v1-diagnostic'
PASSING = {'pass', 'skip', 'todo'}
STATUS = {'passed': 'pass', 'failed': 'fail', 'skipped': 'skip', 'todo': 'todo'}
PREFLIGHT_DENIALS = ['global fetch', 'global crypto', 'global WeakRef', 'global FinalizationRegistry',
    'Event.timeStamp', 'global WebAssembly', 'AbortSignal.timeout', 'eval', 'Function constructor',
    'AsyncFunction constructor', 'GeneratorFunction constructor', 'AsyncGeneratorFunction constructor',
    'global EventTarget.addEventListener', 'global EventTarget.removeEventListener', 'global EventTarget.dispatchEvent']

def state_rows(modules):
    return collections.Counter((m['project'], m['file'], m['pool'], t['name'], t['state']) for m in modules for t in m['rows'])


def file_rows(modules):
    return collections.Counter((m['instanceId'], m['project'], m['file'], m['pool'], m['relativeModuleId'], m['state']) for m in modules)


def instance_map(rows):
    mapped = {row['instanceId']: row for row in rows}
    assert len(mapped) == len(rows), 'duplicate actual project/file/pool instance'
    return mapped


def native(root, baseline=False, passing=True):
    selected = read(root / 'native-selection.json')
    report_bytes = raw(root / 'native-report.json')
    report = decode(report_bytes)
    assert selected['schema'] == 'quod-managed-matrix-native-v1'
    assert selected['matched'] is True and selected['completed'] is True and selected['runStarted'] is True
    assert selected['reportSha256'] == sha(report_bytes)
    assert len(selected['selected']) == len(selected['executed']) == len(report['testResults'])
    assert set(instance_map(selected['selected'])) == set(instance_map(selected['executed']))
    if baseline:
        assert 'originalSelected' not in selected
        selected['originalSelected'] = selected['selected']
    original = instance_map(selected['originalSelected'])
    assert set(instance_map(selected['selected'])) <= set(original)
    assert set(instance_map(selected.get('omittedAsHits', []))) == set(original) - set(instance_map(selected['selected']))
    for row in selected['originalSelected']:
        identity = {key: row[key] for key in ['file', 'project', 'pool', 'testLines', 'testIds', 'testTagsFilter', 'testNamePattern']}
        assert all(identity[key] is None for key in ['testLines', 'testIds', 'testTagsFilter', 'testNamePattern'])
        assert row['pool'] == 'forks' and sha(canonical(identity).encode()) == row['instanceId']
    api = collections.Counter((m['absolute'], t['name'], t['state']) for m in selected['executed'] for t in m['rows'])
    reporter = collections.Counter((m['name'], t['fullName'], STATUS[t['status']]) for m in report['testResults'] for t in m['assertionResults'])
    assert api == reporter, 'actual native per-project API rows differ from raw reporter multiset'
    if passing:
        assert selected['exitCode'] == 0 and selected['failedModules'] == 0 and selected['unhandledErrors'] == [] and report['success'] is True
        for module in selected['executed']:
            assert module['ok'] is True and module['state'] in PASSING and module['errors'] == []
            assert module['providedContext'] == ({'expectedZodAlias': module['project']} if module['project'].startswith('zod-') else {})
            assert all(test['state'] in PASSING for test in module['rows'])
            assert all(suite['state'] in {'passed', 'skipped'} and suite['errors'] == [] for suite in module['suites'])
    return selected


def boundary(data, preflight=False, closed_error=False):
    if closed_error:
        exact(data, ['ok', 'error', 'boundary'])
        assert data['ok'] is False
        exact(data['error'], ['name', 'message', 'stack'])
        assert all(isinstance(data['error'][key], str) for key in ['name', 'message', 'stack'])
        assert 'QUOD_MATRIX_OUTER_CLEANUP_FAILURE' in data['error']['message']
    else:
        assert data['ok'] is True
    if not preflight and not closed_error:
        exact(data, ['ok', 'rows', 'files', 'boundary'])
        assert len(data['files']) == 1 and data['files'][0]['errors'] == []
        assert data['files'][0]['state'] in PASSING
        for row in data['rows']:
            exact(row, ['name', 'state', 'errors'])
            assert isinstance(row['name'], str) and row['name'] and row['state'] in PASSING
            assert isinstance(row['errors'], list) and (row['state'] == 'pass' or not row['errors'])
    b = data['boundary']; c = b['completion']
    assert b['contract'] == CONTRACT and b['intrinsicPolicy'] == INTRINSICS and b['replayEligible'] is False
    assert c['sealed'] is True and c['intrinsicLockdown'] is True and c['reason'] == ''
    assert 2 <= c['nativeTurnFences'] <= 512 and c['protectedIntrinsicObjects'] > 0
    assert b['pendingTimers'] == 0 and b['pendingTimerDetails'] == []
    assert all(c[key] == 0 for key in ['pendingTimerCallbacks', 'pendingMicrotasks', 'pendingEventCallbacks'])
    assert b['unsupported'] == (PREFLIGHT_DENIALS if preflight else [])
    assert all(b[key] == [] for key in ['unguardable', 'symbolUnguardable', 'timerErrors', 'unhandledErrors'])
    for symbol in b['symbolInventory']:
        assert symbol['policy'] in {'fixed primitive data', 'guarded'}


def logical(root, dispatch, expected_states=None, expected_files=None):
    body = raw(root / 'logical-results.json'); value = decode(body); claim = dispatch['logicalReport']
    assert claim['sha256'] == sha(body) and claim['bytes'] == len(body)
    assert pathlib.PurePosixPath(claim['file']).name == 'logical-results.json'
    assert claim['schema'] == value['schema'] == 'quod-managed-matrix-input-logical-results-v1'
    assert claim['instances'] == len(value['files']) == len(dispatch['instances']) and claim['states'] == len(value['rows'])
    assert value['instances'] == dispatch['instances'] and value['exitCode'] == dispatch['exitCode']
    assert value['stickyManagedFailure'] == dispatch['stickyManagedFailure'] and value['observedNativeFailure'] == dispatch['observedNativeFailure']
    actual = collections.Counter((row['project'], row['file'], row['pool'], row['name'], row['state']) for row in value['rows'])
    files = collections.Counter((row['instanceId'], row['project'], row['file'], row['pool'], row['name'], row['state']) for row in value['files'])
    assert len(instance_map(value['files'])) == len(value['files'])
    sorted_rows = sorted([list(row) for row in actual.elements()], key=canonical)
    assert sha(canonical(sorted_rows).encode()) == dispatch['logicalSha256'] == value['sha256']
    if expected_states is not None:
        assert actual == expected_states
    if expected_files is not None:
        assert files == expected_files, 'strong logical project/file/pool/API-file-state multiset differs'
    return value, actual, files


def unpack_envelope(body, key, trusted):
    assert 0 < len(body) <= MAX_ITEM and sha(body) == trusted
    envelope = decode(body)
    assert (canonical(envelope) + '\n').encode() == body
    exact(envelope, ['schema', 'candidateKey', 'certificate', 'receipt'])
    assert envelope['schema'] == ENVELOPE and envelope['candidateKey'] == key and digest(key)
    header, compressed = envelope['certificate'], envelope['receipt']
    exact(header, ['fields', 'sha256']); exact(compressed, ['encoding', 'base64', 'bytes', 'sha256'])
    assert compressed['encoding'] == 'gzip-base64' and digest(header['sha256']) and digest(compressed['sha256'])
    assert type(compressed['bytes']) is int and 0 < compressed['bytes'] <= MAX_ITEM
    encoded = b64(compressed['base64'], MAX_ITEM)
    # Node's reviewed gzipSync(level6) producer has one zero-mtime, no-extra
    # member. Reject concatenated/trailing streams; never allocate from a size
    # claim or decompress without an independent output cap.
    assert encoded[:10] == bytes([31, 139, 8, 0, 0, 0, 0, 0, 0, 3])
    stream = zlib.decompressobj(16 + zlib.MAX_WBITS)
    receipt_bytes = stream.decompress(encoded, MAX_ITEM + 1)
    assert 0 < len(receipt_bytes) <= MAX_ITEM and stream.eof and not stream.unused_data and not stream.unconsumed_tail
    assert len(receipt_bytes) == compressed['bytes'] and sha(receipt_bytes) == compressed['sha256']
    receipt = decode(receipt_bytes)
    assert canonical(receipt).encode() == receipt_bytes
    fields = header['fields']; assert isinstance(fields, dict) and 'compile' not in fields
    certificate = {**fields, 'compile': receipt['execution']['identity']['compile']}
    certificate_bytes = (canonical(certificate) + '\n').encode()
    assert 0 < len(certificate_bytes) <= MAX_ITEM and sha(certificate_bytes) == header['sha256']
    return envelope, certificate, certificate_bytes, receipt, receipt_bytes


def certificate(value, identity, sources):
    exact(value, ['schema', 'limits', 'selection', 'compile', 'rootGeometry', 'tools', 'sourceFiles'])
    assert value['schema'] == CERTIFICATE and value['limits'] == CERTIFICATE_LIMITS and value['compile'] == identity['compile']
    compile = value['compile']; tools = value['tools']; selected = value['selection']
    assert selected == {key: compile[key] for key in ['entry', 'testEntry', 'configurationSha256']}
    assert compile['compilerSha256'] == COMPILER == sources['compiler.mjs']
    assert compile['esbuildVersion'] == '0.28.2' and compile['typescriptVersion'] == '6.0.3'
    assert tools['platform'] == 'linux' and tools['arch'] == 'x64' and tools['node'] == 'v24.18.0'
    assert tools['nodeSha256'] == identity['nativePolicy']['nodeExecutableSha256']
    assert sha(canonical(tools['files']).encode()) == TOOL_FILES and len(tools['files']) == 183
    assert sum(row.get('bytes', 0) for row in tools['files']) == 47351803
    assert tools['counts'] == {'paths': 183, 'bytes': 47351803}
    assert tools['hostSources'] == [{'name': 'compiler.mjs', 'sha256': COMPILER},
        {'name': 'resolved-compile-certificate.mjs', 'sha256': sources['resolved-compile-certificate.mjs']}]
    assert tools['adapterPackageJsonSha256'] == sources['package.json']
    assert tools['packageRoots'] == ['/quod-adapter/node_modules/esbuild', '/quod-adapter/node_modules/@esbuild/linux-x64',
        '/project/node_modules/typescript', '/project/node_modules/@typescript/old']
    exact(value['rootGeometry'], ['project', 'adapter'])
    assert all(isinstance(path, str) and path.startswith('/') for path in value['rootGeometry'].values())
    coverage = compile['inputCoverage']; assert coverage['schema'] == 'quod-managed-physical-input-coverage-v1'
    assert coverage['physicalSources'] == sorted(row['source'] for row in compile['sources'])
    assert coverage['virtualInputs'] == sorted(set(coverage['virtualInputs']))
    assert set(coverage['virtualInputs']) <= {'<define:__QUOD_CONFIG__>', 'quod-managed-als:public-async-local-storage'}
    names = sorted(set(row['source'] for row in compile['sources']))
    assert [row['name'] for row in value['sourceFiles']] == names
    assert all(digest(row['rawSha256']) and type(row['bytes']) is int and 0 <= row['bytes'] <= 16 * 1024 * 1024 for row in value['sourceFiles'])
    assert len(names) <= 100000 and sum(row['bytes'] for row in value['sourceFiles']) <= 256 * 1024 * 1024
    assert compile['resolutionFacts'] and all(row['phase'] in {'original', 'emitted'} for row in compile['resolutionFacts'])
    assert all(row['route'] in {'ordinary', 'test-entry', 'native-als', 'public-als', 'als-implementation', 'injected-helper'} for row in compile['resolutionFacts'])
    return value


def worker_evidence(directory, expected_compile, expected_policy, expected_host, phase):
    invocation = read(directory / 'invocation.json')
    execution = read(directory / 'execution.json')
    response = raw(directory / 'response.json')
    assert invocation['schema'] == 'quod-managed-invocation-v1-research'
    assert invocation['name'] == directory.name and invocation['bundleSha256'] == expected_compile['bundleSha256']
    assert sha(raw(directory / 'worker.mjs')) == expected_compile['bundleSha256']
    assert invocation['configSha256'] == sha(raw(directory / 'config.capnp'))
    assert sha(raw(directory / 'deny.mjs')) == expected_policy['denyModuleSha256']
    # The ephemeral TCP port and absolute retained path are invocation evidence,
    # while all admitted behavioral policy and binary identities must agree.
    policy = {key: value for key, value in expected_policy.items() if key != 'nativeFingerprintSha256'}
    assert invocation['runtimePolicy'] == policy and invocation['host'] == expected_host
    assert invocation['env'] == policy['processEnv'] and invocation['args'][0] == 'serve' and len(invocation['args']) == 2
    assert pathlib.PurePosixPath(invocation['args'][1]).name == 'config.capnp'
    assert invocation['request']['method'] == 'GET' and re.fullmatch(r'http://127\.0\.0\.1:[0-9]+/run', invocation['request']['url'])
    port = invocation['request']['url'].split(':')[-1].split('/')[0]
    config = ('using Workerd = import "/workerd/workerd.capnp";\n'
        'const config :Workerd.Config = (\n'
        '  services = [\n'
        '    (name = "main", worker = (modules = [(name = "worker.mjs", esModule = embed "worker.mjs")],\n'
        '      compatibilityDate = "2026-07-01", compatibilityFlags = ["nodejs_als"], globalOutbound = "deny")),\n'
        '    (name = "deny", worker = (modules = [(name = "deny.mjs", esModule = embed "deny.mjs")],\n'
        '      compatibilityDate = "2026-07-01", globalOutbound = "deny"))\n'
        '  ],\n'
        '  sockets = [(name = "http", address = "127.0.0.1:' + port + '", http = (), service = "main")]\n'
        ');\n').encode()
    assert raw(directory / 'config.capnp') == config, 'actual worker config semantics differ'
    assert phase['name'] == directory.name and phase['responseSha256'] == sha(response)
    # execution.json is emitted by the trusted executor, never used as receipt
    # authority. Keep its native success and exact byte observations linked.
    assert execution['responseSha256'] == sha(response) and execution['name'] == directory.name
    assert execution['schema'] == 'quod-managed-execution-v1-research' and execution['completed'] is True
    assert execution['timedOut'] is False and execution['error'] is None
    assert execution['exit'] == {'code': 0, 'signal': None, 'spawnError': None}
    meta = read(directory / 'response-meta.json')
    assert meta['status'] == 200 and meta['bytes'] == len(response) and meta['sha256'] == sha(response)
    return response


def resolution_requests(certificates):
    requests = {}
    for value in certificates:
        roots = value['rootGeometry']
        def physical(name):
            for prefix, root in [('/project/', roots['project']), ('/quod-adapter/', roots['adapter'])]:
                if name.startswith(prefix):
                    suffix = name[len(prefix):]
                    assert suffix and not any(part in {'', '.', '..'} for part in suffix.split('/'))
                    return root + '/' + suffix
            raise AssertionError('external resolver path')
        for fact in value['compile']['resolutionFacts']:
            if fact['route'] != 'ordinary':
                continue
            importer = physical(fact['source']) if fact['source'] else ''
            target = physical(fact['target'])
            request = {'path': target if fact['kind'] == 'entry-point' else fact['specifier'], 'importer': importer,
                'resolveDir': importer.rsplit('/', 1)[0] if importer else roots['project'], 'kind': fact['kind'], 'zodAlias': fact['zodAlias']}
            key = canonical(request)
            assert key not in requests or requests[key][1] == target
            requests[key] = (request, target)
    return [row[0] for row in requests.values()]


def batch_trace(count):
    rows = ['before-constructor', 'after-constructor/before-compile:0']
    for index in range(count):
        rows += ['after-compile:' + str(index) + '/before-capture:' + str(index),
            'after-capture:' + str(index) + ('/before-publication' if index == count - 1 else '/before-compile:' + str(index + 1))]
    return rows


def batch_evidence(role, root, dispatch, sources):
    assert dispatch['compilationPolicy'] == BATCH_POLICY and 'admissionDeclined' not in dispatch
    assert dispatch['supervisorPolicy']['compilationPolicy'] == BATCH_POLICY
    batch = dispatch['compilationBatch']; file = root / 'batch-compilation.json'
    assert type(dispatch['seconds']) in (int, float) and math.isfinite(dispatch['seconds']) and dispatch['seconds'] >= 0
    if role == 'consumer':
        assert batch == {'attempted': False, 'requests': 0, 'completed': False, 'seconds': 0}
        assert not file.exists() and not file.is_symlink()
        assert [phase['name'] for phase in dispatch['phases']] == ['preflight']
        assert not any(child.name.startswith('managed-') for child in root.iterdir())
        assert sorted(file.relative_to(root).as_posix() for file in root.rglob('worker.mjs')) == ['preflight/worker.mjs']
        assert sorted(file.relative_to(root).as_posix() for file in root.rglob('invocation.json')) == ['preflight/invocation.json']
        assert dispatch['managedExecutions'] == 0 and dispatch['skippedCompilations'] == dispatch['replayedInstances'] == dispatch['inputCandidates'] == 1
        for row in dispatch['instances']:
            assert not row.get('workerEvidenceDirectory')
            if row['candidate']:
                assert row['evidenceDirectory'] == 'instances/' + row['instanceId']
                assert row['compilationSkipped'] is True and row['compilationSeconds'] == 0 and not row.get('workerEvidenceDirectory')
                directory = root / row['evidenceDirectory']
                assert not (directory / 'test-build.json').exists() and not (directory / 'compile-certificate.json').exists()
        return {'requests': 0, 'fullFences': 0, 'seconds': 0}
    assert role in {'seed', 'control'}
    exact(batch, ['attempted', 'requests', 'completed', 'seconds', 'diagnostics'])
    assert read(file) == batch and batch['attempted'] is True and batch['completed'] is True and batch['requests'] == 1
    assert type(batch['seconds']) in (int, float) and math.isfinite(batch['seconds']) and 0 <= batch['seconds'] <= dispatch['seconds'] + 0.01
    diagnostics = batch['diagnostics']
    exact(diagnostics, ['requests', 'fullFences', 'trace', 'totalBundles', 'totalCertificates', 'totalIdentities', 'publicOperationsAtReservation', 'authority'])
    assert diagnostics['requests'] == 1 and diagnostics['fullFences'] == 4 and diagnostics['trace'] == batch_trace(1)
    assert diagnostics['publicOperationsAtReservation'] == 0
    assert diagnostics['authority'] == 'actual compilation and capture only; no response or receipt admission'
    totals = {'totalBundles': 0, 'totalCertificates': 0, 'totalIdentities': 0}
    candidates = [row for row in dispatch['instances'] if row['candidate']]
    assert len(candidates) == 1 and len({row['instanceId'] for row in candidates}) == 1
    for row in candidates:
        assert row['evidenceDirectory'] == 'instances/' + row['instanceId'] and row['decision'] == 'managed'
        directory = root / row['evidenceDirectory']; built = read(directory / 'test-build.json')
        bundle = raw(root / row['workerEvidenceDirectory'] / 'worker.mjs')
        certificate_bytes = raw(directory / 'compile-certificate.json', MAX_ITEM)
        value = decode(certificate_bytes)
        assert sha(bundle) == built['sha256'] == row['bundleSha256'] == built['identity']['bundleSha256']
        assert sha(canonical(built['identity']).encode()) == built['identitySha256']
        assert row['compilationSeconds'] == built['seconds'] and type(built['seconds']) in (int, float)
        assert math.isfinite(built['seconds']) and 0 <= built['seconds'] <= batch['seconds'] + 0.01
        assert not row.get('compilationSkipped', False) and built['closure']['finalBundleParsed'] is True
        assert certificate_bytes == (canonical(value) + '\n').encode() and value['compile'] == built['identity']
        assert value['tools']['hostSources'] == [{'name': 'compiler.mjs', 'sha256': COMPILER},
            {'name': 'resolved-compile-certificate.mjs', 'sha256': sources['resolved-compile-certificate.mjs']}]
        totals['totalBundles'] += len(bundle); totals['totalCertificates'] += len(certificate_bytes)
        totals['totalIdentities'] += len(canonical(built['identity']).encode())
    assert all(diagnostics[key] == value for key, value in totals.items())
    assert totals['totalBundles'] <= CERTIFICATE_LIMITS['sourceBytes']
    assert totals['totalCertificates'] <= CERTIFICATE_LIMITS['certificateSetBytes'] and totals['totalIdentities'] <= CERTIFICATE_LIMITS['certificateSetBytes']
    return {'requests': 1, 'fullFences': 4, 'seconds': batch['seconds'], **totals}



def check_execution(identity, row, sources):
    assert identity['schema'] == EXECUTION and identity['instance'] == {key: row[key] for key in ['instanceId', 'project', 'file', 'pool']}
    assert identity['compile']['compilerSha256'] == COMPILER and identity['preparation']['preparerSha256'] == sources['preparation.mjs']
    assert {row['name']: row['sha256'] for row in identity['preparation']['sources']} == PREPARATION_SOURCE_HASHES
    assert identity['runtimePolicy']['supervisor']['sourceSha256'] == sources['executor.mjs']
    assert identity['supervisorPolicy'] == {'candidateSelectorSha256': sources['matrix-candidates.mjs'],
        'conformanceAdapterSha256': sources['matrix-native.mjs'], 'experimentSha256': sources['matrix-input-dispatcher.mjs'],
        'receiptValidatorSha256': sources['matrix-receipt.mjs'], 'inputValidatorSha256': sources['resolved-compile-certificate.mjs'],
        'candidateEnvelopeSha256': sources['matrix-candidate-envelope.mjs'], 'compilationPolicy': BATCH_POLICY,
        'candidateProfile': PROPERTY_DESCRIPTOR, 'candidateProfileSelectorSha256': sources['matrix-candidate-profiles.mjs']}
    assert identity['nativePolicy']['adapterSha256'] == sources['matrix-native.mjs']
    runtime = identity['runtimePolicy']
    assert runtime['workerdSha256'] == RUNTIME_MEMBERS['node_modules/@cloudflare/workerd-linux-64/bin/workerd']['sha256']
    assert runtime['boundaryContract'] == CONTRACT and runtime['intrinsicPolicy'] == INTRINSICS
    assert runtime['services'] == ['main', 'deny'] and runtime['globalOutbound'] == 'deny' and runtime['bindings'] == []
    assert runtime['compatibilityDate'] == '2026-07-01' and runtime['compatibilityFlags'] == ['nodejs_als']
    assert runtime['request'] == {'method': 'GET', 'path': '/run'} and runtime['moduleName'] == 'worker.mjs'
    assert set(runtime['processEnv']) == {'PATH', 'TZ', 'LANG'} and runtime['processEnv']['TZ'] == 'UTC' and runtime['processEnv']['LANG'] == 'C.UTF-8'
    assert identity['nativePolicy']['host'] == identity['host'] and identity['nativePolicy']['nodeVersion'] == 'v24.18.0'
    assert runtime['supervisor']['nodeExecutableSha256'] == identity['nativePolicy']['nodeExecutableSha256']
    assert all(digest(runtime[key]) for key in ['workerdSha256', 'nativeFingerprintSha256', 'denyModuleSha256'])
    policy = identity['compile']['policy']
    assert policy['keepNames'] is True and policy['external'] == ['node:async_hooks']
    assert policy['dynamicCode'] == 'private-sticky-denial-plus-runtime-constructor-guards'
    assert policy['lazyGlob'] == 'actual-real-test-entry-only; all-other-matches-sticky-denied'
    alias = row['project'] if row['project'].startswith('zod-') else None
    assert identity['preparation']['zodAlias'] == alias and (policy.get('zodAlias') or None) == alias
    assert identity['preparation']['providedContext'] == ({'expectedZodAlias': alias} if alias else {})
    for source in identity['compile']['sources']:
        if source['source'] in ['/quod-adapter/prelude.mjs', '/quod-adapter/runner.mjs']:
            assert source['sourceSha256'] == sources[source['source'].split('/')[-1]]


def full_pin(value):
    return isinstance(value, str) and re.fullmatch('[0-9a-f]{40}', value) and value != '0' * 40


def checked_pins():
    global MAX_ARCHIVE, MAX_EXPANDED, MAX_FILE, MAX_PATHS
    assert REVIEW_APPROVED is True and IMPLEMENTATION_COMPLETE is True, 'DISABLED future property47 packaged data reader; review/pins pending before index'
    assert digest(DISPATCHER), 'PENDING trusted runtime dispatcher source SHA'
    assert type(RUN) is int and RUN > 0, 'PENDING independently reviewed full-job run pin'
    assert all(full_pin(value) for value in [WORKFLOW_SOURCE, ACTION_B, RUNTIME_Q]), 'PENDING trusted W/B/Q Git pins'
    assert set(ARTIFACTS) == set(JOB_IDS) == set(JOB_TIMESTAMPS) == set(ROLES)
    for role in ROLES:
        value = ARTIFACTS[role]
        assert isinstance(value, tuple) and len(value) == 5, 'PENDING artifact ID/bytes/SHA/job/name: ' + role
        assert type(value[0]) is int and value[0] > 0 and type(value[1]) is int and value[1] > 0 and digest(value[2])
        assert value[3] == JOB_IDS[role] and type(value[3]) is int and value[3] > 0 and value[4] == JOB_NAMES[role]
        assert isinstance(JOB_TIMESTAMPS[role], tuple) and len(JOB_TIMESTAMPS[role]) == 2
    assert isinstance(STANDARD_HELPER, tuple) and len(STANDARD_HELPER) == 4, 'PENDING exact standard changes-helper pin'
    assert type(STANDARD_HELPER[0]) is int and STANDARD_HELPER[0] > 0 and STANDARD_HELPER[0] not in JOB_IDS.values()
    assert STANDARD_HELPER[1] == 'changes'
    assert all(isinstance(value, str) and re.fullmatch(r'\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z', value)
        for value in STANDARD_HELPER[2:])
    for profile in [ARCHIVE_PROFILE, RUNTIME_TAR_PROFILE]:
        exact(profile, ['compressedBytes', 'expandedBytes', 'fileBytes', 'paths'])
        assert all(type(value) is int and value > 0 for value in profile.values()), 'PENDING explicit reviewed bounded data profile'
    assert RUNTIME_TAR_PROFILE == REVIEWED_RUNTIME_TAR_PROFILE
    MAX_ARCHIVE, MAX_EXPANDED, MAX_FILE, MAX_PATHS = (ARCHIVE_PROFILE[key]
        for key in ['compressedBytes', 'expandedBytes', 'fileBytes', 'paths'])
    assert all(value[1] <= MAX_ARCHIVE for value in ARTIFACTS.values())
    assert IMPLEMENTATION_COMPLETE is True, 'PENDING completed packaged-adapter review; cannot read an index or issue PASS'


def trusted_git(repository, reference, name, maximum):
    # Fixed reviewed refs plus caller-code literals. Never accept a source path
    # or Git ref taken from artifact JSON. No checkout or artifact execution.
    assert full_pin(reference) and isinstance(name, str) and name and not name.startswith('/')
    assert all(part and part not in {'.', '..'} for part in name.split('/'))
    env = {**os.environ, 'GIT_NO_LAZY_FETCH': '1', 'GIT_TERMINAL_PROMPT': '0'}
    object_name = reference + ':' + name
    size = subprocess.check_output(['git', 'cat-file', '-s', object_name], cwd=repository, env=env)
    assert size.strip().isdigit() and 0 < int(size) <= maximum, 'trusted Git byte bound exceeded'
    value = subprocess.check_output(['git', 'show', object_name], cwd=repository, env=env)
    assert len(value) == int(size)
    return value


def metadata(index):
    exact(index, ['schema', 'runJson', 'jobsJson', 'artifactsJson', 'evidence'])
    assert index['schema'] == 'quod-property47-packaged-action-audit-inputs-v1'
    assert set(index['evidence']) == set(ROLES) | {'baseline'}
    run, jobs, artifacts = (read(index[key], 8 * 1024 * 1024)
        for key in ['runJson', 'jobsJson', 'artifactsJson'])
    assert run['id'] == RUN and run['head_sha'] == WORKFLOW_SOURCE and run['event'] == 'workflow_dispatch'
    assert run['path'] == '.github/workflows/gate.yml' and run['run_attempt'] == 1
    assert run['status'] == 'completed' and run['conclusion'] == 'success'
    by_job = {row['id']: row for row in jobs['jobs']}
    by_artifact = {row['id']: row for row in artifacts['artifacts']}
    assert len(by_job) == len(jobs['jobs']) and len(by_artifact) == len(artifacts['artifacts'])
    assert {row['id'] for row in jobs['jobs'] if row['conclusion'] != 'skipped'} == set(JOB_IDS.values()) | {STANDARD_HELPER[0]}
    times = {}
    for role in ROLES:
        expected = ARTIFACTS[role]; artifact, job = by_artifact[expected[0]], by_job[expected[3]]
        assert artifact['name'] == ARTIFACT_NAMES[role] and artifact['expired'] is False
        assert artifact['workflow_run']['id'] == RUN and artifact['workflow_run']['head_sha'] == WORKFLOW_SOURCE
        assert artifact['size_in_bytes'] == expected[1] and artifact['digest'] == 'sha256:' + expected[2]
        assert job['name'] == expected[4] and job['status'] == 'completed' and job['conclusion'] == 'success'
        assert job['run_id'] == RUN and job['head_sha'] == WORKFLOW_SOURCE and job['run_attempt'] == 1
        for step in ['install', 'build', 'test']:
            found = [row for row in job['steps'] if row['name'] == step]
            assert len(found) == 1 and found[0]['conclusion'] == 'success'
        assert (job['started_at'], job['completed_at']) == JOB_TIMESTAMPS[role]
        start, end = (datetime.datetime.fromisoformat(value.replace('Z', '+00:00')) for value in JOB_TIMESTAMPS[role])
        seconds = (end - start).total_seconds(); assert math.isfinite(seconds) and seconds > 0
        times[role] = {'completeJobSeconds': seconds, 'modeledRoundedMinutes': math.ceil(seconds / 60)}
    helper = by_job[STANDARD_HELPER[0]]
    assert helper['name'] == STANDARD_HELPER[1] and helper['run_id'] == RUN and helper['head_sha'] == WORKFLOW_SOURCE and helper['run_attempt'] == 1
    assert helper['status'] == 'completed' and helper['conclusion'] == 'success'
    assert (helper['started_at'], helper['completed_at']) == STANDARD_HELPER[2:]
    helper_start, helper_end = (datetime.datetime.fromisoformat(value.replace('Z', '+00:00')) for value in STANDARD_HELPER[2:])
    helper_seconds = (helper_end - helper_start).total_seconds()
    assert math.isfinite(helper_seconds) and helper_seconds > 0
    times['standardHelper'] = {'jobId': STANDARD_HELPER[0], 'name': STANDARD_HELPER[1],
        'startedAt': helper['started_at'], 'completedAt': helper['completed_at'],
        'completeJobSeconds': helper_seconds, 'modeledRoundedMinutes': math.ceil(helper_seconds / 60)}
    assert len({by_job[JOB_IDS[role]]['runner_id'] for role in ROLES}) == 4
    assert JOB_TIMESTAMPS['consumer'][0] >= max(JOB_TIMESTAMPS[role][1] for role in ['seed', 'control'])
    return times


def logs(text, marker):
    found, releases = [], []
    for raw_line in text.splitlines():
        line = re.sub(r'^[-0-9TZ:.]+ ', '', raw_line)
        line = re.sub(r'\x1b\[[0-9;]*m', '', line)
        if line.startswith(marker + ' '):
            assert len(line.encode()) <= 65536
            found.append(decode(line[len(marker) + 1:]))
        if re.fullmatch(r'(?:# |ℹ )(?:tests|pass|fail|cancelled|skipped|todo) [0-9]+', line):
            releases.append(re.sub(r'^(?:# |ℹ )', '', line))
    assert len(found) == 1 and sorted(releases) == RELEASE
    return found[0]


def tar_inventory(body):
    """Hash exact trusted-B tar members as data, never extract or execute.

    Bounds remain disabled until exact reviewed profile pins are supplied.
    Audit contained symlink/hardlink graphs as data; never create links.
    The exact B-byte equality is required by the caller before parsing.
    """
    assert len(body) <= RUNTIME_TAR_PROFILE['compressedBytes']
    entries, spelling, total, headers, root_seen = {}, {}, 0, 0, False
    # Streaming iteration avoids getmembers() allocating from unbounded input.
    with tarfile.open(fileobj=io.BytesIO(body), mode='r|gz') as archived:
        for item in archived:
            headers += 1; assert headers <= RUNTIME_TAR_PROFILE['paths']
            name = item.name
            if name in {'.', './'}:
                assert item.isdir() and not root_seen
                root_seen = True
                continue
            if name.startswith('./'):
                name = name[2:]
            name = name.rstrip('/')
            parts = pathlib.PurePosixPath(name).parts
            assert name and not name.startswith('/') and all(part not in {'', '.', '..'} for part in parts)
            assert not any(char in name for char in ('\\', '\0', '\r', '\n', ':'))
            assert pathlib.PurePosixPath(name).as_posix() == name and len(name.encode()) <= 4096 and len(parts) <= 128
            assert name not in entries
            for node in [name, *[parent.as_posix() for parent in pathlib.PurePosixPath(name).parents if parent.as_posix() != '.']]:
                key = unicodedata.normalize('NFC', node).casefold()
                assert key not in spelling or spelling[key] == node
                spelling[key] = node
            assert len(spelling) <= RUNTIME_TAR_PROFILE['paths']
            assert item.isdir() or item.isfile() or item.issym() or item.islnk(), 'special runtime tar member'
            assert 0 <= item.size <= RUNTIME_TAR_PROFILE['fileBytes']
            total += item.size; assert total <= RUNTIME_TAR_PROFILE['expandedBytes']
            entry = {'mode': item.mode & 0o777, 'type': 'dir' if item.isdir() else 'symlink' if item.issym()
                else 'hardlink' if item.islnk() else 'file'}
            if item.isfile():
                stream = archived.extractfile(item); assert stream is not None
                h, size = hashlib.sha256(), 0
                for block in iter(lambda: stream.read(1024 * 1024), b''):
                    size += len(block); assert size <= item.size
                    h.update(block)
                assert size == item.size
                entry.update(bytes=size, sha256=h.hexdigest())
            elif item.issym() or item.islnk():
                target = item.linkname
                assert target and not target.startswith('/') and not any(char in target for char in ('\\', '\0', '\r', '\n', ':'))
                normalized = posixpath.normpath(posixpath.join(posixpath.dirname(name), target)
                    if item.issym() else target)
                assert normalized not in {'.', '..'} and not normalized.startswith('../')
                entry['target'] = target
            entries[name] = entry
    # Do not follow OS symlinks. Walk the recorded tar graph only. Every
    # symlink hop stays inside this namespace and terminates at a real member.
    def resolve(name, seen=()):
        assert len(seen) < 40 and name not in seen
        parts = name.split('/')
        for index in range(1, len(parts) + 1):
            prefix = '/'.join(parts[:index]); value = entries.get(prefix)
            assert value is not None, 'missing runtime tar ancestor or target'
            if value['type'] in {'symlink', 'hardlink'}:
                target = posixpath.normpath(posixpath.join(posixpath.dirname(prefix), value['target'], *parts[index:])
                    if value['type'] == 'symlink' else posixpath.join(value['target'], *parts[index:]))
                assert target not in {'.', '..'} and not target.startswith('../') and not target.startswith('/')
                return resolve(target, (*seen, name))
            if index < len(parts):
                assert value['type'] == 'dir'
        return name
    for name in entries:
        target = resolve(name)
        if entries[name]['type'] in {'symlink', 'hardlink'}:
            assert entries[target]['type'] == 'file', 'runtime executable link must terminate at a regular file'
            entries[name].update(resolved=target, bytes=entries[target]['bytes'], sha256=entries[target]['sha256'],
                resolvedMode=entries[target]['mode'])
    expected_links = {'node_modules/.bin/esbuild': ('symlink', '../esbuild/bin/esbuild'),
        'node_modules/.bin/workerd': ('symlink', '../workerd/bin/workerd'),
        'node_modules/esbuild/bin/esbuild': ('hardlink', './node_modules/@esbuild/linux-x64/bin/esbuild'),
        'node_modules/workerd/bin/workerd': ('hardlink', './node_modules/@cloudflare/workerd-linux-64/bin/workerd')}
    assert {name: (row['type'], row['target']) for name, row in entries.items()
        if row['type'] in {'symlink', 'hardlink'}} == expected_links
    return {'members': entries, 'expandedBytes': total, 'memberCount': len(entries), 'tarHeaderEntries': headers,
        'rootDirectoryPresent': root_seen}


# Exact source literal roster independently matched both verified native VM repeats.
RELEASE_NAMES = ['local release writes changelog/version/lockfile and pushes main without a tag',
 'dirty trees are rejected before versioning',
 'failed checks do not bump or commit a version',
 'rejected pushes preserve the release commit for recovery and create no tags',
 "watch follows this commit's CI run and opens npm after success",
 'failed CI never opens the npm approval page',
 'watch without local npm authentication prints manual approval instructions',
 'alpha releases keep their prerelease identifier',
 'CI uses the release commit even when ordinary commits follow it',
 'CI rejects multiple version bumps in one push',
 'CI does not release an ordinary push',
 'release validation requires notes in the current version section',
 'CI validates complete SemVer and chooses npm-compatible dist-tags',
 'manual dispatch stages the selected branch tip or an earlier release commit',
 'tag job creates the version tag at the staged SHA',
 'tag retries accept the same commit without rewriting the ref',
 'tag job refuses an existing tag pointing elsewhere',
 'tag permission failures remain failures',
 'release jobs use the planned commit and channel with separate credentials',
 'releases are automatic only on main, with explicit dispatch and a fork guard']
RELEASE_SOURCES = {'scripts/release.test.mjs': {'bytes': 14833,
                              'sha256': '230260d28784f5b5fe9d5c4f03c1aec8a4281a71f077bc303bca1c6068dfde9e',
                              'tests': ['local release writes changelog/version/lockfile and pushes main '
                                        'without a tag',
                                        'dirty trees are rejected before versioning',
                                        'failed checks do not bump or commit a version',
                                        'rejected pushes preserve the release commit for recovery and create '
                                        'no tags',
                                        "watch follows this commit's CI run and opens npm after success",
                                        'failed CI never opens the npm approval page',
                                        'watch without local npm authentication prints manual approval '
                                        'instructions',
                                        'alpha releases keep their prerelease identifier',
                                        'CI uses the release commit even when ordinary commits follow it',
                                        'CI rejects multiple version bumps in one push',
                                        'CI does not release an ordinary push',
                                        'release validation requires notes in the current version section',
                                        'CI validates complete SemVer and chooses npm-compatible dist-tags',
                                        'manual dispatch stages the selected branch tip or an earlier '
                                        'release commit']},
 'scripts/tag-release.test.mjs': {'bytes': 5687,
                                  'sha256': '12ecbc31201a7a3d464e63dd3b92647fafd42c59d20b4f6f01fd2a6b42f60ca5',
                                  'tests': ['tag job creates the version tag at the staged SHA',
                                            'tag retries accept the same commit without rewriting the ref',
                                            'tag job refuses an existing tag pointing elsewhere',
                                            'tag permission failures remain failures',
                                            'release jobs use the planned commit and channel with separate '
                                            'credentials',
                                            'releases are automatic only on main, with explicit dispatch and '
                                            'a fork guard']}}


def matrix_admission(role_roots, dispatches, controls, sources, candidate_ids, baseline, pack, pack_sha, service_key, transport, expected_geometry):
    consumer = dispatches['consumer']
    consumer_root = role_roots['consumer'] / 'invocation'
    by_role = {role: instance_map(value['instances']) for role, value in dispatches.items()}
    prepared_by_role = {role: {row['instanceId']: row['identity']
        for row in read(root / 'invocation/preparation.json')['prepared']}
        for role, root in role_roots.items() if role in dispatches}
    entries = {e['executionKey']: e for e in pack['entries']}
    events_list = [e for e in transport['consumer'] if e['operation'] == 'collection-entry']
    events = {e['executionKey']: e for e in events_list}
    assert len(entries) == len(events) == len(events_list) == 1 and set(entries) == set(events)
    seed_native = instance_map(controls['seed']['executed'])
    certificates, outcome_counts, project_counts = [], collections.Counter(), collections.Counter()
    # Preserve current preparation order: the shared resolver deduplicates in
    # first-occurrence order, which is bound by its actual request digest.
    for current in consumer['instances']:
        iid = current['instanceId']
        if iid not in candidate_ids:
            continue
        seed_row = by_role['seed'][iid]; control_row = by_role['control'][iid]
        assert seed_row['decision'] == control_row['decision'] == 'managed' and current['decision'] == 'replay'
        assert current['compilationSkipped'] is True and current['compilationSeconds'] == 0 and not current['nativeExecuted']
        assert not current.get('workerEvidenceDirectory')
        directory = consumer_root / current['evidenceDirectory']
        execution = read(directory / 'execution-identity.json'); identity = execution['identity']; key = execution['key']
        assert key == domain(EXECUTION, identity) == current['executionKey']
        assert execution == read(role_roots['seed'] / 'invocation' / seed_row['evidenceDirectory'] / 'execution-identity.json')
        assert execution == read(role_roots['control'] / 'invocation' / control_row['evidenceDirectory'] / 'execution-identity.json')
        check_execution(identity, current, sources)
        assert identity['runtimePolicy']['nativeFingerprintSha256'] == consumer['nativeFingerprintSha256']
        assert identity['host'] == consumer['host'] and identity['nativePolicy'] == consumer['nativePolicy']
        assert identity['supervisorPolicy'] == consumer['supervisorPolicy']
        assert read(directory / 'preparation.json') == identity['preparation']
        assert all(prepared_by_role[role][iid] == identity['preparation'] for role in dispatches)
        options = {'epoch': 1700000000000, 'seed': 42, 'env': {'NODE_ENV': 'test'}, 'vitest': None,
            'file': '/project/' + current['file'], 'providedContext': identity['preparation']['providedContext']}
        seed_built = read(role_roots['seed'] / 'invocation' / seed_row['evidenceDirectory'] / 'test-build.json')
        options['vitest'] = seed_built['options']['vitest']
        if identity['preparation']['zodAlias']:
            options['zodAlias'] = identity['preparation']['zodAlias']
        assert options == seed_built['options'] and sha(canonical(options).encode()) == identity['compile']['configurationSha256']
        assert sha(canonical(options['vitest']).encode()) == identity['preparation']['configurationSha256']
        route = {'preparation': identity['preparation'], 'options': options, 'runtimePolicy': identity['runtimePolicy'],
            'supervisorPolicy': identity['supervisorPolicy'], 'host': identity['host'], 'nativePolicy': identity['nativePolicy']}
        candidate_key = domain(ENVELOPE, route)
        assert candidate_key == current['candidateKey'] == seed_row['candidateKey'] == control_row['candidateKey']
        entry, event = entries[candidate_key], events[candidate_key]
        exact(entry, ['executionKey', 'receiptBase64', 'receiptSha256'])
        body = b64(entry['receiptBase64'], MAX_ITEM)
        assert sha(body) == entry['receiptSha256'] == event['receiptSha256']
        assert event['matchedServiceKey'] == service_key and event['collectionSha256'] == pack_sha
        assert body == raw(directory / 'candidate-envelope.json', MAX_ITEM)
        assert body == raw(role_roots['seed'] / 'invocation' / seed_row['evidenceDirectory'] / 'recorded-candidate.json', MAX_ITEM)
        envelope, cert, cert_bytes, receipt, receipt_bytes = unpack_envelope(body, candidate_key, entry['receiptSha256'])
        assert read(directory / 'candidate-authentication.json') == {'candidateKey': candidate_key,
            'trustedEnvelopeSha256': entry['receiptSha256'], 'trustedCertificateSha256': sha(cert_bytes), 'trustedReceiptSha256': sha(receipt_bytes)}
        certificate(cert, identity, sources); bind_tool_members(cert['tools']); certificates.append(cert)
        assert receipt['execution'] == execution and sha(receipt_bytes) == current['receiptSha256'] == seed_row['receiptSha256']
        assert receipt_bytes == raw(role_roots['seed'] / 'invocation/receipts' / (iid + '.json'), MAX_ITEM)
        for role, row in [('seed', seed_row), ('control', control_row)]:
            fresh_dir = role_roots[role] / 'invocation' / row['evidenceDirectory']
            assert raw(fresh_dir / 'compile-certificate.json', MAX_ITEM) == cert_bytes
            built = read(fresh_dir / 'test-build.json')
            assert built['identity'] == identity['compile'] and built['preparation'] == identity['preparation'] and built['options'] == options
            assert built['identitySha256'] == sha(canonical(identity['compile']).encode())
            worker_dir = role_roots[role] / 'invocation' / row['workerEvidenceDirectory']
            phase = next(phase for phase in dispatches[role]['phases'] if phase['name'] == worker_dir.name)
            response_bytes = worker_evidence(worker_dir, identity['compile'], identity['runtimePolicy'], identity['host'], phase)
            assert sha(response_bytes) == row['responseSha256']
            data = read(worker_dir / 'response.json'); boundary(data)
            fresh_native = instance_map(controls[role]['executed'])[iid]
            assert collections.Counter((test['name'], test['state']) for test in data['rows']) == collections.Counter((test['name'], test['state']) for test in fresh_native['rows'])
            assert data['files'][0]['name'] == fresh_native['relativeModuleId'] and data['files'][0]['state'] == fresh_native['state']
            assert row['comparison']['equal'] is True and row['fileMatches'] is True and not row.get('disagreement', False)
        response = b64(receipt['response']['bytes'], 16 * 1024 * 1024)
        assert response == raw(directory / 'replayed-response.json')
        assert response == raw(role_roots['seed'] / 'invocation' / seed_row['workerEvidenceDirectory'] / 'response.json')
        data = decode(response); boundary(data)
        assert sha(response) == receipt['response']['sha256'] and receipt['response']['encoding'] == 'base64'
        exact(receipt, ['schema', 'researchOnly', 'execution', 'conformance', 'conformanceSha256', 'response', 'resultDigest'])
        assert receipt['schema'] == RECEIPT and receipt['researchOnly'] is True
        conformance = receipt['conformance']; original = seed_native[iid]
        exact(conformance, ['schema', 'executionKey', 'instance', 'nativeReportSha256', 'nativeSelectionSha256', 'nativePolicySha256', 'file', 'rows'])
        assert conformance['schema'] == CONFORMANCE and conformance['executionKey'] == key and conformance['instance'] == identity['instance']
        assert conformance['nativeReportSha256'] == controls['seed']['reportSha256']
        assert conformance['nativeSelectionSha256'] == sha(raw(role_roots['seed'] / 'invocation/native/native-selection.json'))
        assert conformance['nativePolicySha256'] == sha(canonical(identity['nativePolicy']).encode())
        expected = collections.Counter((test['name'], test['state']) for test in original['rows'])
        assert collections.Counter((test['name'], test['state']) for test in data['rows']) == expected
        assert collections.Counter((test['name'], test['state']) for test in conformance['rows']) == expected
        assert conformance['file'] == {'name': original['relativeModuleId'], 'state': original['state']}
        assert data['files'][0]['name'] == original['relativeModuleId'] and data['files'][0]['state'] == original['state']
        assert receipt['conformanceSha256'] == domain(CONFORMANCE, conformance)
        assert receipt['resultDigest'] == sha((RECEIPT + '\n' + key + '\n' + receipt['conformanceSha256'] + '\n' + receipt['response']['bytes']).encode())
        decision = read(directory / 'input-decision.json')
        assert decision['hit'] is True and decision['identitySha256'] == sha(canonical(identity['compile']).encode())
        assert decision['bundleSha256'] == identity['compile']['bundleSha256'] and 'proof' not in decision and 'identity' not in decision
        assert decision['authority'] == 'input equivalence only; authenticated passing receipt admission still required'
        outcome_counts.update(test['state'] for test in data['rows']); project_counts[current['project']] += 1
    assert len(certificates) == 1
    requests = resolution_requests(certificates)
    tools = certificates[0]['tools']; geometry = certificates[0]['rootGeometry']
    assert all(value['tools'] == tools and value['rootGeometry'] == geometry for value in certificates)
    assert geometry == expected_geometry
    request_body = {'project': geometry['project'], 'adapterRoot': geometry['adapter'], 'requests': requests,
        'expectedToolsSha256': sha(canonical(tools).encode())}
    request_sha = sha((canonical(request_body) + '\n').encode())
    for row in consumer['instances']:
        if not row['candidate']:
            continue
        decision = read(consumer_root / row['evidenceDirectory'] / 'input-decision.json')
        assert decision['uniqueResolverRequests'] == len(requests)
        assert decision['resolver'] == {'requestSha256': request_sha, 'toolIdentitySha256': sha(canonical(tools).encode()),
            'node': 'v24.18.0', 'execArgv': [], 'profile': 'one new plain Node process and one new resolver-only esbuild build'}
    ordinary = controls['consumer']
    assert set(instance_map(ordinary['executed'])) == set(instance_map(baseline['originalSelected'])) - candidate_ids
    assert set(instance_map(ordinary['omittedAsHits'])) == candidate_ids and len(ordinary['executed']) == 119
    baseline_ordinary = [row for row in baseline['executed'] if row['instanceId'] not in candidate_ids]
    assert state_rows(ordinary['executed']) == state_rows(baseline_ordinary), 'actual consumer native119 test states differ from baseline'
    assert file_rows(ordinary['executed']) == file_rows(baseline_ordinary), 'actual consumer native119 API file states differ from baseline'
    assert sum(state_rows(ordinary['executed']).values()) == 4084 and outcome_counts == {'pass': 47}
    return {'certificates': len(certificates), 'resolverRequests': len(requests),
        'replayedStates': dict(outcome_counts), 'candidateProjects': dict(sorted(project_counts.items()))}



RUNTIME_PREFIX = 'scripts/bench-vitest/managed/'
RUNTIME_FILES = ['compiler.mjs', 'preparation.mjs', 'executor.mjs', 'prelude.mjs', 'runner.mjs',
    'matrix-native.mjs', 'matrix-input-dispatcher.mjs', 'matrix-candidates.mjs', 'matrix-candidate-profiles.mjs', 'matrix-receipt.mjs',
    'matrix-candidate-envelope.mjs', 'resolved-compile-certificate.mjs', 'package.json', 'package-lock.json',
    'receipt.mjs', 'dispatcher.mjs', 'ordinary-remainder.mjs', 'preflight.mjs']
DISPATCHER = '36aa6899058b2efdbc55c5e072dc7a7129974189c768533a55a953104b59231a'
V0_CANDIDATE = 'packages/convex-helpers/server/stream.property.test.ts'
V0_EXPECTED = {'count': 47, 'file': 'server/stream.property.test.ts'}
V0_SOURCE = '9798bab303233bca846a89ccb39ce28bd968329e0704f3e5d67adca934c11c75'
V0_EXECUTION = 'quod-managed-execution-v1-research'
V0_RECEIPT = 'quod-managed-receipt-v1-research'


def trusted_names(repository, reference, prefix):
    assert full_pin(reference) and prefix in {RUNTIME_PREFIX, 'dist/'}
    env = {**os.environ, 'GIT_NO_LAZY_FETCH': '1', 'GIT_TERMINAL_PROMPT': '0'}
    value = subprocess.check_output(['git', 'ls-tree', '-r', '-z', '--name-only', reference, '--', prefix],
        cwd=repository, env=env)
    assert len(value) <= 2 * 1024 * 1024
    names = value.decode().rstrip('\0').split('\0')
    assert names and len(names) == len(set(names)) and all(name.startswith(prefix) for name in names)
    return names


def source_inputs():
    global PREPARATION_SOURCE_HASHES, PROJECT_DATA_SOURCE_HASHES
    bodies = {name: trusted_git(RUNTIME_REPOSITORY, RUNTIME_Q, RUNTIME_PREFIX + name, 2 * 1024 * 1024)
        for name in RUNTIME_FILES}
    sources = {name: sha(body) for name, body in bodies.items()}
    preparation_names = ['vitest.config.mts', 'scripts/zodVersions.mjs',
        'packages/convex-helpers/vitest.config.mts', 'example-ui/vite.config.mts']
    PREPARATION_SOURCE_HASHES = {name: sha(trusted_git(PROJECT_REPOSITORY, PROJECT_C11, name, 1024 * 1024))
        for name in preparation_names}
    PROJECT_DATA_SOURCE_HASHES = {name: sha(trusted_git(PROJECT_REPOSITORY, PROJECT_C11, name, 4 * 1024 * 1024))
        for name in ['package.json', 'package-lock.json']}
    assert sha(trusted_git(PROJECT_REPOSITORY, PROJECT_C11, V0_CANDIDATE, 2 * 1024 * 1024)) == V0_SOURCE
    assert sources['compiler.mjs'] == COMPILER and sources['resolved-compile-certificate.mjs'] == BATCH_CORE
    assert sources['matrix-input-dispatcher.mjs'] == DISPATCHER
    literal = bodies['matrix-candidates.mjs'].decode().split('Object.freeze(', 1)[1].split('.map(row => Object.freeze(row))', 1)[0]
    default_roster = decode(literal); default_ids = set(instance_map(default_roster)); assert len(default_ids) == 108
    assert sources['matrix-candidate-profiles.mjs'] == PROPERTY_SELECTOR_SHA256
    assert [row for row in default_roster if row['instanceId'] == PROPERTY_TARGET['instanceId']] == [PROPERTY_TARGET]
    candidate_ids = {PROPERTY_TARGET['instanceId']}
    release = []
    for name, expected in RELEASE_SOURCES.items():
        body = trusted_git(PROJECT_REPOSITORY, PROJECT_C11, name, 1024 * 1024)
        assert len(body) == expected['bytes'] and sha(body) == expected['sha256']
        names = [decode('"' + value + '"') for value in re.findall(r'^test\("((?:[^"\\]|\\.)*)",', body.decode(), re.M)]
        assert names == expected['tests']; release += names
    assert release == RELEASE_NAMES and len(set(release)) == len(release) == 20
    package = decode(trusted_git(PROJECT_REPOSITORY, PROJECT_C11, 'package.json', 1024 * 1024))
    assert package['scripts']['test:release'] == 'node --test scripts/*.test.mjs'
    assert package['scripts']['test'] == 'vitest run && npm run test:release'
    helper = trusted_git(WORKFLOW_REPOSITORY, WORKFLOW_SOURCE,
        RUNTIME_PREFIX + 'quod-property47-packaged-proof-probe.cjs', 1024 * 1024)
    workflow = trusted_git(WORKFLOW_REPOSITORY, WORKFLOW_SOURCE, '.github/workflows/gate.yml', 2 * 1024 * 1024)
    assert ("const ACTION_B = '" + ACTION_B + "'").encode() in helper
    assert ("const SOURCE_Q = '" + RUNTIME_Q + "'").encode() in helper
    assert workflow.count(('uses: pango07/quod-action@' + ACTION_B).encode()) == 4
    assert workflow.count(("PACKAGED_ACTION_SHA: '" + ACTION_B + "'").encode()) == 4
    assert workflow.count(("PACKAGED_SOURCE_SHA: '" + RUNTIME_Q + "'").encode()) == 4
    text = workflow.decode()
    for role in ROLES:
        match = re.search(r'^  packaged-property47-' + role + r':\n(.*?)(?=^  [A-Za-z0-9_-]+:|\Z)', text, re.M | re.S)
        assert match is not None
        block = match[1]
        assert "inputs.only == 'benchmark-managed-property47-packaged'" in block
        assert '          repository: get-convex/convex-helpers\n          ref: ' + PROJECT_C11 + '\n' in block
        assert '      - name: install\n        run: npm ci\n' in block
        assert '      - name: build\n        run: npm run build\n' in block
        assert '          npm run test 2>&1 | tee "$RUNNER_TEMP/property47-packaged-proof-' + role + '/test.log"' in block
        assert "          managed-vitest: '" + ('v0' if role == 'v0' else 'matrix-property47-control' if role == 'control' else 'matrix-property47') + "'" in block
        assert "          record: '" + ('true' if role in {'seed', 'v0'} else 'false') + "'" in block
        assert "          managed-env: '{\"NODE_ENV\":\"test\"}'" in block
        assert "          managed-epoch: '1700000000000'" in block and "          managed-seed: '42'" in block
        assert "          cloud: 'false'" in block and "          task-cache: 'false'" in block
        if role == 'consumer':
            assert '    needs: [packaged-property47-seed, packaged-property47-control]' in block
    return bodies, sources, candidate_ids, helper, workflow


def action_inputs(runtime_bodies):
    manifest_bytes = trusted_git(ACTION_REPOSITORY, ACTION_B, 'dist/managed/manifest.json', 1024 * 1024)
    manifest = decode(manifest_bytes)
    assert manifest == {'schema': 'quod-managed-action-runtime-v1',
        'source': {'repository': 'pango07/quod', 'commit': RUNTIME_Q}, 'sha256': manifest['sha256']}
    body = trusted_git(ACTION_REPOSITORY, ACTION_B, 'dist/managed/runtime.tar.gz', RUNTIME_TAR_PROFILE['compressedBytes'])
    assert sha(body) == manifest['sha256']
    members = tar_inventory(body)
    source_names = trusted_names(RUNTIME_REPOSITORY, RUNTIME_Q, RUNTIME_PREFIX)
    top = {name[len(RUNTIME_PREFIX):] for name in source_names
        if '/' not in name[len(RUNTIME_PREFIX):] and name.endswith('.mjs')}
    assert {name for name in members['members'] if '/' not in name and name.endswith('.mjs')} == top
    for name in sorted(top | {'package.json', 'package-lock.json'}):
        source = runtime_bodies.get(name)
        if source is None:
            source = trusted_git(RUNTIME_REPOSITORY, RUNTIME_Q, RUNTIME_PREFIX + name, 2 * 1024 * 1024)
        actual = members['members'][name]
        assert actual['type'] == 'file' and actual['bytes'] == len(source) and actual['sha256'] == sha(source)
    ncc_names = sorted(name[len('dist/'):] for name in trusted_names(ACTION_REPOSITORY, ACTION_B, 'dist/')
        if name.endswith('.js') and name[len('dist/'):].split('/')[0] in NCC_BUNDLES)
    assert len(ncc_names) == 12
    assert set(ncc_names) == {bundle + '/' + name for bundle in NCC_BUNDLES for name in ['index.js', '50.index.js', '835.index.js']}
    ncc = []
    for name in ncc_names:
        value = trusted_git(ACTION_REPOSITORY, ACTION_B, 'dist/' + name, 16 * 1024 * 1024)
        ncc.append({'file': name, 'bytes': len(value), 'sha256': sha(value)})
    source_hashes = {name: sha(trusted_git(ACTION_REPOSITORY, ACTION_B, name, 2 * 1024 * 1024))
        for name in ACTION_SOURCES}
    return manifest_bytes, body, members, ncc, source_hashes


def bind_tool_members(tools):
    # Both observed adapter tool trees must be exactly the bytes shipped in B.
    # Project TS wrapper/delegate trees are bound by the separately reviewed
    # four-tree literal digest, actual routes and pinned validator policy.
    for row in tools['files']:
        if not row['name'].startswith('/quod-adapter/node_modules/'):
            continue
        name = row['name'][len('/quod-adapter/'):]; member = RUNTIME_MEMBERS[name]
        if row['kind'] == 'directory':
            assert member['type'] == 'dir' and row['mode'] == member['mode']
        elif row['kind'] == 'file':
            assert member['type'] in {'file', 'hardlink'} and row['mode'] == member.get('resolvedMode', member['mode'])
            assert row['bytes'] == member['bytes'] and row['sha256'] == member['sha256']
        else:
            raise AssertionError('unexpected reviewed adapter tool inventory kind')


def namespace(role):
    return 'packaged-property47-' + str(RUN) + '-' + ('v0' if role == 'v0' else 'property47-control' if role == 'control' else 'property47') + '-'


def complete_log(file, marker):
    text = raw(file, MAX_FILE).decode()
    event = logs(text, marker)
    names = []
    for raw_line in text.splitlines():
        line = re.sub(r'^[-0-9TZ:.]+ ', '', raw_line)
        line = re.sub(r'\x1b\[[0-9;]*m', '', line)
        assert not line.startswith('not ok '), 'release TAP failure retained'
        spec = re.fullmatch(r'✔ (.+) \(([0-9.]+)(ms|s)\)', line)
        tap = re.fullmatch(r'ok [0-9]+ - (.+)', line)
        if spec:
            assert math.isfinite(float(spec[2])) and float(spec[2]) >= 0
            names.append(spec[1])
        elif tap:
            names.append(tap[1])
    assert collections.Counter(names) == collections.Counter(RELEASE_NAMES), 'exact20 release test names/results differ'
    return event


def common_role(role, root, sources, helper, manifest, runtime_bytes, ncc):
    preflight = read(root / 'preflight.json'); proof = read(root / 'proof.json')
    assert preflight['schema'] == 'quod-property47-packaged-proof-preflight-v1' and preflight['role'] == role
    assert proof['schema'] == 'quod-property47-packaged-proof-v1' and proof['role'] == role and proof['passed'] is True
    assert raw(root / 'proof-probe.cjs', 1024 * 1024) == helper
    for value in [preflight, proof]:
        assert value['actionPin'] == ACTION_B and value['sourcePin'] == RUNTIME_Q and value['workflowPin'] == WORKFLOW_SOURCE
        assert value['projectPin'] == PROJECT_C11 and value['namespace'] == namespace(role) and value['fullJobHit'] is False
    assert preflight['commands'] == ['npm ci', 'npm run build', 'npm run test'] and preflight['nodeVersion'] == 'v24.18.0'
    project = preflight['project']; assert isinstance(project, str) and project.startswith('/')
    assert raw(root / 'action-manifest.json', 1024 * 1024) == manifest
    assert raw(root / 'runtime.tar.gz', RUNTIME_TAR_PROFILE['compressedBytes']) == runtime_bytes
    provenance = read(root / 'runtime-provenance.json')
    assert provenance['schema'] == 'quod-property47-packaged-runtime-v1' and provenance['actionPin'] == ACTION_B and provenance['sourcePin'] == RUNTIME_Q
    assert provenance['archiveSha256'] == proof['archiveSha256'] == sha(runtime_bytes) and provenance['archiveBytes'] == len(runtime_bytes)
    assert sorted(provenance['files'], key=lambda row: row['file']) == ncc
    assert provenance['candidateProfileSelectorSha256'] == (None if role == 'v0' else sources['matrix-candidate-profiles.mjs'])
    assert proof['releaseTests'] == 20
    assert not any('cache-auth' in path.name.lower() or path.name == 'session.json' for path in root.rglob('*'))
    marker = 'QUOD_MANAGED_RESULT' if role == 'v0' else 'QUOD_MANAGED_MATRIX_INPUT_RESULT'
    event = complete_log(root / 'test.log', marker)
    return preflight, proof, event


def matrix_role(role, root, event, proof, sources, baseline, times, ncc):
    dispatch_root = root / 'invocation'; d = read(dispatch_root / 'result.json')
    assert read(root / 'bridge-summary.json') == event
    assert event['schema'] == 'quod-managed-matrix-action-result-v1'
    assert event['mode'] == ('matrix-property47-control' if role == 'control' else 'matrix-property47')
    for key in ['exitCode', 'selectedInstances', 'candidateInstances', 'replayedInstances', 'managedExecutions',
        'skippedCompilations', 'nativeSpecsExecuted', 'ordinaryInstances', 'recordedReceipts', 'logicalSha256', 'logicalStates', 'reportFidelity']:
        assert event[key] == d[key]
    assert d['schema'] == 'quod-managed-matrix-input-dispatch-v1-research' and d['productionEnabled'] is False
    assert d['passed'] is True and d['exitCode'] == 0 and d['fullJobHit'] is False and event['fullJobHit'] is False
    assert d['selectedInstances'] == 120 and d['candidateInstances'] == 1 and d['ordinaryInstances'] == 119 and d['logicalStates'] == 4131
    assert d['candidatePolicy'] == PROPERTY_PROFILE and d['candidateProfile'] == PROPERTY_DESCRIPTOR
    assert d['supervisorPolicy']['candidateProfile'] == PROPERTY_DESCRIPTOR
    assert d['supervisorPolicy']['candidateProfileSelectorSha256'] == sources['matrix-candidate-profiles.mjs']
    assert d['initialOrdinaryInstances'] == 119
    candidates = [row for row in d['instances'] if row['candidate']]
    assert len(candidates) == 1 and {key: candidates[0][key] for key in PROPERTY_TARGET} == PROPERTY_TARGET
    assert d['nativeConformanceInstances'] == d['managedExecutions'] == (0 if role == 'consumer' else 1)
    children = list((dispatch_root / 'instances').iterdir())
    assert len(children) == 1 and children[0].name == PROPERTY_TARGET['instanceId'] and stat.S_ISDIR(children[0].lstat().st_mode)
    expected_builds = set() if role == 'consumer' else {'instances/' + PROPERTY_TARGET['instanceId'] + '/test-build.json'}
    expected_certs = set() if role == 'consumer' else {'instances/' + PROPERTY_TARGET['instanceId'] + '/compile-certificate.json'}
    assert {path.relative_to(dispatch_root).as_posix() for path in dispatch_root.rglob('test-build.json')} == expected_builds
    assert {path.relative_to(dispatch_root).as_posix() for path in dispatch_root.rglob('compile-certificate.json')} == expected_certs
    assert d['selectedScope'] == event['selectedScope'] == {'kind': 'complete-original-default-selection', 'files': None}
    assert d['stickyManagedFailure'] is False and d['observedNativeFailure'] is False and event['stickyManagedFailure'] is False
    scope = event['cacheTransport']; assert scope['namespace'] == namespace(role)
    expected_scope = {'schema': 'quod-managed-matrix-action-input-scope-v1',
        'source': {'repository': 'pango07/quod', 'commit': RUNTIME_Q}, 'runtimeArchiveSha256': proof['archiveSha256'],
        'bridgeSha256': next(row['sha256'] for row in ncc if row['file'] == 'matrix-bridge/index.js'),
        'nodeVersion': 'v24.18.0', 'cacheSdkVersion': '4.1.0', 'candidateProfile': PROPERTY_PROFILE}
    assert scope['scopeInput'] == expected_scope and scope['scope'] == sha(canonical(expected_scope).encode())
    assert event['authLifecycle']['sessionConsumedBeforeVitestDispatch'] is True and event['authLifecycle']['authAbsentOutsideSdk'] is True
    producer_invocation = pathlib.PurePosixPath(event['evidenceDirectory'])
    assert producer_invocation.name.startswith('matrix-run-') and producer_invocation.parent.name == 'quod'
    assert scope['file'] == str(producer_invocation / 'cache-transport.json')
    assert pathlib.PurePosixPath(event['logicalReport']['file']) == producer_invocation.parent / 'matrix-reports' / (producer_invocation.name + '.logical.json')
    body = raw(root / 'logical-results.json')
    assert body == raw(dispatch_root / 'logical-results.json')
    assert event['logicalReport']['sha256'] == sha(body) and event['logicalReport']['bytes'] == len(body)
    assert event['logicalReport']['instances'] == 120 and event['logicalReport']['states'] == 4131
    assert event['logicalReport']['schema'] == 'quod-managed-matrix-input-logical-results-v1'
    value, actual_states, actual_files = logical(dispatch_root, d, state_rows(baseline['executed']), file_rows(baseline['executed']))
    assert proof['logicalSha256'] == event['logicalSha256'] == value['sha256']
    computed_file_states = sorted([list(row)[1:] for row in actual_files.elements()], key=canonical)
    assert sha(canonical(computed_file_states).encode()) == proof['fileStatesSha256']
    native_control = native(dispatch_root / 'native')
    assert set(instance_map(native_control['originalSelected'])) == set(instance_map(baseline['originalSelected']))
    preparation = read(dispatch_root / 'preparation.json')
    assert preparation['profile'] == 'convex-helpers-original-twelve-matrix' and len(preparation['selected']) == 120
    assert set(instance_map(preparation['selected'])) == set(instance_map(baseline['selected']))
    assert {row['name']: row['sha256'] for row in preparation['reviewedSources']} == PREPARATION_SOURCE_HASHES
    assert {row['name']: row['sha256'] for row in preparation['reviewedDataSources']} == PROJECT_DATA_SOURCE_HASHES
    capability = read(dispatch_root / 'preflight/response.json'); boundary(capability, preflight=True)
    assert len(capability['checks']) == len({row['name'] for row in capability['checks']}) == 31 and all(row['pass'] is True for row in capability['checks'])
    built = read(dispatch_root / 'preflight-build.json')
    assert sha(canonical(built['identity']).encode()) == built['identitySha256'] and built['identity']['compilerSha256'] == COMPILER
    fingerprint = sha(canonical({'preflightIdentity': built['identitySha256'], 'inventory': capability['boundary']['inventory'],
        'symbols': capability['boundary']['symbolInventory']}).encode())
    assert fingerprint == d['nativeFingerprintSha256']
    phases = d['phases']; assert len(phases) == (1 if role == 'consumer' else 2)
    expected_phases = {'preflight'}
    if role != 'consumer':
        for row in d['instances']:
            if row['candidate']:
                assert row['workerEvidenceDirectory'] == 'managed-' + row['instanceId']
                expected_phases.add(row['workerEvidenceDirectory'])
    assert len({phase['name'] for phase in phases}) == len(phases) and {phase['name'] for phase in phases} == expected_phases
    assert {file.relative_to(dispatch_root).as_posix() for file in dispatch_root.rglob('worker.mjs')} == {name + '/worker.mjs' for name in expected_phases}
    assert {file.relative_to(dispatch_root).as_posix() for file in dispatch_root.rglob('invocation.json')} == {name + '/invocation.json' for name in expected_phases}
    phase = next(phase for phase in phases if phase['name'] == 'preflight')
    invocation = read(dispatch_root / 'preflight/invocation.json')
    worker_evidence(dispatch_root / 'preflight', built['identity'], invocation['runtimePolicy'], d['host'], phase)
    batch = batch_evidence(role, dispatch_root, d, sources)
    assert type(d['seconds']) in (int, float) and 0 <= d['seconds'] <= times['completeJobSeconds'] + 2
    assert event['seconds'] == d['seconds']
    times.update(dispatchSeconds=d['seconds'], inputValidationSeconds=d['inputValidationSeconds'],
        nativeSeconds=d['native']['seconds'], batch=batch)
    return d, native_control



def collection_chain(role_roots, events, dispatches, proofs):
    transports = {}
    for role in ['seed', 'control', 'consumer']:
        wrapper = read(role_roots[role] / 'invocation/cache-transport.json')
        assert wrapper['schema'] == 'quod-managed-matrix-action-cache-v1'
        transports[role] = records = wrapper['events']
        assert isinstance(records, list) and len(records) <= 2048
        counts = collections.Counter(row['operation'] for row in records)
        assert counts == ({'collection-restore': 1, 'collection-save': 1} if role == 'seed' else
            {'collection-restore': 1, 'collection-entry': 1} if role == 'consumer' else {})
    assert len({event['cacheTransport']['scope'] for event in events.values()}) == 1
    assert namespace('seed') == namespace('consumer') != namespace('control')
    scope = events['seed']['cacheTransport']['scope']
    pack_bytes = raw(role_roots['seed'] / 'invocation/published-collection.json', MAX_PACK)
    assert pack_bytes == raw(role_roots['consumer'] / 'invocation/restored-collection.json', MAX_PACK)
    pack = decode(pack_bytes); pack_sha = sha(pack_bytes)
    exact(pack, ['schema', 'scope', 'entries'])
    assert pack['schema'] == 'quod-managed-collection-v1' and pack['scope'] == scope
    assert len(pack['entries']) == 1
    entries, previous = [], ''
    for entry in pack['entries']:
        exact(entry, ['executionKey', 'receiptSha256', 'receiptBase64'])
        assert digest(entry['executionKey']) and entry['executionKey'] > previous and digest(entry['receiptSha256'])
        body = b64(entry['receiptBase64'], MAX_ITEM); assert sha(body) == entry['receiptSha256']
        previous = entry['executionKey']; entries.append({name: entry[name]
            for name in ['executionKey', 'receiptSha256', 'receiptBase64']})
    reconstructed = {'schema': pack['schema'], 'scope': pack['scope'], 'entries': entries}
    assert (json.dumps(reconstructed, separators=(',', ':'), ensure_ascii=False, allow_nan=False) + '\n').encode() == pack_bytes
    restored_seed = transports['seed'][0]; saved = transports['seed'][1]; restored = transports['consumer'][0]
    service_key = 'quod-managed-collection-v1-' + namespace('seed') + scope + '-' + pack_sha
    assert restored_seed['reason'] == 'miss' and restored_seed['matchedServiceKey'] is None and restored_seed['collectionSha256'] is None and restored_seed['entries'] == 0
    assert restored['reason'] == 'hit' and restored['matchedServiceKey'] == saved['key'] == service_key
    assert restored['collectionSha256'] == saved['collectionSha256'] == pack_sha
    assert restored['entries'] == saved['entries'] == 1
    assert type(saved['cacheId']) is int and saved['cacheId'] > 0
    for role, kind in [('seed', 'published'), ('consumer', 'restored')]:
        retained = events[role]['retainedCollections']; assert set(retained) == {kind}
        descriptor = retained[kind]
        assert descriptor['file'] == str(pathlib.PurePosixPath(events[role]['evidenceDirectory']) / (kind + '-collection.json'))
        assert descriptor['serviceKey'] == service_key and descriptor['sha256'] == pack_sha and descriptor['bytes'] == len(pack_bytes)
        if role == 'seed':
            assert descriptor['cacheId'] == saved['cacheId']
        assert proofs[role]['collectionSha256'] == pack_sha and proofs[role]['scopeSha256'] == scope
        assert proofs[role]['sdkRestores'] == 1 and proofs[role]['sdkSaves'] == (1 if role == 'seed' else 0)
    assert proofs['seed']['cacheId'] == saved['cacheId']
    assert events['control']['retainedCollections'] == {} and proofs['control']['sdkRestores'] == proofs['control']['sdkSaves'] == 0
    assert events['seed']['publication'] == dispatches['seed']['publication'] and events['seed']['publication']['saved'] is True
    assert events['seed']['publication']['cacheId'] == saved['cacheId']
    assert events['consumer']['publication'] == dispatches['consumer']['publication'] and events['consumer']['publication']['saved'] is False
    assert not list((role_roots['control'] / 'invocation').glob('*collection.json'))
    return pack, pack_sha, service_key, transports, {'cacheId': saved['cacheId'], 'packBytes': len(pack_bytes),
        'packSha256': pack_sha, 'matchedServiceKey': service_key, 'scopeSha256': scope}



def native_report_counter(report, project):
    assert report['success'] is True and isinstance(report['testResults'], list)
    result = []
    for file in report['testResults']:
        name = file['name']; assert name.startswith(project.rstrip('/') + '/') and file['status'] == 'passed'
        relative = name[len(project.rstrip('/')) + 1:]
        tests = []
        for row in file['assertionResults']:
            assert row['status'] in {'passed', 'skipped', 'pending', 'todo'} and not row.get('failureMessages')
            tests.append((row['fullName'], row['status']))
        result.append((relative, file['status'], tuple(sorted(tests))))
    return collections.Counter(result)


def v0_receipt(body, execution):
    data = decode(body)
    assert canonical(data).encode() == body
    exact(data, ['schema', 'researchOnly', 'execution', 'expected', 'response', 'resultDigest'])
    assert data['schema'] == V0_RECEIPT and data['researchOnly'] is True and data['execution'] == execution and data['expected'] == V0_EXPECTED
    exact(data['response'], ['encoding', 'bytes', 'sha256'])
    assert data['response']['encoding'] == 'base64'
    response = b64(data['response']['bytes'], 16 * 1024 * 1024)
    assert sha(response) == data['response']['sha256']
    assert data['resultDigest'] == sha((V0_RECEIPT + '\n' + execution['key'] + '\n' + data['response']['bytes']).encode())
    value = decode(response); boundary(value)
    assert len(value['rows']) == 47 and len({row['name'] for row in value['rows']}) == 47
    assert all(row['state'] == 'pass' and row['errors'] == [] for row in value['rows'])
    assert value['files'] == [{'name': V0_EXPECTED['file'], 'state': 'pass', 'errors': []}]
    return response, value


def old_v0(root, event, proof, sources, baseline, baseline_root, preflight, times):
    out = root / 'invocation'; result = read(out / 'result.json')
    assert result['schema'] == 'quod-managed-dispatch-v1' and result['mode'] == event['mode'] == 'v0' and result['optIn'] is True
    assert result['hit'] is event['hit'] is False and result['fallback'] is event['fallback'] is False
    assert result['exitCode'] == event['exitCode'] == 0 and result['managedExecutions'] == event['managedExecutions'] == 1
    assert not result.get('managedError') and result['candidate'] == V0_CANDIDATE
    for name in ['effectsSha256', 'runtimeClassSha256', 'nativeFingerprintSha256', 'executionKey', 'commandSeconds']:
        assert result[name] == event[name]
    assert event['files'] == [{'file': V0_CANDIDATE, 'decision': 'managed'}]
    assert len(result['files']) == 1 and result['files'][0]['file'] == V0_CANDIDATE and result['files'][0]['decision'] == 'managed'
    assert result['args'][0] == 'run' and all(re.fullmatch(r'--(?:reporter=|outputFile\.json=).+', value) for value in result['args'][1:])
    assert result['project'] == preflight['project']
    execution = read(out / 'execution-identity.json'); exact(execution, ['identity', 'key'])
    identity = execution['identity']; exact(identity, ['schema', 'compile', 'preparation', 'runtimePolicy', 'supervisorPolicy', 'host'])
    assert identity['schema'] == V0_EXECUTION and execution['key'] == domain(V0_EXECUTION, identity) == result['executionKey']
    policy, host = identity['runtimePolicy'], identity['host']
    assert policy['workerdSha256'] == RUNTIME_MEMBERS['node_modules/@cloudflare/workerd-linux-64/bin/workerd']['sha256']
    assert policy['boundaryContract'] == CONTRACT and policy['intrinsicPolicy'] == INTRINSICS
    assert policy['services'] == ['main', 'deny'] and policy['globalOutbound'] == 'deny' and policy['bindings'] == []
    assert policy['compatibilityDate'] == '2026-07-01' and policy['compatibilityFlags'] == ['nodejs_als']
    assert policy['moduleName'] == 'worker.mjs' and policy['request'] == {'method': 'GET', 'path': '/run'}
    assert policy['supervisor']['sourceSha256'] == sources['executor.mjs'] and policy['supervisor']['watchdogMs'] == 300000
    assert policy['supervisor']['responseBytesMax'] == 16 * 1024 * 1024
    assert policy['supervisor']['completionBounds'] == {'nativeTurnFencesMax': 512, 'timerCallbackBudget': 4096,
        'timerCallbackBudgetScope': 'callbacks after completion begins; reject when exceeded', 'quietNativeTurns': 2}
    assert identity['supervisorPolicy'] == {'receiptValidatorSha256': sources['receipt.mjs'], 'experimentSha256': sources['dispatcher.mjs'],
        'ordinarySelectionPolicy': 'vitest-5-native-specifications-v1', 'ordinaryAdapterSha256': sources['ordinary-remainder.mjs'], 'expected': V0_EXPECTED}
    assert result['runtimeClassSha256'] == sha(canonical({'runtimePolicy': policy, 'host': host}).encode())
    assert host['platform'] == 'linux' and host['arch'] == 'x64' and host['nodeVersion'] == 'v24.18.0' and host['ImageOS'] and host['ImageVersion']
    phases = result['phases']; assert [phase['name'] for phase in phases] == ['preflight', 'managed']
    assert sorted(path.relative_to(out).as_posix() for path in out.rglob('worker.mjs')) == ['managed/worker.mjs', 'preflight/worker.mjs']
    for name, phase in [('preflight', phases[0]), ('managed', phases[1])]:
        built = read(out / ('preflight-build.json' if name == 'preflight' else 'test-build.json'))
        assert built['identity']['entry'] == '/quod-adapter/' + ('preflight.mjs' if name == 'preflight' else 'runner.mjs')
        assert built['identity']['compilerSha256'] == COMPILER
        assert built['sha256'] == built['identity']['bundleSha256'] and built['identitySha256'] == sha(canonical(built['identity']).encode())
        assert built['identity']['globs'] == built['globs'] and built['identity']['adapters'] == built['adapters']
        assert built['closure']['finalBundleParsed'] is True and built['closure']['remainingDynamicImports'] == built['closure']['remainingDynamicRequires'] == 0
        assert all(value == 'node:async_hooks' for value in built['closure']['externalImports'])
        expected_options = {'epoch': 1700000000000, 'seed': 42, 'env': {'NODE_ENV': 'test'}} if name == 'preflight' else built['options']
        assert built['identity']['configurationSha256'] == sha(canonical(expected_options).encode())
        response = worker_evidence(out / name, built['identity'], policy, host, phase)
        data = decode(response); boundary(data, preflight=name == 'preflight')
        if name == 'preflight':
            assert len(data['checks']) == len({row['name'] for row in data['checks']}) == 31 and all(row['pass'] is True for row in data['checks'])
            fingerprint = sha(canonical({'preflightIdentity': built['identitySha256'], 'inventory': data['boundary']['inventory'],
                'symbols': data['boundary']['symbolInventory']}).encode())
            assert fingerprint == policy['nativeFingerprintSha256'] == result['nativeFingerprintSha256']
        else:
            assert built['identity'] == identity['compile'] and built['preparation'] == identity['preparation']
            assert built['options']['epoch'] == 1700000000000 and built['options']['seed'] == 42 and built['options']['env'] == {'NODE_ENV': 'test'}
            assert built['options']['file'] == '/project/' + V0_CANDIDATE
            assert identity['preparation']['preparerSha256'] == sources['preparation.mjs']
            assert {row['name']: row['sha256'] for row in identity['preparation']['sources']} == PREPARATION_SOURCE_HASHES
            assert identity['preparation']['configurationSha256'] == sha(canonical(built['options']['vitest']).encode())
            assert any(row['source'] == '/project/' + V0_CANDIDATE and row['sourceSha256'] == V0_SOURCE for row in built['identity']['sources'])
            receipt_bytes = raw(out / 'receipt.json', 24 * 1024 * 1024)
            selected, selected_data = v0_receipt(receipt_bytes, execution)
            assert selected == response
            assert read(out / 'selected-response.json', 16 * 1024 * 1024) == selected_data
    effect = {'ok': selected_data['ok'], 'rows': selected_data['rows'], 'files': selected_data['files'],
        'logs': selected_data['boundary']['logs'], 'virtualTime': selected_data['boundary']['virtualTime']}
    assert result['effects'] == effect and result['effectsSha256'] == sha(canonical(effect).encode())
    assert len(result['receipts']) == 1
    receipt_meta = result['receipts'][0]
    assert receipt_meta['key'] == execution['key'] and receipt_meta['receiptSha256'] == sha(receipt_bytes)
    assert receipt_meta['bytes'] == len(receipt_bytes) and receipt_meta['responseSha256'] == sha(selected) and receipt_meta['expected'] == V0_EXPECTED
    assert result['files'][0]['receipt'] == receipt_meta and result['files'][0]['executionKey'] == execution['key']
    preparation = read(out / 'preparation.json')
    assert preparation['profile'] == 'convex-helpers-original-twelve' and len(preparation['selected']) == 120
    assert {row['name']: row['sha256'] for row in preparation['reviewedSources']} == PREPARATION_SOURCE_HASHES
    assert {row['name']: row['sha256'] for row in preparation['reviewedDataSources']} == PROJECT_DATA_SOURCE_HASHES
    assert collections.Counter((row['file'], row['project']) for row in preparation['selected']) == collections.Counter(
        (row['file'], row['project']) for row in baseline['originalSelected'])
    assert all(row['absolute'] == result['project'] + '/' + row['file'] for row in preparation['selected'])
    counts = lambda rows: collections.Counter((row['absolute'], row['project']) for row in rows)
    selected_instances = counts(preparation['selected']); assert len(selected_instances) == 120
    candidates = [row for row in preparation['selected'] if row['file'] == V0_CANDIDATE]
    assert len(candidates) == 1 and candidates[0]['project'] == 'convex-helpers'
    candidate = {key: candidates[0][key] for key in ['absolute', 'project']}
    request = read(out / 'ordinary-remainder-request.json'); selection = read(out / 'ordinary-remainder-selection.json')
    assert request['schema'] == 'quod-ordinary-remainder-request-v1' and request['project'] == result['project']
    assert request['selected'] == preparation['selected'] and request['excluded'] == candidate and request['filters'] == []
    assert len(request['reporters']) == 2 and set(request['reporters']) == {'default', 'json'}
    assert selection['schema'] == 'quod-ordinary-remainder-selection-v1' and selection['matched'] is True and selection['runStarted'] is True and selection['completed'] is True
    assert selection['exitCode'] == selection['failedModules'] == selection['unhandledErrors'] == 0 and 'error' not in selection and 'closeError' not in selection
    assert counts(selection['selected']) == selected_instances and counts(selection['excluded']) == counts([candidate])
    assert counts(selection['remainder']) == counts(selection['executed']) == selected_instances - counts([candidate])
    assert len(selection['executed']) == 119 and result['ordinarySelection'] == selection
    assert result['ordinaryRepeatedCandidate'] is False and len(result['ordinaryRuns']) == 1 and result['ordinaryRuns'][0]['exitCode'] == result['ordinary']['exitCode'] == 0
    invocation = read(out / 'ordinary-remainder-invocation.json')
    assert invocation['entry'].endswith('/ordinary-remainder.mjs') and invocation['cwd'] == result['project']
    assert invocation['args'] == [request['evidenceFile'].replace('ordinary-remainder-selection.json', 'ordinary-remainder-request.json')]
    merged_bytes = raw(root / 'native-report.json'); merged = decode(merged_bytes); ordinary = read(out / 'ordinary-report.json')
    assert merged_bytes == raw(root / 'original-native-report.json')
    assert result['reportPath'].endswith('/property47-packaged-proof-v0/original-native-report.json')
    assert len(ordinary['testResults']) == 119 and collections.Counter(file['name'] for file in ordinary['testResults']) == collections.Counter(row['absolute'] for row in selection['executed'])
    subset = [file for file in merged['testResults'] if not file['name'].endswith('/' + V0_CANDIDATE)]
    assert collections.Counter(canonical(file) for file in subset) == collections.Counter(canonical(file) for file in ordinary['testResults'])
    actual = native_report_counter(merged, result['project'])
    original = native_report_counter(read(baseline_root / 'native-report.json'), baseline['selected'][0]['absolute'].rpartition('/' + baseline['selected'][0]['file'])[0])
    assert actual == original and len(merged['testResults']) == 120
    assert sum(len(file['assertionResults']) for file in merged['testResults']) == merged['numTotalTests'] == 4131
    assert merged['numTotalTestSuites'] == 120 and merged['numFailedTests'] == merged['numFailedTestSuites'] == 0
    v0_status = {**STATUS, 'pending': 'skip'}
    normalized = collections.Counter((file['name'], row['fullName'], v0_status[row['status']])
        for file in merged['testResults'] for row in file['assertionResults'])
    assert normalized == collections.Counter((row['file'], row['name'], row['state']) for row in result['rows'])
    candidate_files = [file for file in merged['testResults'] if file['name'].endswith('/' + V0_CANDIDATE)]
    assert len(candidate_files) == 1
    assert collections.Counter((row['fullName'], STATUS[row['status']]) for row in candidate_files[0]['assertionResults']) == collections.Counter((row['name'], row['state']) for row in selected_data['rows'])
    assert proof['ordinarySemanticSha256'] == NATIVE_C11 and proof['managedTests'] == 47 and proof['nativeSpecsExecuted'] == 119
    records = read(out / 'cache-transport.json'); assert records['schema'] == 'quod-managed-action-cache-v1'
    assert len(records['events']) == 2
    restored, saved = records['events']; prefix = 'quod-managed-v1-' + namespace('v0') + execution['key'] + '-'
    assert restored['operation'] == 'restore' and restored['executionKey'] == execution['key'] and restored['prefix'] == prefix
    assert restored['matchedServiceKey'] is None and restored['trustedReceiptSha256'] is None
    assert saved['operation'] == 'save' and saved['executionKey'] == execution['key'] and saved['receiptSha256'] == sha(receipt_bytes)
    assert saved['key'] == prefix + sha(receipt_bytes) and type(saved['cacheId']) is int and saved['cacheId'] > 0
    assert 0 <= result['commandSeconds'] <= times['completeJobSeconds'] + 2
    return {'managedTests': 47, 'nativeInstances': 119, 'logicalStates': 4131, 'executionKey': execution['key'],
        'receiptSha256': sha(receipt_bytes), 'responseSha256': sha(selected), 'positiveCacheId': saved['cacheId'],
        'fullNativeMultisetEqualPinnedBaseline': True, 'fullJobHit': False}


def audit(index):
    global RUNTIME_MEMBERS
    times = metadata(index)
    roots, provenance = {}, {}
    for role in [*ROLES, 'baseline']:
        roots[role], provenance[role] = archive(role, index['evidence'][role], BASELINE if role == 'baseline' else ARTIFACTS[role])
    baseline_root = locate(roots['baseline'], ['managed-matrix', 'native-selection.json']).parent
    baseline_bytes = raw(baseline_root / 'native-selection.json', 4 * 1024 * 1024)
    assert len(baseline_bytes) == BASELINE_SELECTION_BYTES and sha(baseline_bytes) == BASELINE_SELECTION_SHA256
    baseline = native(baseline_root, baseline=True)
    assert len(baseline['executed']) == 120 and sum(state_rows(baseline['executed']).values()) == 4131
    assert collections.Counter(row['state'] for row in baseline['executed']) == {'pass': 119, 'skip': 1}
    runtime_bodies, sources, candidate_ids, helper, workflow = source_inputs()
    manifest, runtime_bytes, runtime_inventory, ncc, action_sources = action_inputs(runtime_bodies)
    RUNTIME_MEMBERS = runtime_inventory['members']
    role_roots, preflights, proofs, events = {}, {}, {}, {}
    for role in ROLES:
        role_root = locate(roots[role], ['proof.json']).parent; role_roots[role] = role_root
        preflights[role], proofs[role], events[role] = common_role(role, role_root, sources, helper, manifest, runtime_bytes, ncc)
    assert len({row['project'] for row in preflights.values()}) == 1, 'project root geometry differs across fresh packaged jobs'
    dispatches, controls = {}, {}
    for role in ['seed', 'control', 'consumer']:
        d, n = matrix_role(role, role_roots[role], events[role], proofs[role], sources, baseline, times[role], ncc)
        dispatches[role], controls[role] = d, n
        assert {row['instanceId'] for row in d['instances'] if row['candidate']} == candidate_ids
        for name in ['selectedInstances', 'candidateInstances', 'replayedInstances', 'skippedCompilations', 'managedExecutions', 'nativeSpecsExecuted', 'logicalStates']:
            assert proofs[role][name] == d[name]
    for role in ['seed', 'control']:
        d, n = dispatches[role], controls[role]
        assert d['managedExecutions'] == 1 and d['replayedInstances'] == d['skippedCompilations'] == d['inputCandidates'] == 0
        assert d['nativeSpecsExecuted'] == 120 and state_rows(n['executed']) == state_rows(baseline['executed'])
        assert file_rows(n['executed']) == file_rows(baseline['executed'])
    assert dispatches['seed']['recordedReceipts'] == 1 and dispatches['control']['recordedReceipts'] == 0
    current = dispatches['consumer']
    assert current['managedExecutions'] == current['nativeConformanceInstances'] == current['recordedReceipts'] == current['refreshedCandidates'] == 0
    assert current['replayedInstances'] == current['skippedCompilations'] == current['inputCandidates'] == 1
    assert current['nativeSpecsExecuted'] == 119 and current['publication']['saved'] is False
    for role in ['control', 'consumer']:
        assert not list((role_roots[role] / 'invocation/receipts').glob('*.json'))
    assert not any(path.name in {'test-build.json', 'compile-certificate.json', 'recorded-candidate.json'}
        for path in (role_roots['consumer'] / 'invocation/instances').rglob('*'))
    assert len({event['logicalSha256'] for role, event in events.items() if role != 'v0'}) == 1
    assert len({proofs[role]['fileStatesSha256'] for role in ['seed', 'control', 'consumer']}) == 1
    pack, pack_sha, service_key, transport, cache = collection_chain(
        {role: role_roots[role] for role in ['seed', 'control', 'consumer']},
        {role: events[role] for role in ['seed', 'control', 'consumer']}, dispatches, proofs)
    admission = matrix_admission(role_roots, dispatches, controls, sources, candidate_ids, baseline,
        pack, pack_sha, service_key, transport, {'project': preflights['consumer']['project'],
            'adapter': str(pathlib.PurePosixPath(events['consumer']['evidenceDirectory']).parent / 'managed-runtime')})
    old = old_v0(role_roots['v0'], events['v0'], proofs['v0'], sources, baseline, baseline_root,
        preflights['v0'], times['v0'])
    return {'schema': 'quod-property47-packaged-action-audit-v1', 'passed': True, 'issues': [],
        'run': RUN, 'workflowSource': WORKFLOW_SOURCE, 'actionPin': ACTION_B, 'runtimeSourcePin': RUNTIME_Q,
        'baselineSourcePin': BASELINE_SOURCE, 'provenance': provenance,
        'runtimeArchive': {'bytes': len(runtime_bytes), 'sha256': sha(runtime_bytes), **runtime_inventory},
        'nccExecutableMembers': ncc, 'runtimeSourceSha256': sources, 'actionSourceSha256': action_sources,
        'proofReaderSha256': sha(helper), 'workflowSha256': sha(workflow), 'timing': times,
        'sdk': cache, 'admission': admission, 'oldV0': old,
        'release': {'names': RELEASE_NAMES, 'sources': RELEASE_SOURCES, 'passedInEachFreshRole': 20},
        'outcomes': {'selectedInstances': 120, 'logicalStates': 4131, 'consumerReplayed': 1,
            'consumerCandidateCompiles': 0, 'consumerCandidateWorkers': 0, 'consumerNativeInstances': 119,
            'fullProjectFilePoolNameStateAndApiFileStatesEqualBothControlsAndPinnedBaseline': True, 'fullJobHit': False},
        'limits': ['Four fresh full-job diagnostics; not an original48 cold-window or controlled speedup/50-percent result.',
            'Complete job costs come from pinned GitHub API timestamps; component timers are diagnostic only.',
            'Opaque proof issuance and auth cleanup are enforced by reviewed B/Q/W sources; retained bytes bind the actual reported execution.',
            'The initial Node/verifier/finite workflow parent is trusted; no arbitrary-host or active-writer atomic snapshot attestation.',
            'One preflight compilation/worker and 119 ordinary native instances remain on the matrix consumer.',
            'Logical project/file/pool/name/state and API file states are exact; native reporter timing metadata is not reconstructed.',
            'Runtime tar and all executable ncc chunks are read as data only; no artifact code is imported or executed.']}



def main():
    global RUNTIME_REPOSITORY, ACTION_REPOSITORY, WORKFLOW_REPOSITORY, PROJECT_REPOSITORY
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--runtime-repository', required=True, type=pathlib.Path)
    parser.add_argument('--action-repository', required=True, type=pathlib.Path)
    parser.add_argument('--workflow-repository', required=True, type=pathlib.Path)
    parser.add_argument('--project-repository', required=True, type=pathlib.Path)
    parser.add_argument('index', type=pathlib.Path)
    args = parser.parse_args()
    try:
        checked_pins()  # All future authority/bounds/review checks precede opening index.
        RUNTIME_REPOSITORY = args.runtime_repository.resolve()
        ACTION_REPOSITORY = args.action_repository.resolve()
        WORKFLOW_REPOSITORY = args.workflow_repository.resolve()
        PROJECT_REPOSITORY = args.project_repository.resolve()
        index = read(args.index, 1024 * 1024); parent = args.index.resolve().parent
        for name in ['runJson', 'jobsJson', 'artifactsJson']:
            index[name] = str(parent / index[name])
        for descriptor in index['evidence'].values():
            exact(descriptor, ['root', 'archive'])
            for name in ['root', 'archive']:
                descriptor[name] = str(parent / descriptor[name])
        result = audit(index)
    except Exception as error:
        result = {'schema': 'quod-property47-packaged-action-audit-v1', 'passed': False,
            'issues': [type(error).__name__ + ': ' + str(error)]}
    print(json.dumps(result, indent=2, ensure_ascii=False, allow_nan=False))
    return 0 if result['passed'] else 1


if __name__ == '__main__':
    sys.exit(main())

#!/usr/bin/env python3
"""Independent data-only reader for completed closed-batch SDK gate42423962148.

Usage: --repository TRUSTED_SOURCE_CHECKOUT EVIDENCE_INDEX.json
Exact reviewed run/source/job/archive authority is pinned below and checked
before opening the index. The Action transport and ff baseline pins are fixed.
Never imports or executes artifact code.
"""
import argparse
import base64
import collections
import datetime
import hashlib
import json
import math
import os
import pathlib
import re
import stat
import subprocess
import sys
import unicodedata
import zipfile
import zlib

# Immutable authority for this completed, independently reviewed diagnostic.
RUN = 42423962148
SOURCE = '85c96d6c62c9252f3aa464a851f514348353d010'
ACTION = '02f65151206f3a12d6f17e3b3d1831b704f28a0b'
ARTIFACTS = {'fixtures': (13595135981, 5787213, '20a4d1ef52d999797191fea3549939976ad5b93e91c58f75980302ab51ce1ad6', 128558145208, 'matrix-input-dispatcher-fixtures'), 'seed': (13594821748, 52576699, '277c258b123e785352accf1680081500875adb8ce997722ecaf16e4a3b75f70e', 128558617805, 'matrix-input-sdk-seed'), 'control': (13595575076, 39171055, 'bf97c53efa38de5d68344279d43aaace46058c77a6fdd8841d2311dce8c75378', 128558617786, 'matrix-input-sdk-control'), 'consumer': (13594982878, 13740696, 'bb5d54daaa910c36585ec1a482c07e59a42e13e71a713260500023738e71de72', 128560665459, 'matrix-input-sdk-consumer')}
JOB_TIMESTAMPS = {'fixtures': ('2026-11-07T23:09:49Z', '2026-11-07T23:11:42Z'), 'seed': ('2026-11-07T23:11:44Z', '2026-11-07T23:19:13Z'), 'control': ('2026-11-07T23:11:44Z', '2026-11-07T23:20:28Z'), 'consumer': ('2026-11-07T23:20:29Z', '2026-11-07T23:21:47Z')}
JOB_IDS = {'fixtures': 128558145208, 'seed': 128558617805, 'control': 128558617786, 'consumer': 128560665459}
ACTION_SOURCES = {'collectionSourceSha256': '1b4aa76730aee3fd2984da6ed79a15eabe40c21d9977077ce2ff64ab3d5473dc',
 'authSourceSha256': 'dab0b9d4a7d8b54d3cde5791d0d0cbc2694bf2b7a00598d1f49a67ffe253de22'}
BASELINE_SOURCE = 'ff279ea5b59b359d26985f08f22e0749f9eccbde'
BASELINE_RUN = 42406036816
BASELINE = (13587034973, 36186785,
            'f8e20f3a68cc3fbec7ce01a5ea90e8b50a1d558cbf08146a7fb049e242adc282',
            128503824968, 'benchmark-managed-matrix')
COMPILER = 'a87f1f8fafea2e0f1b4a1e299199f5581172a2fe90f59882d3c64eb50f68acaf'
BATCH_CORE = 'f01cd9a55f6eceb913c5b402481fd6b6dd7266191d8182f97ca60f158765c703'
BATCH_POLICY = 'closed-adjacent-full-fences-before-any-candidate-worker-v1-diagnostic'
TOOL_FILES = '7b7ae881ef3186e4b564c26ffa6e8d40b3d3875a15e41b10d93a6bb24cb3a354'
CONTRACT = 'quod-managed-v0-protectors-frozen-slots-investigation'
INTRINSICS = 'quod-managed-frozen-array-promise-slots-v1-probe'
EXECUTION = 'quod-managed-matrix-execution-v2-research'
CONFORMANCE = 'quod-managed-matrix-conformance-v2-research'
RECEIPT = 'quod-managed-matrix-receipt-v2-research'
ENVELOPE = 'quod-managed-matrix-input-candidate-v1-research'
CERTIFICATE = 'quod-managed-resolved-compile-certificate-v1-diagnostic'
MAX_ITEM = 2 * 1024 * 1024
MAX_PACK = 32 * 1024 * 1024
MAX_ARCHIVE = 100_000_000
MAX_EXPANDED = 500_000_000
MAX_FILE = 32 * 1024 * 1024
MAX_PATHS = 20000
CERTIFICATE_LIMITS = {'paths': 100000, 'resolverBytes': 16 * 1024 * 1024, 'resolverTimeoutMs': 30000,
    'toolBytes': 128 * 1024 * 1024, 'sourceBytes': 256 * 1024 * 1024,
    'fileBytes': 16 * 1024 * 1024, 'certificateBytes': 16 * 1024 * 1024,
    'certificateSetBytes': 128 * 1024 * 1024, 'certificates': 512}
ROOT = None
PASSING = {'pass', 'skip', 'todo'}
STATUS = {'passed': 'pass', 'failed': 'fail', 'skipped': 'skip', 'todo': 'todo'}
PREFLIGHT_DENIALS = ['global fetch', 'global crypto', 'global WeakRef', 'global FinalizationRegistry',
    'Event.timeStamp', 'global WebAssembly', 'AbortSignal.timeout', 'eval', 'Function constructor',
    'AsyncFunction constructor', 'GeneratorFunction constructor', 'AsyncGeneratorFunction constructor',
    'global EventTarget.addEventListener', 'global EventTarget.removeEventListener', 'global EventTarget.dispatchEvent']
SOURCE_FILES = ['compiler.mjs', 'preparation.mjs', 'executor.mjs', 'prelude.mjs', 'runner.mjs',
    'matrix-native.mjs', 'matrix-input-dispatcher.mjs', 'matrix-candidates.mjs', 'matrix-receipt.mjs',
    'matrix-input-sdk-probe.mjs', 'matrix-candidate-envelope.mjs', 'resolved-compile-certificate.mjs',
    'matrix-input-dispatcher-fixtures.mjs', 'matrix-input-proof-fixtures.mjs',
    'matrix-receipt-fixtures.mjs', 'matrix-probe.mjs', 'package.json', 'cache-session-action/index.cjs', 'cache-session-action/action.yml',
    'batch-compile-certificate-probe.mjs', 'matrix-batch-dispatcher-fixtures.mjs', '.github/workflows/gate.yml']


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


def checked_pins():
    assert isinstance(RUN, int) and RUN > 0, 'PENDING reviewed remote run pin; no artifact may choose it'
    assert isinstance(SOURCE, str) and re.fullmatch('[0-9a-f]{40}', SOURCE)
    assert isinstance(ACTION, str) and re.fullmatch('[0-9a-f]{40}', ACTION)
    assert isinstance(ACTION_SOURCES, dict) and set(ACTION_SOURCES) == {'collectionSourceSha256', 'authSourceSha256'}
    assert all(digest(value) for value in ACTION_SOURCES.values())
    for role, value in ARTIFACTS.items():
        assert isinstance(value, tuple) and len(value) == 5, 'PENDING exact artifact/job pins: ' + role
        assert isinstance(value[0], int) and 0 < value[1] <= MAX_ARCHIVE and digest(value[2]) and isinstance(value[3], int)
        assert isinstance(JOB_TIMESTAMPS[role], tuple) and len(JOB_TIMESTAMPS[role]) == 2
        assert type(JOB_IDS[role]) is int and JOB_IDS[role] > 0 and value[3] == JOB_IDS[role]


def git_source(name):
    # Only reviewed local repository objects are read. Artifact paths never
    # become Python imports, shell commands, Git source names or executables.
    assert name in SOURCE_FILES
    env = {**os.environ, 'GIT_NO_LAZY_FETCH': '1', 'GIT_TERMINAL_PROMPT': '0'}
    selected = name if name == '.github/workflows/gate.yml' else 'scripts/bench-vitest/managed/' + name
    value = subprocess.check_output(['git', 'show', SOURCE + ':' + selected], cwd=ROOT, env=env)
    assert 0 < len(value) <= 2 * 1024 * 1024
    return value


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


def checks(value, count, key='passed'):
    assert value['passed'] is True and len(value['checks']) == count
    assert all(row[key] is True for row in value['checks'])
    assert len({row['name'] for row in value['checks']}) == count


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


def audit_legacy(root, result, matrix, native_control, sources, candidate_ids):
    assert result['actualInstances'] == 108 and result['actualStates'] == 3947
    actual = read(matrix / 'result.json')
    rows = {row['instanceId']: row for row in actual['instances'] if row['decision'] == 'managed'}
    assert set(rows) == candidate_ids
    modules = instance_map(native_control['executed'])
    total, outcomes = 0, collections.Counter()
    for item in result['receipts']:
        iid = item['instanceId']; original = modules[iid]
        body = raw(root / 'receipts' / (iid + '.json'), MAX_ITEM)
        assert len(body) == item['bytes'] and sha(body) == item['receiptSha256']
        value = decode(body); assert canonical(value).encode() == body and value['schema'] == RECEIPT and value['researchOnly'] is True
        execution = value['execution']; identity = execution['identity']
        assert execution['key'] == domain(EXECUTION, identity) == item['key']
        directory = matrix / rows[iid]['evidenceDirectory']; built = read(directory / 'test-build.json')
        assert identity['compile'] == built['identity'] and identity['preparation'] == read(directory / 'preparation.json')
        assert identity['instance'] == {key: rows[iid][key] for key in ['instanceId', 'project', 'file', 'pool']}
        assert identity['supervisorPolicy']['receiptValidatorSha256'] == sources['matrix-receipt.mjs']
        assert identity['supervisorPolicy']['experimentSha256'] == sources['matrix-receipt-fixtures.mjs']
        assert identity['nativePolicy']['adapterSha256'] == sources['matrix-probe.mjs']
        conformance = value['conformance']
        assert conformance['executionKey'] == execution['key'] and conformance['instance'] == identity['instance']
        assert conformance['nativeSelectionSha256'] == sha(raw(matrix / 'native-selection.json'))
        assert conformance['nativeReportSha256'] == sha(raw(matrix / 'native-report.json'))
        assert conformance['nativePolicySha256'] == sha(canonical(identity['nativePolicy']).encode())
        assert value['conformanceSha256'] == domain(CONFORMANCE, conformance)
        response = b64(value['response']['bytes'], 16 * 1024 * 1024)
        assert response == raw(directory / 'managed/response.json') and sha(response) == value['response']['sha256'] == item['responseSha256']
        assert value['resultDigest'] == sha((RECEIPT + '\n' + execution['key'] + '\n' + value['conformanceSha256'] + '\n' + value['response']['bytes']).encode())
        data = decode(response); boundary(data)
        expected = collections.Counter((test['name'], test['state']) for test in original['rows'])
        assert collections.Counter((test['name'], test['state']) for test in data['rows']) == expected
        assert collections.Counter((test['name'], test['state']) for test in conformance['rows']) == expected
        assert data['files'][0]['name'] == conformance['file']['name'] == original['relativeModuleId']
        assert data['files'][0]['state'] == conformance['file']['state'] == item['fileState'] == original['state']
        assert item['count'] == len(data['rows'])
        total += len(body); outcomes.update(test['state'] for test in data['rows'])
    assert outcomes == {'pass': 3946, 'skip': 1} and total == result['totalReceiptBytes']
    assert 512 + sum(((item['bytes'] + 2) // 3) * 4 + 512 for item in result['receipts']) == result['transportBytesUpperBound'] <= MAX_PACK


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
        assert dispatch['managedExecutions'] == 0 and dispatch['skippedCompilations'] == dispatch['replayedInstances'] == dispatch['inputCandidates'] == 108
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
    assert read(file) == batch and batch['attempted'] is True and batch['completed'] is True and batch['requests'] == 108
    assert type(batch['seconds']) in (int, float) and math.isfinite(batch['seconds']) and 0 <= batch['seconds'] <= dispatch['seconds'] + 0.01
    diagnostics = batch['diagnostics']
    exact(diagnostics, ['requests', 'fullFences', 'trace', 'totalBundles', 'totalCertificates', 'totalIdentities', 'publicOperationsAtReservation', 'authority'])
    assert diagnostics['requests'] == 108 and diagnostics['fullFences'] == 218 and diagnostics['trace'] == batch_trace(108)
    assert diagnostics['publicOperationsAtReservation'] == 0
    assert diagnostics['authority'] == 'actual compilation and capture only; no response or receipt admission'
    totals = {'totalBundles': 0, 'totalCertificates': 0, 'totalIdentities': 0}
    candidates = [row for row in dispatch['instances'] if row['candidate']]
    assert len(candidates) == 108 and len({row['instanceId'] for row in candidates}) == 108
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
    return {'requests': 108, 'fullFences': 218, 'seconds': batch['seconds'], **totals}


def audit(index):
    exact(index, ['schema', 'runJson', 'jobsJson', 'artifactsJson', 'evidence'])
    assert index['schema'] == 'quod-managed-matrix-input-sdk-audit-inputs-v1'
    assert set(index['evidence']) == {'fixtures', 'seed', 'control', 'consumer', 'baseline'}
    run, jobs, metadata = read(index['runJson']), read(index['jobsJson']), read(index['artifactsJson'])
    assert run['id'] == RUN and run['head_sha'] == SOURCE and run['event'] == 'workflow_dispatch'
    assert run['status'] == 'completed' and run['conclusion'] == 'success' and run['run_attempt'] == 1
    assert len({job['id'] for job in jobs['jobs']}) == len(jobs['jobs'])
    assert len({item['id'] for item in metadata['artifacts']}) == len(metadata['artifacts'])
    by_job = {job['id']: job for job in jobs['jobs']}; by_artifact = {item['id']: item for item in metadata['artifacts']}
    roots, provenance, times = {}, {}, {}
    for role in ['fixtures', 'seed', 'control', 'consumer', 'baseline']:
        expected = BASELINE if role == 'baseline' else ARTIFACTS[role]
        roots[role], provenance[role] = archive(role, index['evidence'][role], expected)
        if role == 'baseline':
            continue
        artifact = by_artifact[expected[0]]; job = by_job[expected[3]]
        assert artifact['workflow_run']['id'] == RUN and artifact['workflow_run']['head_sha'] == SOURCE
        assert artifact['digest'] == 'sha256:' + expected[2] and artifact['size_in_bytes'] == expected[1]
        assert artifact['name'] == 'managed-matrix-input-' + ('dispatcher-fixtures' if role == 'fixtures' else 'sdk-' + role)
        assert job['name'] == expected[4] and job['status'] == 'completed' and job['conclusion'] == 'success'
        assert (job['started_at'], job['completed_at']) == JOB_TIMESTAMPS[role]
        start, end = (datetime.datetime.fromisoformat(job[key].replace('Z', '+00:00')) for key in ['started_at', 'completed_at'])
        seconds = (end - start).total_seconds(); assert math.isfinite(seconds) and seconds > 0
        times[role] = {'completeJobSeconds': seconds, 'modeledRoundedMinutes': math.ceil(seconds / 60),
            'startedAt': job['started_at'], 'completedAt': job['completed_at']}
        provenance[role].update(jobId=expected[3], artifactName=artifact['name'])
    assert len({by_job[ARTIFACTS[role][3]]['runner_id'] for role in ['seed', 'control', 'consumer']}) == 3
    sources = {name: sha(git_source(name)) for name in SOURCE_FILES}
    assert sources['compiler.mjs'] == COMPILER
    assert sources['resolved-compile-certificate.mjs'] == BATCH_CORE
    roster_text = git_source('matrix-candidates.mjs').decode()
    roster = decode(roster_text.split('Object.freeze(', 1)[1].split('.map(row => Object.freeze(row))', 1)[0])
    candidate_ids = set(instance_map(roster)); assert len(candidate_ids) == 108
    baseline_root = locate(roots['baseline'], ['managed-matrix', 'native-selection.json']).parent
    baseline = native(baseline_root, baseline=True)
    original_states, original_files = state_rows(baseline['executed']), file_rows(baseline['executed'])
    assert len(baseline['executed']) == 120 and sum(original_states.values()) == 4131
    assert collections.Counter(module['state'] for module in baseline['executed']) == {'pass': 119, 'skip': 1}
    fixture_root = locate(roots['fixtures'], ['matrix-input-dispatcher-fixtures', 'result.json']).parent
    fixture = read(fixture_root / 'result.json'); checks(fixture, 142)
    assert fixture['originalSourcesRestored'] is True and fixture['caseCount'] == 13 and fixture['checkCount'] == 142
    proof = read(fixture_root / 'input-proof-controls.json'); checks(proof, 18, 'pass')
    assert proof == fixture['inputEnvelopeControls'] and proof['actualSdkAuthenticationClaimed'] is False
    legacy_root = locate(roots['fixtures'], ['input-legacy-ledger', 'result.json']).parent
    legacy = read(legacy_root / 'result.json'); checks(legacy, 152)
    assert all(legacy[key] is False for key in ['productionEnabled', 'cachePublished', 'actualSdkAuthenticationClaimed'])
    assert len(legacy['receipts']) == 108 and set(instance_map(legacy['receipts'])) == candidate_ids
    previous = read(locate(roots['fixtures'], ['input-ledger-independent-audit.json']))
    assert previous['passed'] is True and previous['issues'] == [] and previous['sourcePin'] == BASELINE_SOURCE
    audit_legacy(legacy_root, legacy, baseline_root, baseline, sources, candidate_ids)
    audit_fixtures(fixture_root, fixture)
    sdk, dispatches, controls, role_roots, by_role, batches = {}, {}, {}, {}, {}, {}
    for role in ['seed', 'control', 'consumer']:
        role_root = locate(roots[role], ['sdk-result.json']).parent; role_roots[role] = role_root
        s = sdk[role] = read(role_root / 'sdk-result.json'); d = dispatches[role] = s['dispatch']; dispatch_root = role_root / 'dispatch'
        assert s['schema'] == 'quod-managed-matrix-input-sdk-v1-research' and s['role'] == role and s['passed'] is True
        assert s['productionEnabled'] is False and s['sourcePin'] == SOURCE and s['actionPin'] == ACTION
        assert s['scope']['schema'] == 'quod-managed-matrix-input-sdk-scope-v1'
        assert s['scope']['sourcePin'] == SOURCE and s['scope']['actionPin'] == ACTION
        assert all(s['scope'][key] == value for key, value in ACTION_SOURCES.items())
        assert s['scope']['nodeVersion'] == 'v24.18.0' and s['scope']['cacheSdkVersion'] == '4.1.0'
        assert s['scopeSha256'] == sha(canonical(s['scope']).encode())
        assert s['namespace'] == 'matrix-input-gate-' + str(RUN) + '-'
        assert s['servicePrefix'] == 'quod-managed-collection-v1-' + s['namespace'] + s['scopeSha256'] + '-'
        assert s['privateSessionRemovedBeforeExecution'] is True and s['privateSessionRemoved'] is True and s['authAbsentOutsideSdk'] is True
        assert not any('session' in p.name.lower() or 'cache-auth' in p.name.lower() for p in role_root.rglob('*') if p.is_file())
        assert read(dispatch_root / 'result.json') == d
        assert d['schema'] == 'quod-managed-matrix-input-dispatch-v1-research' and d['productionEnabled'] is False
        assert d['passed'] is True and d['exitCode'] == 0 and d['selectedInstances'] == 120 and d['candidateInstances'] == 108
        assert d['ordinaryInstances'] == 12 and d['logicalStates'] == 4131 and d['fullJobHit'] is False
        assert d['stickyManagedFailure'] is False and d['observedNativeFailure'] is False
        assert d['selectedScope'] == {'kind': 'complete-original-default-selection', 'files': None}
        by_role[role] = instance_map(d['instances'])
        assert {row['instanceId'] for row in d['instances'] if row['candidate']} == candidate_ids
        n = controls[role] = native(dispatch_root / 'native')
        assert set(instance_map(n['originalSelected'])) == set(instance_map(baseline['originalSelected']))
        logical(dispatch_root, d, original_states, original_files)
        assert s['logicalSha256'] == d['logicalSha256']
        preflight = read(dispatch_root / 'preflight/response.json'); boundary(preflight, preflight=True)
        assert len(preflight['checks']) == 31 and len({row['name'] for row in preflight['checks']}) == 31
        assert all(row['pass'] is True for row in preflight['checks'])
        build = read(dispatch_root / 'preflight-build.json')
        fingerprint = sha(canonical({'preflightIdentity': build['identitySha256'], 'inventory': preflight['boundary']['inventory'],
            'symbols': preflight['boundary']['symbolInventory']}).encode())
        assert fingerprint == d['nativeFingerprintSha256']
        assert sha(canonical(build['identity']).encode()) == build['identitySha256']
        assert build['identity']['compilerSha256'] == COMPILER
        assert sha(raw(dispatch_root / 'preflight/worker.mjs')) == build['identity']['bundleSha256']
        assert [phase['name'] for phase in d['phases']].count('preflight') == 1
        assert len(d['phases']) == (1 if role == 'consumer' else 109)
        preflight_phase = next(phase for phase in d['phases'] if phase['name'] == 'preflight')
        preflight_invocation = read(dispatch_root / 'preflight/invocation.json')
        worker_evidence(dispatch_root / 'preflight', build['identity'], preflight_invocation['runtimePolicy'], d['host'], preflight_phase)
        batches[role] = batch_evidence(role, dispatch_root, d, sources)
        assert 0 <= d['seconds'] <= times[role]['completeJobSeconds'] + 2
        times[role].update(dispatchSeconds=d['seconds'], inputValidationSeconds=d['inputValidationSeconds'],
            compileSeconds=sum(row.get('compilationSeconds', 0) for row in d['instances']), nativeSeconds=d['native']['seconds'],
            managedWorkerSeconds=sum(row.get('workerSeconds', 0) for row in d['instances']),
            batchSeconds=batches[role]['seconds'], batchRequests=batches[role]['requests'], batchFullFences=batches[role]['fullFences'])
    assert sdk['seed']['sdkCalls'] == {'restore': 1, 'save': 1} and sdk['control']['sdkCalls'] == {'restore': 0, 'save': 0}
    assert sdk['consumer']['sdkCalls'] == {'restore': 1, 'save': 0}
    for role in ['seed', 'control']:
        d = dispatches[role]
        assert d['managedExecutions'] == 108 and d['replayedInstances'] == d['skippedCompilations'] == d['inputCandidates'] == 0
        assert d['nativeSpecsExecuted'] == 120 and state_rows(controls[role]['executed']) == original_states
        assert file_rows(controls[role]['executed']) == original_files
    assert dispatches['seed']['recordedReceipts'] == 108 and dispatches['control']['recordedReceipts'] == 0
    consumer = dispatches['consumer']
    assert consumer['managedExecutions'] == 0 and consumer['replayedInstances'] == consumer['skippedCompilations'] == consumer['inputCandidates'] == 108
    assert consumer['nativeSpecsExecuted'] == 12 and consumer['nativeConformanceInstances'] == 0 and consumer['recordedReceipts'] == 0
    assert consumer['publication']['saved'] is False and consumer['refreshedCandidates'] == 0
    consumer_root = role_roots['consumer'] / 'dispatch'
    assert not any(p.name.startswith('managed-') for p in consumer_root.iterdir())
    assert [p.relative_to(consumer_root).as_posix() for p in consumer_root.rglob('worker.mjs')] == ['preflight/worker.mjs']
    assert not any(p.name in {'test-build.json', 'compile-certificate.json', 'recorded-candidate.json'} for p in (consumer_root / 'instances').rglob('*'))
    assert len({s['scopeSha256'] for s in sdk.values()}) == len({s['logicalSha256'] for s in sdk.values()}) == 1
    seed_pack = raw(role_roots['seed'] / 'authenticated-collection.json', MAX_PACK)
    assert seed_pack == raw(role_roots['consumer'] / 'authenticated-collection.json', MAX_PACK)
    pack = decode(seed_pack); pack_sha = sha(seed_pack)
    exact(pack, ['schema', 'scope', 'entries'])
    assert pack['schema'] == 'quod-managed-collection-v1' and pack['scope'] == sdk['seed']['scopeSha256']
    assert len(pack['entries']) == 108 and [entry['executionKey'] for entry in pack['entries']] == sorted(entry['executionKey'] for entry in pack['entries'])
    encoded_entries = []
    for entry in pack['entries']:
        exact(entry, ['executionKey', 'receiptSha256', 'receiptBase64'])
        encoded_entries.append({key: entry[key] for key in ['executionKey', 'receiptSha256', 'receiptBase64']})
    # The fixed Action encoder uses this insertion order plus newline. It does
    # not use the sorted canonical envelope/receipt format checked separately.
    encoded_pack = {'schema': pack['schema'], 'scope': pack['scope'], 'entries': encoded_entries}
    assert (json.dumps(encoded_pack, separators=(',', ':'), ensure_ascii=False, allow_nan=False) + '\n').encode() == seed_pack
    saved = [e for e in sdk['seed']['transport'] if e['operation'] == 'collection-save']
    restored = [e for e in sdk['consumer']['transport'] if e['operation'] == 'collection-restore']
    assert len(saved) == len(restored) == 1
    service_key = sdk['consumer']['servicePrefix'] + pack_sha
    assert saved[0]['key'] == restored[0]['matchedServiceKey'] == service_key and restored[0]['reason'] == 'hit'
    assert saved[0]['collectionSha256'] == restored[0]['collectionSha256'] == pack_sha
    assert saved[0]['entries'] == restored[0]['entries'] == 108 and type(saved[0]['cacheId']) is int and saved[0]['cacheId'] > 0
    assert dispatches['seed']['publication']['saved'] is True and dispatches['seed']['publication']['cacheId'] == saved[0]['cacheId']
    assert sdk['seed']['collectionSha256'] == sdk['consumer']['collectionSha256'] == pack_sha
    seed_restores = [e for e in sdk['seed']['transport'] if e['operation'] == 'collection-restore']
    assert len(seed_restores) == 1 and seed_restores[0]['reason'] == 'miss'
    assert sdk['control']['transport'] == [] and sdk['control']['actualSdkAuthenticationVerified'] is False
    assert collections.Counter(event['operation'] for event in sdk['seed']['transport']) == {'collection-restore': 1, 'collection-save': 1}
    assert collections.Counter(event['operation'] for event in sdk['consumer']['transport']) == {'collection-restore': 1, 'collection-entry': 108}
    assert sdk['seed']['actualSdkAuthenticationVerified'] is True and sdk['consumer']['actualSdkAuthenticationVerified'] is True
    assert not (role_roots['control'] / 'authenticated-collection.json').exists()
    for role in ['seed', 'consumer']:
        retained = sdk[role]['retainedCollection']
        assert retained == {'file': 'authenticated-collection.json', 'sizeBytes': len(seed_pack), 'sha256': pack_sha,
            'matchedServiceKey': service_key, 'source': 'SDK-operation-verified-bytes; audit-only'}
    entries = {e['executionKey']: e for e in pack['entries']}
    events_list = [e for e in sdk['consumer']['transport'] if e['operation'] == 'collection-entry']
    events = {e['executionKey']: e for e in events_list}
    assert len(entries) == len(events) == len(events_list) == 108 and set(entries) == set(events)
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
        assert execution == read(role_roots['seed'] / 'dispatch' / seed_row['evidenceDirectory'] / 'execution-identity.json')
        assert execution == read(role_roots['control'] / 'dispatch' / control_row['evidenceDirectory'] / 'execution-identity.json')
        check_execution(identity, current, sources)
        assert identity['runtimePolicy']['nativeFingerprintSha256'] == consumer['nativeFingerprintSha256']
        assert identity['host'] == consumer['host'] and identity['nativePolicy'] == consumer['nativePolicy']
        assert identity['supervisorPolicy'] == consumer['supervisorPolicy']
        assert read(directory / 'preparation.json') == identity['preparation']
        options = {'epoch': 1700000000000, 'seed': 42, 'env': {'NODE_ENV': 'test'}, 'vitest': None,
            'file': '/project/' + current['file'], 'providedContext': identity['preparation']['providedContext']}
        seed_built = read(role_roots['seed'] / 'dispatch' / seed_row['evidenceDirectory'] / 'test-build.json')
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
        assert body == raw(role_roots['seed'] / 'dispatch' / seed_row['evidenceDirectory'] / 'recorded-candidate.json', MAX_ITEM)
        envelope, cert, cert_bytes, receipt, receipt_bytes = unpack_envelope(body, candidate_key, entry['receiptSha256'])
        assert read(directory / 'candidate-authentication.json') == {'candidateKey': candidate_key,
            'trustedEnvelopeSha256': entry['receiptSha256'], 'trustedCertificateSha256': sha(cert_bytes), 'trustedReceiptSha256': sha(receipt_bytes)}
        certificate(cert, identity, sources); certificates.append(cert)
        assert receipt['execution'] == execution and sha(receipt_bytes) == current['receiptSha256'] == seed_row['receiptSha256']
        assert receipt_bytes == raw(role_roots['seed'] / 'dispatch/receipts' / (iid + '.json'), MAX_ITEM)
        for role, row in [('seed', seed_row), ('control', control_row)]:
            fresh_dir = role_roots[role] / 'dispatch' / row['evidenceDirectory']
            assert raw(fresh_dir / 'compile-certificate.json', MAX_ITEM) == cert_bytes
            built = read(fresh_dir / 'test-build.json')
            assert built['identity'] == identity['compile'] and built['preparation'] == identity['preparation'] and built['options'] == options
            assert built['identitySha256'] == sha(canonical(identity['compile']).encode())
            worker_dir = role_roots[role] / 'dispatch' / row['workerEvidenceDirectory']
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
        assert response == raw(role_roots['seed'] / 'dispatch' / seed_row['workerEvidenceDirectory'] / 'response.json')
        data = decode(response); boundary(data)
        assert sha(response) == receipt['response']['sha256'] and receipt['response']['encoding'] == 'base64'
        exact(receipt, ['schema', 'researchOnly', 'execution', 'conformance', 'conformanceSha256', 'response', 'resultDigest'])
        assert receipt['schema'] == RECEIPT and receipt['researchOnly'] is True
        conformance = receipt['conformance']; original = seed_native[iid]
        exact(conformance, ['schema', 'executionKey', 'instance', 'nativeReportSha256', 'nativeSelectionSha256', 'nativePolicySha256', 'file', 'rows'])
        assert conformance['schema'] == CONFORMANCE and conformance['executionKey'] == key and conformance['instance'] == identity['instance']
        assert conformance['nativeReportSha256'] == controls['seed']['reportSha256']
        assert conformance['nativeSelectionSha256'] == sha(raw(role_roots['seed'] / 'dispatch/native/native-selection.json'))
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
    assert len(certificates) == 108
    requests = resolution_requests(certificates)
    tools = certificates[0]['tools']; geometry = certificates[0]['rootGeometry']
    assert all(value['tools'] == tools and value['rootGeometry'] == geometry for value in certificates)
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
    assert set(instance_map(ordinary['omittedAsHits'])) == candidate_ids and len(ordinary['executed']) == 12
    assert sum(state_rows(ordinary['executed']).values()) == 184 and outcome_counts == {'pass': 3946, 'skip': 1}
    return {'schema': 'quod-managed-matrix-batch-input-sdk-audit-v1', 'passed': True, 'issues': [], 'run': RUN,
        'sourcePin': SOURCE, 'actionPin': ACTION, 'baselineSourcePin': BASELINE_SOURCE,
        'provenance': provenance, 'reviewedSourceSha256': sources, 'timing': times, 'batches': batches,
        'prerequisite': {'dispatcherCases': 13, 'dispatcherChecks': 142, 'proofChecks': 18, 'legacyLedgerChecks': 152},
        'cache': {'positiveSeedCacheId': saved[0]['cacheId'], 'consumerReturnedServiceKey': service_key,
            'packBytes': len(seed_pack), 'packSha256': pack_sha, 'candidateEnvelopesVerified': 108,
            'certificatesReconstructedExactly': 108, 'receiptsVerified': 108, 'sharedCurrentResolverRequests': len(requests),
            'sdkCalls': {role: value['sdkCalls'] for role, value in sdk.items()}},
        'outcomes': {'selectedInstances': 120, 'replayedInstances': 108, 'skippedCandidateCompilations': 108,
            'managedTestWorkers': 0, 'nativeInstances': 12, 'nativeStates': 184, 'replayedStates': dict(outcome_counts),
            'logicalStates': 4131, 'logicalSha256': sdk['consumer']['logicalSha256'],
            'candidateProjects': dict(sorted(project_counts.items())), 'fullJobHit': False,
            'fullProjectFilePoolNamesStatesAndApiFileStatesEqualBothControlsAndBaseline': True},
        'limits': ['one fixed source profile; no original twelve-commit or full-job-hit claim',
            'one preflight worker and compilation remain; only candidate compilation and managed test workers are absent',
            'opaque proof issuance and auth lifecycle are enforced by reviewed pinned controller code; this data reader binds their retained evidence',
            'full native timing/reporter metadata is not reconstructed; native API file states and all logical rows are preserved',
            'sampled nonadversarial workspace validation; no adversarial atomic snapshot claim']}


def check_execution(identity, row, sources):
    assert identity['schema'] == EXECUTION and identity['instance'] == {key: row[key] for key in ['instanceId', 'project', 'file', 'pool']}
    assert identity['compile']['compilerSha256'] == COMPILER and identity['preparation']['preparerSha256'] == sources['preparation.mjs']
    assert identity['runtimePolicy']['supervisor']['sourceSha256'] == sources['executor.mjs']
    assert identity['supervisorPolicy'] == {'candidateSelectorSha256': sources['matrix-candidates.mjs'],
        'conformanceAdapterSha256': sources['matrix-native.mjs'], 'experimentSha256': sources['matrix-input-dispatcher.mjs'],
        'receiptValidatorSha256': sources['matrix-receipt.mjs'], 'inputValidatorSha256': sources['resolved-compile-certificate.mjs'],
        'candidateEnvelopeSha256': sources['matrix-candidate-envelope.mjs'], 'compilationPolicy': BATCH_POLICY}
    assert identity['nativePolicy']['adapterSha256'] == sources['matrix-native.mjs']
    runtime = identity['runtimePolicy']
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


def audit_fixtures(root, fixture):
    expected = {'seed': 0, 'hit': 0, 'source-byte-miss': 0, 'fresh-resolution-miss': 0, 'missing-authority': 0,
        'wrong-authority': 0, 'authenticated-malformed': 0, 'unsupported-pass': 0, 'unsupported-failure': 1,
        'assertion-failure': 1, 'hook-failure': 1, 'outer-cleanup-error': 1, 'ordinary-failure': 1}
    assert {row['name']: row['exitCode'] for row in fixture['cases']} == expected
    assert len(fixture['cases']) == len(expected) and fixture['actualSdkAuthenticationClaimed'] is False and fixture['productionEnabled'] is False
    compact_cases = {row['name']: row for row in fixture['cases']}
    seed_states = None
    for name, code in expected.items():
        native_code = 0 if name == 'outer-cleanup-error' else code
        case_root = root / name; case = read(case_root / 'result.json'); selected = native(case_root / 'native', passing=native_code == 0)
        assert case['exitCode'] == code and selected['exitCode'] == native_code
        assert case['selectedInstances'] == 2 and case['candidateInstances'] == 1 and case['logicalStates'] == 13
        assert len(selected['originalSelected']) == 2 and len(selected['executed']) == (1 if name == 'hit' else 2)
        assert case['managedExecutions'] == (0 if name == 'hit' else 1) and case['nativeSpecsExecuted'] == (1 if name == 'hit' else 2)
        value, rows, files = logical(case_root, case)
        assert sum(rows.values()) == 13 and sum(files.values()) == 2
        candidate = next(row for row in case['instances'] if row['candidate'])
        compact = compact_cases[name]
        assert compact['publishes'] == 1 and compact['publicationExit'] == code
        assert compact['records'] == (0 if code or name in {'hit', 'unsupported-pass'} else 1)
        assert compact['managedExecutions'] == case['managedExecutions'] and compact['nativeSpecsExecuted'] == case['nativeSpecsExecuted']
        assert compact['logicalSha256'] == case['logicalSha256'] and compact['logicalStates'] == 13
        log = re.sub(r'\x1b\[[0-9;]*m', '', raw(case_root / 'native/native.log').decode('utf-8'))
        marker_lines = [line.strip() for line in log.splitlines()]
        assert marker_lines.count('QUOD_MATRIX_ORDINARY_ONCE') == 1
        assert marker_lines.count('QUOD_MATRIX_CANDIDATE_ONCE') == (0 if name == 'hit' else 1)
        originals = instance_map(selected['executed'])
        for file in value['files']:
            assert {key: file[key] for key in ['instanceId', 'project', 'file', 'pool']} == {key: instance_map(case['instances'])[file['instanceId']][key] for key in ['instanceId', 'project', 'file', 'pool']}
            if file['instanceId'] in originals:
                actual = originals[file['instanceId']]
                assert file['name'] == actual['relativeModuleId'] and file['state'] == actual['state']
            else:
                replay = read(case_root / candidate['evidenceDirectory'] / 'replayed-response.json')
                assert name == 'hit' and file['name'] == replay['files'][0]['name'] and file['state'] == replay['files'][0]['state']
        if name == 'seed':
            seed_states = rows
        elif code == 0 or name == 'outer-cleanup-error':
            assert rows == seed_states
        if name == 'hit':
            assert candidate['decision'] == 'replay' and candidate['compilationSkipped'] is True and candidate['compilationSeconds'] == 0
            assert case['skippedCompilations'] == 1 and not candidate.get('workerEvidenceDirectory')
            assert not (case_root / candidate['evidenceDirectory'] / 'test-build.json').exists()
        else:
            response = read(case_root / candidate['workerEvidenceDirectory'] / 'response.json')
            assert sum('QUOD_MATRIX_CANDIDATE_ONCE' in log[1] for log in response['boundary']['logs']) == 1
            if name in ['source-byte-miss', 'fresh-resolution-miss']:
                assert case['skippedCompilations'] == 0 and isinstance(candidate['inputMissReason'], str)
        if code:
            assert case['recordedReceipts'] == 0 and not (case_root / 'receipts').exists()
        if name in ['assertion-failure', 'hook-failure']:
            assert case['stickyManagedFailure'] is True and candidate['supported'] is True and candidate['decision'] == 'supported-failure'
        if name == 'assertion-failure':
            assert any(row['state'] == 'fail' for row in response['rows'])
        if name == 'hook-failure':
            assert response['ok'] is False and all(row['state'] == 'pass' for row in response['rows']) and response['files'][0]['errors']
        if name == 'outer-cleanup-error':
            boundary(response, closed_error=True)
            assert candidate['closedFailure'] is True and candidate['supported'] is False and candidate['decision'] == 'ordinary'
            assert candidate['reason'] == 'sealed managed runner failed' and candidate['nativeExecuted'] is True
            assert case['stickyManagedFailure'] is True and case['observedNativeFailure'] is False
            assert case['native']['exitCode'] == 0 and selected['failedModules'] == 0 and selected['unhandledErrors'] == []
            assert all(row['state'] in PASSING for row in value['rows'])
        if name.startswith('unsupported-'):
            assert case['stickyManagedFailure'] is False and candidate['supported'] is False and candidate['decision'] == 'ordinary'
            assert response['ok'] is False and 'global fetch' in response['boundary']['unsupported']
        if name == 'ordinary-failure':
            assert candidate['decision'] == 'managed' and response['ok'] is True and case['observedNativeFailure'] is True and case['stickyManagedFailure'] is False


def main():
    global ROOT
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repository', required=True, type=pathlib.Path)
    parser.add_argument('index', type=pathlib.Path)
    args = parser.parse_args()
    try:
        checked_pins()
        ROOT = args.repository.resolve()
        index = read(args.index, 1024 * 1024); parent = args.index.resolve().parent
        for key in ['runJson', 'jobsJson', 'artifactsJson']:
            index[key] = str(parent / index[key])
        for descriptor in index['evidence'].values():
            for key in ['root', 'archive']:
                descriptor[key] = str(parent / descriptor[key])
        result = audit(index)
    except Exception as error:
        result = {'schema': 'quod-managed-matrix-batch-input-sdk-audit-v1', 'passed': False,
            'issues': [type(error).__name__ + ': ' + str(error)]}
    print(json.dumps(result, indent=2, ensure_ascii=False, allow_nan=False))
    return 0 if result['passed'] else 1


if __name__ == '__main__':
    sys.exit(main())

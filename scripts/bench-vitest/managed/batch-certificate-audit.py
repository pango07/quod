#!/usr/bin/env python3
"""Independent data-only reader for completed batch certificate gate42419977028.

Usage: --repository TRUSTED_SOURCE_CHECKOUT INDEX.json
Exact completed run/source/job/archive authority is pinned below. Reads only
ZIP/JSON/Git data and never imports or executes artifact code. Opaque provenance
and quarantine are properties of the pinned trusted controller; retained data
independently binds its exact checks and actual byte outcomes.
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

# ALL future authority remains absent. checked_pins runs before opening INDEX.
RUN = 42419977028
SOURCE = '615fe7303a7566bddb58227827604e65173fb694'
JOB_ID = 128546107010
JOB_TIMESTAMPS = ('2026-11-07T22:18:23Z', '2026-11-07T22:40:51Z')
ARTIFACTS = {'controls': (13593448454, 25190521, 'f830701dd6bef44de359fb0dd437b05bc94e7f402a52f8ba299cb7989c67907e'), 'current-matrix': (13593229892, 34835623, 'd7170f6c96527c90a4b6f35a53b737f60048b7b9547ab1a0669d82e646b593bb'), 'pairs': (13594245657, 49115068, '94b3ac392d6d4e451555f7144184c352792ffd38c6aa75df6e93efc912a4cb46')}
JOB_NAME = 'benchmark-managed-batch-certificate'
ARTIFACT_NAMES = {role: 'managed-batch-certificate-' + role for role in ARTIFACTS}
BASELINE_SOURCE = 'ff279ea5b59b359d26985f08f22e0749f9eccbde'
BASELINE_RUN = 42406036816
BASELINE = (13587034973, 36186785, 'f8e20f3a68cc3fbec7ce01a5ea90e8b50a1d558cbf08146a7fb049e242adc282')
ORIGINAL_SOURCE = '766ff388f1dcc3305d43a14e99655404dc01b596'
ORIGINAL_PROBE = '3c6cf9be9cf705322ef9768da2dc5dc5a68e0369ba75be4d2817aa27613aa894'
COMPILER = 'a87f1f8fafea2e0f1b4a1e299199f5581172a2fe90f59882d3c64eb50f68acaf'
TOOL_FILES = '7b7ae881ef3186e4b564c26ffa6e8d40b3d3875a15e41b10d93a6bb24cb3a354'
CONTRACT = 'quod-managed-v0-protectors-frozen-slots-investigation'
INTRINSICS = 'quod-managed-frozen-array-promise-slots-v1-probe'
EXECUTION = 'quod-managed-matrix-execution-v2-research'
CONFORMANCE = 'quod-managed-matrix-conformance-v2-research'
RECEIPT = 'quod-managed-matrix-receipt-v2-research'
CERTIFICATE = 'quod-managed-resolved-compile-certificate-v1-diagnostic'
MAX_ITEM, MAX_PACK = 2 * 1024 * 1024, 32 * 1024 * 1024
MAX_ARCHIVE, MAX_EXPANDED, MAX_FILE, MAX_PATHS = 100_000_000, 500_000_000, 32 * 1024 * 1024, 20000
CERTIFICATE_LIMITS = {'paths': 100000, 'resolverBytes': 16 * 1024 * 1024, 'resolverTimeoutMs': 30000,
    'toolBytes': 128 * 1024 * 1024, 'sourceBytes': 256 * 1024 * 1024, 'fileBytes': 16 * 1024 * 1024,
    'certificateBytes': 16 * 1024 * 1024, 'certificateSetBytes': 128 * 1024 * 1024, 'certificates': 512}
PASSING = {'pass', 'skip', 'todo'}
STATUS = {'passed': 'pass', 'failed': 'fail', 'skipped': 'skip', 'todo': 'todo'}
PREFLIGHT_DENIALS = ['global fetch', 'global crypto', 'global WeakRef', 'global FinalizationRegistry',
    'Event.timeStamp', 'global WebAssembly', 'AbortSignal.timeout', 'eval', 'Function constructor',
    'AsyncFunction constructor', 'GeneratorFunction constructor', 'AsyncGeneratorFunction constructor',
    'global EventTarget.addEventListener', 'global EventTarget.removeEventListener', 'global EventTarget.dispatchEvent']
SOURCE_FILES = ['compiler.mjs', 'resolved-compile-certificate.mjs', 'batch-compile-certificate-probe.mjs',
    'preparation.mjs', 'matrix-probe.mjs', 'matrix-candidates.mjs', 'matrix-native.mjs',
    'matrix-receipt.mjs', 'matrix-receipt-fixtures.mjs', 'executor.mjs', 'prelude.mjs', 'runner.mjs',
    'matrix-input-dispatcher.mjs', 'matrix-candidate-envelope.mjs', 'matrix-input-dispatcher-fixtures.mjs',
    'matrix-input-proof-fixtures.mjs', 'package.json']
ROOT = None
CHECK_NAMES = {
    'compiler': (55, 'passed', '06e711d6e9d2d43439fadb28e89413f19165fe3f6c1459c3b2d91a173fc84098'),
    'resolved-targeted': (213, 'pass', 'b8029f4a47adcaf0df7a4d26b06be6d2d069663a6125ab72417d38ad926ad741'),
    'dispatcher': (142, 'passed', '3600c16924b44414a737f1dc3121b8a24acfece887d9af3771de8d299a6d1cac'),
    'dispatcher/input-proof-controls.json': (18, 'pass', '1235a9145de80795eb537e84f5f1213369bc07ba7e08d3850bf8793800cc415f'),
    'legacy-ledger': (152, 'passed', 'eb3635ccf0483672568473dd548bc29610902fbc938192dd6c4fa5008c530f79')}
MODES = ['cold-batch', 'cold-prior-initialization', 'cold-unknown-tools', 'malformed', 'prior-factory',
    'prior-compile', 'prior-validation', 'prior-capture', 'reentry', 'concurrent-batch', 'late-compile-failure',
    'before-tool-bytes', 'before-api', 'before-environment', 'during-api', 'during-environment',
    'during-unknown-cache', 'source-resolution-glob']

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


def checked_pins():
    assert type(RUN) is int and RUN > 0, 'reviewed RUN required; no index opened'
    assert isinstance(SOURCE, str) and re.fullmatch('[0-9a-f]{40}', SOURCE)
    assert type(JOB_ID) is int and JOB_ID > 0
    assert isinstance(JOB_TIMESTAMPS, tuple) and len(JOB_TIMESTAMPS) == 2
    assert all(isinstance(value, str) for value in JOB_TIMESTAMPS)
    for role, value in ARTIFACTS.items():
        assert isinstance(value, tuple) and len(value) == 3, 'reviewed artifact pin required: ' + role
        assert type(value[0]) is int and 0 < value[1] < MAX_ARCHIVE and digest(value[2])


def git_source(name, commit=None):
    assert name in SOURCE_FILES or name == 'resolved-compile-certificate-probe.mjs'
    selected = commit or SOURCE
    assert selected in (SOURCE, ORIGINAL_SOURCE)
    env = {**os.environ, 'GIT_NO_LAZY_FETCH': '1', 'GIT_TERMINAL_PROMPT': '0'}
    body = subprocess.check_output(['git', 'show', selected + ':scripts/bench-vitest/managed/' + name], cwd=ROOT, env=env)
    assert 0 < len(body) <= 2 * 1024 * 1024
    return body


def number(value):
    assert type(value) in (int, float) and math.isfinite(value) and value >= 0
    return value


def build(directory, filename='test-build.json', bundle='worker.mjs'):
    value, body = read(directory / filename), raw(directory / bundle)
    assert sha(body) == value['sha256'] == value['identity']['bundleSha256']
    assert sha(canonical(value['identity']).encode()) == value['identitySha256']
    assert value['closure']['finalBundleParsed'] is True
    number(value['seconds'])
    return value, body


def inventory(root, proof):
    value = read(root / 'inventory.json', 1024 * 1024)
    assert value['schema'] == 'quod-managed-batch-evidence-inventory-v1'
    assert value['inventoryReservedPaths'] == 1 and value['inventoryReservedBytes'] == 1024 * 1024
    assert value['maximumPaths'] == MAX_PATHS and value['maximumExpandedBytes'] == MAX_EXPANDED and value['maximumFileBytes'] == MAX_FILE
    expected = {name: row for name, row in proof['files'].items() if name != 'inventory.json'}
    actual = {row['name']: {key: row[key] for key in ['bytes', 'sha256']} for row in value['files']}
    assert len(actual) == len(value['files']) and actual == expected
    assert value['bytesBeforeInventory'] == sum(row['bytes'] for row in actual.values())
    assert value['pathsBeforeInventory'] == len(actual) + proof['directoriesVerified'] + 1
    assert value['bytesBeforeInventory'] <= MAX_EXPANDED - 1024 * 1024
    assert value['pathsBeforeInventory'] <= MAX_PATHS - 1


def exact_controls(root):
    evidence = read(root / 'original-controls.json')
    assert evidence['schema'] == 'quod-managed-batch-original-controls-v1' and evidence['passed'] is True
    expected = []
    results = {}
    for name, (count, field, names_sha) in CHECK_NAMES.items():
        filename = name if name.endswith('.json') else name + '/result.json'
        body = raw(root / filename); value = decode(body)
        assert len(value['checks']) == count and all(row[field] is True for row in value['checks'])
        names = sorted(row['name'] for row in value['checks'])
        # Original twelve-spec forgery loop checks three tokens under each name.
        expected_unique = 189 if name == 'resolved-targeted' else count
        assert len(set(names)) == expected_unique and sha(canonical(names).encode()) == names_sha
        if name != 'compiler':
            assert value['passed'] is True
        expected.append({'file': filename, 'bytes': len(body), 'sha256': sha(body), 'checks': count})
        results[name] = value
    assert evidence['checked'] == [expected[i] for i in [0, 1, 4, 2, 3]]
    assert results['dispatcher']['caseCount'] == 13 and results['dispatcher']['originalSourcesRestored'] is True
    assert results['dispatcher/input-proof-controls.json'] == results['dispatcher']['inputEnvelopeControls']
    assert results['dispatcher/input-proof-controls.json']['actualSdkAuthenticationClaimed'] is False
    audit_fixtures(root / 'dispatcher', results['dispatcher'])
    return results


def strict_controls(root, sources):
    result = read(root / 'result.json')
    assert result['passed'] is True and result['productionEnabled'] is False and result['receiptAdmission'] is False
    assert result['nativePreparationAlwaysRuns'] is True
    assert result['bootstrapEnvironment']['afterInitialization'] == result['bootstrapEnvironment']['afterPreparation'] == {'TEST': 'true', 'VITEST': 'true', 'NODE_ENV': 'test'}
    assert len(result['pairs']) == len({row['instanceId'] for row in result['pairs']}) == 12
    for row in result['pairs']:
        directory = root / row['instanceId']
        seed, sb = build(directory, 'seed-build.json', 'seed.mjs')
        fresh, fb = build(directory, 'fresh-build.json', 'fresh.mjs')
        cbytes = raw(directory / 'certificate.json'); cert = decode(cbytes)
        certificate(cert, {'compile': seed['identity'], 'nativePolicy': {'nodeExecutableSha256': cert['tools']['nodeSha256']}}, sources)
        decision = read(directory / 'validated.json')
        assert cbytes == (canonical(cert) + '\n').encode() and sha(cbytes) == row['certificateSha256'] and len(cbytes) == row['certificateBytes']
        assert sb == fb and seed['identity'] == fresh['identity'] == decision['identity'] and decision['hit'] is True
        assert decision['identitySha256'] == seed['identitySha256'] == fresh['identitySha256']
        assert decision['bundleSha256'] == seed['sha256'] == fresh['sha256']
    seed, seed_body = build(root, 'fixture-seed-build.json', 'fixture-seed.mjs')
    accepts = {'package sideEffects metadata ignored by path-only plugin', 'package type metadata ignored by path-only plugin',
        'tsconfig bytes changed', 'unrelated path added allows equal input candidate', 'unrelated JSON added allows equal input candidate'}
    assert len(result['mutations']) == 18
    mutants = {read(p / 'mutation.json')['name']: p for p in (root / 'mutations').iterdir() if p.is_dir()}
    assert len(mutants) == 18 and set(mutants) == {row['name'] for row in result['mutations']}
    for row in result['mutations']:
        directory = mutants[row['name']]; decision = read(directory / 'decision.json'); accepted = row['name'] in accepts
        assert decision['hit'] is accepted and (accepted or decision['next'] == 'full-compile')
        if (directory / 'fresh-build.json').exists():
            fresh, body = build(directory, 'fresh-build.json', 'fresh.mjs')
            if accepted:
                assert body == seed_body and fresh['identity'] == seed['identity'] == decision['identity']
        else:
            assert row['rejected'] and read(directory / 'fresh-error.json')
        if (row.get('observation') or {}).get('sameMetadataRequired'):
            assert row['observation']['sizeUnchanged'] and row['observation']['mtimeUnchanged']
    warm = {'compiler-source', 'esbuild-api', 'typescript-wrapper-api', 'typescript-delegated-api',
        'typescript-exports', 'typescript-delegated-exports', 'compiler-environment-NODE_ENV'}
    assert {row['name'] for row in result['warmTools']} == warm and len(result['warmTools']) == 7
    for row in result['warmTools']:
        assert row['captureError'] and row['buildError'] and row['factoryError'] and (row.get('plainCaptureError') or row.get('plainBuildError'))
    warm_compiler = root / 'warm-tools/compiler-source'
    warm_child, warm_error = read(warm_compiler / 'child-process.json'), read(warm_compiler / 'separate-node/error.json')
    assert warm_child['status'] == 1 and warm_child['signal'] is None and warm_child['error'] is None
    assert warm_error['message'] == 'unreviewed compiler source bytes' and not (warm_compiler / 'separate-node/certificate.json').exists()
    assert len(result['coldTools']) == 5
    assert {row['name'] for row in result['coldTools']} == {'esbuild-api', 'typescript-wrapper-api', 'typescript-delegated-api', 'platform-tree-extra-file', 'typescript-wrapper-manifest'}
    for row in result['coldTools']:
        directory = root / 'cold-tools' / row['name']; child = read(directory / 'child-process.json'); error = read(directory / 'separate-node/error.json')
        assert sha(raw(directory / 'mutated-tool.data')) == row['changedSha256'] != row['originalSha256'] and row['packageVersionUnchanged'] is True
        assert child['status'] == 1 and child['signal'] is None and child['error'] is None
        assert error['message'] == ('unreviewed TypeScript delegation wrapper' if row['name'] == 'typescript-wrapper-api' else 'unreviewed compiler tool file inventory')
        assert not (directory / 'separate-node/certificate.json').exists()
        if row['name'] == 'typescript-wrapper-manifest':
            mutant = read(directory / 'mutated-tool.data')
            assert mutant['version'] == '6.0.2' and mutant['quodUnreviewedFixture'] is True
    assert [row['name'] for row in result['apiRoutes']] == ['esbuild', 'typescript']
    seed_certificate = read(root / 'fixture-certificate.json')
    for row in result['apiRoutes']:
        name = row['name']; directory = root / 'api-routes' / name
        assert row == read(directory / 'result.json')
        assert row['status'] == 0 and row['signal'] is None and row['error'] is None
        decision = row['decision']; assert decision == decode(raw(directory / 'stdout.log', 1024 * 1024))
        assert decision['execArgv'] == []
        assert len(decision['decisions']) == 1 and decision['decisions'][0]['hit'] is False
        assert decision['decisions'][0]['reason'] == 'unreviewed actual compiler API or binary entry'
        assert decision['decisions'][0]['next'] == 'full-compile'
        original, changed = read(directory / 'original-package.json'), read(directory / 'mutated-package.json')
        assert changed == {**original, 'name': name, 'exports': {'.': {'import': './.quod-api-wrapper.mjs',
            'require': './.quod-api-require.cjs'}, './package.json': './.quod-api-manifest.json'}}
        routes = read(directory / 'routes.json')
        root_name = 'adapter' if name == 'esbuild' else 'project'
        api_name = 'esbuildESM' if name == 'esbuild' else 'typescriptCJS'
        api = seed_certificate['tools']['entries'][api_name]
        label = '/quod-adapter/' if name == 'esbuild' else '/project/'
        assert api.startswith(label)
        relative = './' + api[len(label):]
        assert routes['manifestTarget'] == seed_certificate['rootGeometry'][root_name] + '/node_modules/' + name + '/package.json'
        assert routes['wrapperSource'] == 'export * from ' + json.dumps(relative) + ';\n'
        assert routes['requireSource'] == 'module.exports=require(' + json.dumps(relative) + ');\n'
    relocation = root / 'adapter-relocation'; value = read(relocation / 'result.json')
    inventories = [read(relocation / name) for name in ['source-inventory.json', 'A-inventory.json', 'B-inventory.json']]
    assert inventories[0] == inventories[1] == inventories[2] and sha(canonical(inventories[0]).encode()) == value['sourceInventorySha256']
    relocated = []
    for index, label in enumerate(['A', 'B']):
        built, body = build(relocation, label + '-build.json', label + '.mjs'); cbytes = raw(relocation / (label + '-certificate.json')); cert = decode(cbytes)
        certificate(cert, {'compile': built['identity'], 'nativePolicy': {'nodeExecutableSha256': cert['tools']['nodeSha256']}}, sources)
        assert sha(cbytes) == value['certificates'][index]['sha256'] and len(cbytes) == value['certificates'][index]['bytes']; relocated.append(body)
    assert value['positive']['hit'] is True and value['relocated']['hit'] is False and value['relocated']['next'] == 'full-compile' and relocated[0] != relocated[1]
    return {'pairs': 12, 'mutations': 18, 'warmTools': 7, 'coldTools': 5, 'apiRoutes': 2}


def trace(count):
    rows = ['before-constructor', 'after-constructor/before-compile:0']
    for index in range(count):
        rows += ['after-compile:' + str(index) + '/before-capture:' + str(index),
            'after-capture:' + str(index) + ('/before-publication' if index == count - 1 else '/before-compile:' + str(index + 1))]
    return rows


def batch_diagnostics(value, count, members):
    assert value['requests'] == count and value['fullFences'] == 2 * count + 2 and value['trace'] == trace(count)
    assert value['publicOperationsAtReservation'] == 0
    assert value['authority'] == 'actual compilation and capture only; no response or receipt admission'
    assert value['totalBundles'] == sum(len(item[1]) for item in members)
    assert value['totalIdentities'] == sum(len(canonical(item[0]['identity']).encode()) for item in members)
    assert value['totalCertificates'] == sum(len(item[3]) for item in members)


def main_check_names(chosen, candidates):
    names = ['current original native120 and managed108 matrix passed',
        'all4131 actual native original project/name/states remain unchanged',
        'all120 actual native API file states and repeated projects remain unchanged',
        'all actual project/file identities survive new preparation',
        'batch12 retains all26 constructor and adjacent operation full fences']
    names += ['strict versus batch12 full raw pair ' + row['instanceId'] for row in chosen]
    names += ['batch108 retains all218 constructor and adjacent operation full fences']
    for index, row in enumerate(candidates):
        names += ['current matrix versus batch108 raw bundle and full identity ' + row['instanceId'],
            'batch privately issued actual build recaptures exact strict certificate ' + str(index)]
    for index in range(len(candidates)):
        names += ['current complete resolver and opaque proof validate batch108 member ' + str(index),
            'batch build and current opaque proof give identical whole execution key ' + str(index)]
    names += ['fresh-process boundary control ' + mode for mode in MODES]
    assert len(names) == len(set(names)) == 468
    return names


def child_expected(mode):
    initial = ['project module was not evaluated before trusted compiler work']
    final = ['actual project test source remains unevaluated in trusted compiler process']
    fixed = {
        'cold-batch': ['batch can initialize the strictly pinned cold compiler epoch', 'cold batch build has actual private provenance'],
        'cold-prior-initialization': ['an already initializing public factory excludes the batch', 'declined overlap does not poison completed public initialization'],
        'cold-unknown-tools': ['unreviewed prior tool load cannot initialize a batch epoch', 'failed cold reservation is poisoned'],
        'malformed': ['malformed request ' + str(i) + ' rejects' for i in range(9)] + ['accessor is never invoked', 'pre-reservation declines do not poison valid batch'],
        'reentry': ['reserved batch blocks public API ' + str(i) for i in range(5)] + ['declined public reentry leaves original batch valid'],
        'concurrent-batch': ['second batch cannot overlap first reservation'],
        'source-resolution-glob': ['batch certificate actually validates before mutation'] +
            [name + tail for name in ['source bytes', 'higher-priority resolution', 'lazy glob names'] for tail in
             [' invalidates batch certificate', ' fresh actual compile identity changes', ' exact restoration recovers current proof']]}
    if mode in fixed:
        return initial + fixed[mode] + final
    if mode.startswith('prior-'):
        return initial + ['batch rejects an already active public ' + mode, 'public operation finishes and rejected reservation does not poison'] + final
    rows = ['failed batch exposes no build/certificate/proof result']
    if mode == 'late-compile-failure':
        rows += ['late actual compile fails after first captured member']
    if mode.startswith('during-'):
        rows += ['mutation is observed after actual reserved work starts']
    return initial + rows + ['reserved failure poisons later certified API ' + str(i) for i in range(6)] + final


def child_controls(root, summary):
    assert [child['mode'] for child in summary['children']] == MODES
    positive = {'cold-batch', 'cold-prior-initialization', 'malformed', 'prior-factory', 'prior-compile',
        'prior-validation', 'prior-capture', 'reentry', 'concurrent-batch', 'source-resolution-glob'}
    for child in summary['children']:
        mode = child['mode']; value = read(root / mode / 'result.json'); request = read(root / (mode + '-request.json'))
        assert child['status'] == 0 and child['signal'] is None and child['error'] is None and child['passed'] is True
        # A synchronous watchdog does not prove an extra one-second scheduling
        # bound. Only the actual enclosing measured interval is authoritative.
        assert number(child['seconds']) <= number(summary['seconds']) + 0.01
        assert value['mode'] == request['mode'] == mode and value['passed'] is True
        assert [row['name'] for row in value['checks']] == child_expected(mode) and all(row['pass'] is True for row in value['checks'])
        assert value['publishedResults'] == (1 if mode in positive else 0)
        assert not set(value) & {'results', 'built', 'certificate', 'proof'}
        assert pathlib.PurePosixPath(request['good']['testFile']).name == 'fixture.test.ts'
        assert pathlib.PurePosixPath(request['bad']['testFile']).name == 'bad.ts'
        if mode in {'cold-unknown-tools', 'late-compile-failure', 'before-tool-bytes', 'before-api', 'before-environment', 'during-api', 'during-environment', 'during-unknown-cache'}:
            error = value['failure']; assert error['publicationFields'] == [] and isinstance(error['stack'], str)
            expected = ('compiler tools already loaded outside certified epoch' if mode == 'cold-unknown-tools'
                else 'compiler tool epoch changed' if mode.endswith('environment')
                else 'loaded compiler exports changed' if mode.endswith('api')
                else 'batch loaded an unknown or replaced cached module' if mode == 'during-unknown-cache'
                else 'unreviewed compiler tool file inventory' if mode == 'before-tool-bytes'
                else 'only a named AsyncLocalStorage import is supported: node:fs')
            assert expected in error['message']
            if mode.startswith('during-'):
                assert 'after-constructor/before-compile:0' in error['batchFenceTrace']
            if mode == 'late-compile-failure':
                assert 'after-capture:0/before-compile:1' in error['batchFenceTrace']
                assert not any(name.endswith('/before-publication') for name in error['batchFenceTrace'])
        if mode == 'source-resolution-glob':
            for name in ['source-bytes', 'higher-priority-resolution', 'lazy-glob-names']:
                built, body = build(root / mode, name + '-build.json', name + '.mjs')
                decision = read(root / mode / (name + '-decision.json'))
                assert decision['hit'] is False and decision['next'] == 'full-compile'
                assert b'__quodBatchModuleEvaluations' in body
    return {'freshProcesses': 18, 'sourceUnevaluatedChecksPerProcess': 2,
        'privateProvenanceScope': 'exact pinned controller and retained rejection checks; data cannot reconstruct WeakMap state'}


def timing(result, current, job_seconds):
    # API timestamps have one-second precision; process timers use a monotonic
    # clock. Permit only a small timestamp/serialization tolerance across them.
    job_tolerance, timer_tolerance = 2.0, 0.01
    probe_seconds, matrix_seconds = number(result['seconds']), number(current['seconds'])
    assert probe_seconds <= job_seconds + job_tolerance and matrix_seconds <= job_seconds + job_tolerance
    # These are separate sequential workflow commands in the pinned gate.
    assert probe_seconds + matrix_seconds <= job_seconds + job_tolerance
    phases = {key: number(result[key]) for key in ['strict12Seconds', 'batch12Seconds', 'batch108Seconds', 'validation108Seconds']}
    assert all(value <= probe_seconds + timer_tolerance for value in phases.values())
    child_seconds = sum(number(row['seconds']) for row in result['children'])
    # Pinned probe source orders all four measured sections and the eighteen
    # child intervals sequentially, outside each other and inside probe.seconds.
    assert sum(phases.values()) + child_seconds <= probe_seconds + timer_tolerance
    return {'jobSeconds': job_seconds, 'modeledRoundedMinutes': math.ceil(job_seconds / 60),
        **phases, 'seconds': probe_seconds, 'currentMatrixSeconds': matrix_seconds,
        'childSeconds': child_seconds, 'apiTimestampToleranceSeconds': job_tolerance,
        'monotonicIntervalToleranceSeconds': timer_tolerance}


def audit(index):
    exact(index, ['schema', 'runJson', 'jobsJson', 'artifactsJson', 'evidence'])
    assert index['schema'] == 'quod-managed-batch-certificate-audit-inputs-v1'
    assert set(index['evidence']) == {'controls', 'current-matrix', 'pairs', 'baseline'}
    run, jobs, metadata = read(index['runJson']), read(index['jobsJson']), read(index['artifactsJson'])
    assert run['id'] == RUN and run['head_sha'] == SOURCE and run['run_attempt'] == 1
    assert run['repository']['full_name'] == 'pango07/quod' and run['event'] == 'workflow_dispatch' and run['conclusion'] == 'success' and run['status'] == 'completed'
    job_rows = [row for row in jobs['jobs'] if row['id'] == JOB_ID]; assert len(job_rows) == 1
    job = job_rows[0]; assert job['name'] == JOB_NAME and job['run_id'] == RUN and job['head_sha'] == SOURCE
    assert job['status'] == 'completed' and job['conclusion'] == 'success' and (job['started_at'], job['completed_at']) == JOB_TIMESTAMPS
    for name in ['Preserve all 55 actual compiler controls', 'Preserve all 213 strict API and current-input controls',
        'Independently audit legacy data without executing it', 'Preserve all 152 original ledger controls',
        'Preserve all 13 dispatcher cases, 142 assertions and 18 proof controls', 'Check exact original assertion inventories',
        'Execute all original native120 and current managed108 instances', 'Compare actual batch12 and batch108 bundles, certificates and proofs',
        'Bound and hash each separate evidence role even on failure']:
        steps = [row for row in job['steps'] if row['name'] == name]; assert len(steps) == 1 and steps[0]['conclusion'] == 'success'
    assert len({row['id'] for row in metadata['artifacts']}) == len(metadata['artifacts'])
    artifacts = {row['id']: row for row in metadata['artifacts']}; roots, provenance = {}, {}
    for role in ['controls', 'current-matrix', 'pairs', 'baseline']:
        pin = BASELINE if role == 'baseline' else ARTIFACTS[role]
        if role != 'baseline':
            item = artifacts[pin[0]]; assert item['name'] == ARTIFACT_NAMES[role] and item['size_in_bytes'] == pin[1] and item['digest'] == 'sha256:' + pin[2]
            assert not item['expired'] and item['workflow_run']['id'] == RUN and item['workflow_run']['head_sha'] == SOURCE
        roots[role], provenance[role] = archive(role, index['evidence'][role], pin)
        if role != 'baseline':
            inventory(roots[role], provenance[role])
    sources = {name: sha(git_source(name)) for name in SOURCE_FILES}; assert sources['compiler.mjs'] == COMPILER
    assert sha(git_source('resolved-compile-certificate-probe.mjs', ORIGINAL_SOURCE)) == ORIGINAL_PROBE
    assert raw(roots['controls'] / 'source-pin.txt').decode().strip() == SOURCE
    expected_source_paths = {name: sources[name] for name in ['compiler.mjs', 'resolved-compile-certificate.mjs', 'batch-compile-certificate-probe.mjs']}
    expected_source_paths['resolved-compile-certificate-probe.mjs'] = ORIGINAL_PROBE
    observed = {}
    for line in raw(roots['controls'] / 'source-sha256.txt').decode().splitlines():
        digest_value, filename = line.split(maxsplit=1); name = pathlib.PurePosixPath(filename).name
        assert filename == 'scripts/bench-vitest/managed/' + name and name not in observed; observed[name] = digest_value
    assert observed == expected_source_paths
    original = native(locate(roots['baseline'], ['managed-matrix', 'native-selection.json']).parent, baseline=True)
    reference = read(roots['controls'] / 'reference-audit.json'); assert reference['passed'] is True and reference['issues'] == [] and reference['sourcePin'] == BASELINE_SOURCE
    controls = exact_controls(roots['controls']); strict_controls(roots['controls'] / 'resolved-targeted', sources)
    baseline_root = locate(roots['baseline'], ['managed-matrix', 'native-selection.json']).parent
    roster = decode(git_source('matrix-candidates.mjs').decode().split('Object.freeze(', 1)[1].split('.map(row => Object.freeze(row))', 1)[0]); candidate_ids = set(instance_map(roster)); assert len(candidate_ids) == 108
    audit_legacy(roots['controls'] / 'legacy-ledger', controls['legacy-ledger'], baseline_root, original, sources, candidate_ids)
    current_root = roots['current-matrix']; current = read(current_root / 'result.json'); n = native(current_root, baseline=True)
    assert state_rows(n['executed']) == state_rows(original['executed']) and file_rows(n['executed']) == file_rows(original['executed'])
    assert len(n['executed']) == 120 and sum(state_rows(n['executed']).values()) == 4131
    assert current['passed'] is True and all(current[key] is False for key in ['productionEnabled', 'cacheEnabled', 'receiptAdmission'])
    assert current['selected'] == 120 and current['coverage']['managedPassing'] == 108 and current['coverage']['nativeAssertions'] == 4131 and current['coverage']['disagreements'] == 0
    assert all(row['pass'] is True for row in current['checks']) and current['node'] == 'v24.18.0'
    assert current['runtimePolicy']['supervisor']['sourceSha256'] == sources['executor.mjs']
    assert current['runtimePolicy']['boundaryContract'] == CONTRACT and current['runtimePolicy']['intrinsicPolicy'] == INTRINSICS
    assert current['runtimePolicy']['services'] == ['main', 'deny'] and current['runtimePolicy']['globalOutbound'] == 'deny'
    assert current['runtimePolicy']['bindings'] == [] and current['runtimePolicy']['compatibilityDate'] == '2026-07-01'
    assert current['runtimePolicy']['compatibilityFlags'] == ['nodejs_als'] and current['runtimePolicy']['moduleName'] == 'worker.mjs'
    assert current['runtimePolicy']['request'] == {'method': 'GET', 'path': '/run'}
    native_modules = instance_map(n['executed']); current_instances = instance_map(current['instances'])
    assert len(current_instances) == 120 and set(current_instances) == set(native_modules)
    assert collections.Counter(row['decision'] for row in current['instances']) == {'managed': 108, 'ordinary': 12}
    for iid, row in current_instances.items():
        assert {key: row[key] for key in ['instanceId', 'project', 'file', 'pool']} == {key: native_modules[iid][key] for key in ['instanceId', 'project', 'file', 'pool']}
    candidates = {row['instanceId']: row for row in current['instances'] if row['decision'] == 'managed'}; assert set(candidates) == candidate_ids
    preflight_build, _ = build(current_root, 'preflight-build.json', 'preflight-one/worker.mjs')
    preflights = [read(current_root / label / 'response.json') for label in ['preflight-one', 'preflight-two']]
    for label, data in zip(['preflight-one', 'preflight-two'], preflights):
        boundary(data, preflight=True); assert len(data['checks']) == 31 and all(row['pass'] is True for row in data['checks'])
        assert len({row['name'] for row in data['checks']}) == 31
        phase = next(row for row in current['phases'] if row['name'] == label)
        worker_evidence(current_root / label, preflight_build['identity'], current['runtimePolicy'], current['host'], phase)
    assert preflights[0]['random'] == preflights[1]['random'] and preflights[0]['boundary']['inventory'] == preflights[1]['boundary']['inventory'] and preflights[0]['boundary']['symbolInventory'] == preflights[1]['boundary']['symbolInventory']
    state_counts = collections.Counter(); actual_builds = {}
    for iid, row in candidates.items():
        directory = current_root / row['evidenceDirectory']; built, body = build(directory, 'test-build.json', 'managed/worker.mjs'); actual_builds[iid] = (built, body)
        assert row['bundleSha256'] == built['sha256'] and row['compilationSeconds'] == built['seconds']
        assert number(row['compilationSeconds']) <= number(current['seconds']) + 0.01
        assert number(row['workerSeconds']) <= number(current['seconds']) + 0.01
        expected_policy = {**current['runtimePolicy'], 'supervisor': {**current['runtimePolicy']['supervisor'], 'watchdogMs': 180000 if row['file'].endswith('/stream.property.test.ts') else 30000}}
        response = worker_evidence(directory / 'managed', built['identity'], expected_policy, current['host'], {'name': 'managed', 'responseSha256': row['responseSha256']})
        value = decode(response); boundary(value); actual = native_modules[iid]
        assert row['supported'] is True and row['comparison']['equal'] is True and row['comparison']['differences'] == []
        assert collections.Counter((test['name'], test['state']) for test in value['rows']) == collections.Counter((test['name'], test['state']) for test in actual['rows'])
        assert value['files'][0]['name'] == actual['relativeModuleId'] and value['files'][0]['state'] == actual['state']
        assert built['preparation'] == read(directory / 'preparation.json') and built['identity']['compilerSha256'] == COMPILER
        assert built['preparation']['preparerSha256'] == sources['preparation.mjs']
        assert built['identity']['policy']['keepNames'] is True and built['identity']['policy']['external'] == ['node:async_hooks']
        alias = row['project'] if row['project'].startswith('zod-') else None
        assert built['preparation']['zodAlias'] == alias and (built['identity']['policy'].get('zodAlias') or None) == alias
        assert built['preparation']['providedContext'] == ({'expectedZodAlias': alias} if alias else {})
        for source in built['identity']['sources']:
            if source['source'] in ['/quod-adapter/prelude.mjs', '/quod-adapter/runner.mjs']:
                assert source['sourceSha256'] == sources[source['source'].split('/')[-1]]
        state_counts.update(test['state'] for test in value['rows'])
    assert state_counts == {'pass': 3946, 'skip': 1}
    result = read(roots['pairs'] / 'result.json'); assert result['schema'] == 'quod-managed-certified-batch-probe-v1-diagnostic'
    assert result['passed'] is True and result['sourcePin'] == SOURCE and result['node'] == 'v24.18.0' and result['execArgv'] == []
    assert all(result[key] is False for key in ['productionEnabled', 'receiptAdmission', 'dispatcherWired'])
    assert result['sourceFiles'] == [{'name': name, 'sha256': sources[name]} for name in ['compiler.mjs', 'resolved-compile-certificate.mjs', 'batch-compile-certificate-probe.mjs']]
    assert len(result['checks']) == 468 and len({row['name'] for row in result['checks']}) == 468 and all(row['pass'] is True for row in result['checks'])
    chosen = [row for row in current['instances'] if row['decision'] == 'managed' and
        (row['project'] == 'convex-helpers' and row['file'].replace('packages/convex-helpers/', '') in
         {'standardSchema.test.ts', 'server/stream.property.test.ts', 'server/stream.test.ts', 'server/zod4.functions.test.ts', 'server/stream.distinct.test.ts'} or
         re.fullmatch(r'zod-4\.[0-6]', row['project']) and row['file'] == 'packages/convex-helpers/standardSchema.test.ts')]
    assert len(chosen) == 12
    assert [row['name'] for row in result['checks']] == main_check_names(chosen, list(candidates.values()))
    # These summary digests use sorted canonical strings, precisely as the
    # pinned probe constructs them; Counter comparisons above remain stronger.
    current_native_bytes = raw(current_root / 'native-selection.json')
    original_native_bytes = raw(baseline_root / 'native-selection.json')
    sorted_states = sorted(canonical([module['project'], module['file'], module['pool'], test['name'], test['state']]) for module in n['executed'] for test in module['rows'])
    sorted_files = sorted(canonical([module['project'], module['file'], module['pool'], module['relativeModuleId'], module['state']]) for module in n['executed'])
    assert result['nativeComparison'] == {'currentSelectionSha256': sha(current_native_bytes),
        'originalSelectionSha256': sha(original_native_bytes), 'projectStateSha256': sha(canonical(sorted_states).encode()),
        'projectFileStateSha256': sha(canonical(sorted_files).encode())}
    twelve = []
    for row in chosen:
        a = roots['pairs'] / ('strict-' + row['instanceId']); b = roots['pairs'] / ('batch12-' + row['instanceId'])
        strict, sb = build(a); batched, bb = build(b); ac, bc = raw(a / 'compile-certificate.json'), raw(b / 'compile-certificate.json')
        assert sb == bb and strict['identity'] == batched['identity'] and strict['identitySha256'] == batched['identitySha256'] and ac == bc
        assert strict['options'] == batched['options'] and strict['preparation'] == batched['preparation']
        cert = decode(bc); certificate(cert, {'compile': batched['identity'], 'nativePolicy': {'nodeExecutableSha256': cert['tools']['nodeSha256']}}, sources)
        assert bc == (canonical(cert) + '\n').encode(); twelve.append((batched, bb, cert, bc))
    batch_diagnostics(result['batch12Diagnostics'], 12, twelve)
    assert len(result['pairs']) == len({row['instanceId'] for row in result['pairs']}) == 108
    assert [row['instanceId'] for row in result['pairs']] == list(candidates)
    assert {row['instanceId'] for row in result['pairs']} == candidate_ids; members, decisions = [], []
    fingerprint = sha(canonical({'preflightIdentity': preflight_build['identitySha256'], 'inventory': preflights[0]['boundary']['inventory'], 'symbols': preflights[0]['boundary']['symbolInventory']}).encode())
    for row in result['pairs']:
        iid = row['instanceId']; directory = roots['pairs'] / ('batch108-' + iid); built, body = build(directory); certbytes = raw(directory / 'compile-certificate.json'); cert = decode(certbytes)
        assert (body, built['identity']) == (actual_builds[iid][1], actual_builds[iid][0]['identity'])
        assert built['options'] == actual_builds[iid][0]['options'] and built['preparation'] == actual_builds[iid][0]['preparation']
        decision = read(directory / 'validated.json'); execution = read(directory / 'execution-identity.json'); identity = execution['identity']
        assert execution['key'] == domain(EXECUTION, identity) == row['executionKey'] and identity['compile'] == built['identity'] and identity['preparation'] == built['preparation']
        assert identity['instance'] == {key: candidates[iid][key] for key in ['instanceId', 'project', 'file', 'pool']}
        assert identity['host'] == current['host'] and identity['runtimePolicy']['nativeFingerprintSha256'] == fingerprint
        assert identity['supervisorPolicy'] == {'receiptValidatorSha256': sources['matrix-receipt.mjs'], 'experimentSha256': sources['batch-compile-certificate-probe.mjs'], 'conformanceAdapterSha256': sources['matrix-native.mjs'], 'candidateSelectorSha256': sources['matrix-candidates.mjs']}
        assert identity['nativePolicy'] == {'adapterSha256': sources['matrix-native.mjs'], 'nodeExecutableSha256': identity['runtimePolicy']['supervisor']['nodeExecutableSha256'], 'nodeVersion': 'v24.18.0', 'vitestVersion': '5.0.0', 'host': current['host']}
        expected_policy = {**current['runtimePolicy'], 'nativeFingerprintSha256': fingerprint, 'supervisor': {**current['runtimePolicy']['supervisor'], 'watchdogMs': 180000 if row['file'].endswith('/stream.property.test.ts') else 30000}}
        assert identity['runtimePolicy'] == expected_policy
        certificate(cert, identity, sources); assert certbytes == (canonical(cert) + '\n').encode() and sha(certbytes) == row['certificateSha256']
        assert decision['hit'] is True and decision['proof'] == {} and decision['identity'] == built['identity']
        assert decision['identitySha256'] == built['identitySha256'] == row['identitySha256'] and decision['bundleSha256'] == built['sha256'] == row['bundleSha256']
        assert decision['authority'] == 'input equivalence only; authenticated passing receipt admission still required'
        members.append((built, body, cert, certbytes)); decisions.append(decision)
    batch_diagnostics(result['batch108Diagnostics'], 108, members)
    requests = resolution_requests([item[2] for item in members]); tools = members[0][2]['tools']; geometry = members[0][2]['rootGeometry']
    request_digest = sha((canonical({'project': geometry['project'], 'adapterRoot': geometry['adapter'], 'requests': requests, 'expectedToolsSha256': sha(canonical(tools).encode())}) + '\n').encode())
    for decision in decisions:
        assert decision['uniqueResolverRequests'] == len(requests) and decision['resolver'] == {'requestSha256': request_digest, 'toolIdentitySha256': sha(canonical(tools).encode()), 'node': 'v24.18.0', 'execArgv': [], 'profile': 'one new plain Node process and one new resolver-only esbuild build'}
    children = child_controls(roots['pairs'], result)
    starts, ends = (datetime.datetime.fromisoformat(value.replace('Z', '+00:00')) for value in JOB_TIMESTAMPS); job_seconds = (ends - starts).total_seconds(); number(job_seconds)
    return {'schema': 'quod-managed-batch-certificate-audit-v1', 'passed': True, 'issues': [], 'run': RUN, 'sourcePin': SOURCE,
        'provenance': provenance, 'reviewedSourceSha256': sources, 'originalProbeSha256': ORIGINAL_PROBE,
        'controls': {name: value[0] for name, value in CHECK_NAMES.items()}, 'children': children,
        'pairs': {'strictBatch12': 12, 'currentBatch108': 108, 'fences12': 26, 'fences108': 218, 'currentResolverRequests': len(requests)},
        'outcomes': {'nativeInstances': 120, 'nativeStates': 4131, 'managedInstances': 108, 'managedStates': dict(state_counts), 'nativeApiFileStatesEqualBaseline': True},
        'timing': timing(result, current, job_seconds),
        'limits': ['unwired diagnostic; no receipts/cache/dispatcher authority', 'actual fixed-order timers only; no speedup or original twelve-commit window claim', 'sampled nonadversarial workspace and reviewed trusted controller/Node/tool scope', 'private quarantine/provenance is enforced by pinned source; retained bytes bind outcomes, not WeakMap internals']}


def main():
    global ROOT
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repository', required=True, type=pathlib.Path)
    parser.add_argument('index', type=pathlib.Path)
    args = parser.parse_args()
    try:
        checked_pins()  # Deliberately fails before any index/artifact/Git read.
        ROOT = args.repository.resolve()
        index = read(args.index, 1024 * 1024); parent = args.index.resolve().parent
        for key in ['runJson', 'jobsJson', 'artifactsJson']:
            index[key] = str(parent / index[key])
        for descriptor in index['evidence'].values():
            exact(descriptor, ['root', 'archive'])
            for key in ['root', 'archive']:
                descriptor[key] = str(parent / descriptor[key])
        result = audit(index)
    except Exception as error:
        result = {'schema': 'quod-managed-batch-certificate-audit-v1', 'passed': False, 'issues': [type(error).__name__ + ': ' + str(error)]}
    print(json.dumps(result, indent=2, ensure_ascii=False, allow_nan=False))
    return 0 if result['passed'] else 1


if __name__ == '__main__':
    sys.exit(main())

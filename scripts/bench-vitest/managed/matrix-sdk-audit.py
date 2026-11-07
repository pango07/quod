#!/usr/bin/env python3
"""Data-only audit of one pinned SDK experiment; never execute artifact code.

Usage: matrix-sdk-audit.py EVIDENCE_INDEX.json
The index supplies local paths only. Expected source pins, archive byte SHAs,
job IDs and the baseline are reviewed constants below, not artifact authority.
"""
import argparse
import base64
import collections
import datetime
import hashlib
import json
import pathlib
import re
import stat
import subprocess
import sys
import zipfile

RUN = 42408971680
SOURCE = 'd575e2221c0aac26305e0667928a0472e0b733af'
ACTION = '02f65151206f3a12d6f17e3b3d1831b704f28a0b'
BASELINE_SOURCE = 'ff279ea5b59b359d26985f08f22e0749f9eccbde'
CONTRACT = 'quod-managed-v0-protectors-frozen-slots-investigation'
INTRINSICS = 'quod-managed-frozen-array-promise-slots-v1-probe'
ARTIFACTS = {
    'fixtures': (13588572992, 2383560, '5b6837959a37ff3fe11318d313c62ea2b4e364465e64fc1365a60435e8dee8d3', 128512744062, 'matrix-dispatcher-fixtures'),
    'seed': (13587849768, 39681460, '6507f0279798d3dfced9db57ed59888242ae5851418195eb5e746f0a9f163d22', 128513018007, 'matrix-sdk-seed'),
    'control': (13588504068, 34113183, '1c9b5c5c97f64b7cefcd447dfb1c75fa23369646f54cce27bdc066043ac54adc', 128513018153, 'matrix-sdk-control'),
    'consumer': (13588713530, 8028539, 'eb768579085bf693e9119881ebb3c7d5df72b2a824909382a912befc80211f9a', 128514414023, 'matrix-sdk-consumer'),
    'baseline': (13587034973, 36186785, 'f8e20f3a68cc3fbec7ce01a5ea90e8b50a1d558cbf08146a7fb049e242adc282', 128503824968, 'benchmark-managed-matrix'),
}
ACTION_SOURCES = {'collectionSourceSha256': '1b4aa76730aee3fd2984da6ed79a15eabe40c21d9977077ce2ff64ab3d5473dc',
                  'authSourceSha256': 'dab0b9d4a7d8b54d3cde5791d0d0cbc2694bf2b7a00598d1f49a67ffe253de22'}
JOB_TIMESTAMPS = {
    'fixtures': ('2026-11-07T20:01:07Z', '2026-11-07T20:02:09Z'),
    'seed': ('2026-11-07T20:02:11Z', '2026-11-07T20:06:35Z'),
    'control': ('2026-11-07T20:02:11Z', '2026-11-07T20:07:39Z'),
    'consumer': ('2026-11-07T20:07:41Z', '2026-11-07T20:10:00Z'),
}
PASSING = {'pass', 'skip', 'todo'}
STATUS = {'passed': 'pass', 'failed': 'fail', 'skipped': 'skip', 'todo': 'todo'}
PREFLIGHT_DENIALS = ['global fetch', 'global crypto', 'global WeakRef', 'global FinalizationRegistry',
    'Event.timeStamp', 'global WebAssembly', 'AbortSignal.timeout', 'eval', 'Function constructor',
    'AsyncFunction constructor', 'GeneratorFunction constructor', 'AsyncGeneratorFunction constructor',
    'global EventTarget.addEventListener', 'global EventTarget.removeEventListener', 'global EventTarget.dispatchEvent']
REPOSITORY = pathlib.Path(__file__).resolve().parents[3]


def sha(value):
    return hashlib.sha256(value).hexdigest()


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=False)


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        assert key not in result, 'duplicate JSON key: ' + key
        result[key] = value
    return result


def decode(value):
    return json.loads(value, object_pairs_hook=unique_object)


def read(file):
    return decode(pathlib.Path(file).read_bytes())


def domain(name, value):
    return sha((name + '\n' + canonical(value)).encode())


def git_source(file):
    return subprocess.check_output(['git', 'show', SOURCE + ':scripts/bench-vitest/managed/' + file], cwd=REPOSITORY)


def archive(role, descriptor):
    root, file = pathlib.Path(descriptor['root']), pathlib.Path(descriptor['archive'])
    expected = ARTIFACTS[role]
    assert root.is_dir() and file.is_file() and not root.is_symlink() and not file.is_symlink()
    body = file.read_bytes()
    assert len(body) == expected[1] and sha(body) == expected[2], role + ' archive byte authority differs'
    names = set()
    with zipfile.ZipFile(file) as source:
        for entry in source.infolist():
            name = pathlib.PurePosixPath(entry.filename)
            assert not name.is_absolute() and '..' not in name.parts and '\\' not in entry.filename
            assert not stat.S_ISLNK(entry.external_attr >> 16)
            if entry.is_dir():
                continue
            assert entry.filename not in names
            names.add(entry.filename)
            target = root.joinpath(*name.parts)
            assert target.is_file() and not target.is_symlink()
            assert target.read_bytes() == source.read(entry), role + ' extracted entry differs: ' + entry.filename
    assert {str(p.relative_to(root)) for p in root.rglob('*') if p.is_file()} == names, role + ' retained file set differs'
    return root, {'artifactId': expected[0], 'archiveSizeBytes': len(body), 'archiveSha256': sha(body), 'filesByteVerified': len(names)}


def state_rows(modules):
    return collections.Counter((m['project'], m['file'], m['pool'], t['name'], t['state'])
                               for m in modules for t in m['rows'])


def native(root, baseline=False):
    selected = read(root / 'native-selection.json')
    report_bytes = (root / 'native-report.json').read_bytes()
    report = decode(report_bytes)
    assert selected['matched'] and selected['completed'] and selected['runStarted'] and selected['exitCode'] == 0
    assert selected['failedModules'] == 0 and selected['unhandledErrors'] == [] and report['success']
    assert selected['reportSha256'] == sha(report_bytes)
    assert len(selected['selected']) == len(selected['executed']) == len(report['testResults'])
    assert {r['instanceId'] for r in selected['selected']} == {r['instanceId'] for r in selected['executed']}
    if baseline:
        assert 'originalSelected' not in selected
        selected['originalSelected'] = selected['selected']
    for row in selected['originalSelected']:
        identity = {k: row[k] for k in ['file', 'project', 'pool', 'testLines', 'testIds', 'testTagsFilter', 'testNamePattern']}
        assert all(identity[k] is None for k in ['testLines', 'testIds', 'testTagsFilter', 'testNamePattern'])
        assert sha(canonical(identity).encode()) == row['instanceId']
    api = collections.Counter((m['absolute'], t['name'], t['state']) for m in selected['executed'] for t in m['rows'])
    reporter = collections.Counter((m['name'], t['fullName'], STATUS[t['status']]) for m in report['testResults'] for t in m['assertionResults'])
    assert api == reporter, 'unchanged native reporter differs from public per-project results'
    for module in selected['executed']:
        assert module['ok'] and module['state'] in PASSING and module['errors'] == []
        assert all(t['state'] in PASSING for t in module['rows'])
    return selected


def boundary(data, file_required=True):
    assert data['ok']
    if file_required:
        assert len(data['files']) == 1 and not data['files'][0]['errors']
    b = data['boundary']; c = b['completion']
    assert b['contract'] == CONTRACT and b['intrinsicPolicy'] == INTRINSICS and b['replayEligible'] is False
    assert c['sealed'] and c['intrinsicLockdown'] and c['reason'] == '' and 2 <= c['nativeTurnFences'] <= 512
    assert b['pendingTimers'] == 0 and b['pendingTimerDetails'] == []
    assert all(c[k] == 0 for k in ['pendingTimerCallbacks', 'pendingMicrotasks', 'pendingEventCallbacks'])
    assert b['unsupported'] == ([] if file_required else PREFLIGHT_DENIALS)
    assert all(b[k] == [] for k in ['unguardable', 'symbolUnguardable', 'timerErrors', 'unhandledErrors'])
    if file_required:
        assert all(t['state'] in PASSING and isinstance(t['name'], str) for t in data['rows'])


def audit(index):
    assert index['schema'] == 'quod-managed-matrix-sdk-audit-inputs-v1'
    run, jobs, metadata = read(index['runJson']), read(index['jobsJson']), read(index['artifactsJson'])
    assert run['id'] == RUN and run['head_sha'] == SOURCE and run['event'] == 'workflow_dispatch'
    assert run['status'] == 'completed' and run['conclusion'] == 'success' and run['run_attempt'] == 1
    jobs_by_id = {j['id']: j for j in jobs['jobs']}
    artifacts_by_id = {a['id']: a for a in metadata['artifacts']}
    roots, provenance, times = {}, {}, {}
    for role in ['fixtures', 'seed', 'control', 'consumer', 'baseline']:
        roots[role], provenance[role] = archive(role, index['evidence'][role])
        if role == 'baseline':
            continue
        artifact_id, _, digest, job_id, job_name = ARTIFACTS[role]
        actual = artifacts_by_id[artifact_id]
        assert actual['workflow_run']['id'] == RUN and actual['workflow_run']['head_sha'] == SOURCE
        assert actual['digest'] == 'sha256:' + digest and actual['size_in_bytes'] == ARTIFACTS[role][1]
        assert actual['name'] == 'managed-matrix-' + ('dispatcher-fixtures' if role == 'fixtures' else 'sdk-' + role)
        job = jobs_by_id[job_id]
        assert job['name'] == job_name and job['status'] == 'completed' and job['conclusion'] == 'success'
        assert (job['started_at'], job['completed_at']) == JOB_TIMESTAMPS[role]
        start = datetime.datetime.fromisoformat(job['started_at'].replace('Z', '+00:00'))
        end = datetime.datetime.fromisoformat(job['completed_at'].replace('Z', '+00:00'))
        times[role] = {'completeJobSeconds': (end - start).total_seconds(), 'startedAt': job['started_at'], 'completedAt': job['completed_at']}
        provenance[role].update({'jobId': job_id, 'artifactName': actual['name']})
    assert len({jobs_by_id[ARTIFACTS[r][3]]['runner_id'] for r in ['seed', 'control', 'consumer']}) == 3
    fixture = read(roots['fixtures'] / 'result.json')
    assert fixture['passed'] and fixture['originalSourcesRestored'] and fixture['caseCount'] == 10 and fixture['checkCount'] == 89
    assert len(fixture['checks']) == 89 and all(row['passed'] for row in fixture['checks'])
    expected_cases = {'seed': 0, 'hit': 0, 'missing-authority': 0, 'wrong-authority': 0, 'authenticated-malformed': 0,
                      'unsupported-pass': 0, 'unsupported-failure': 1, 'assertion-failure': 1, 'hook-failure': 1, 'ordinary-failure': 1}
    assert {row['name']: row['exitCode'] for row in fixture['cases']} == expected_cases
    for name, code in expected_cases.items():
        case = read(roots['fixtures'] / name / 'result.json')
        selected = read(roots['fixtures'] / name / 'native/native-selection.json')
        assert selected['matched'] and selected['completed'] and selected['exitCode'] == code
        assert len(selected['originalSelected']) == 2 and len(selected['executed']) == (1 if name == 'hit' else 2)
        assert case['exitCode'] == code and case['selectedInstances'] == 2 and case['candidateInstances'] == 1 and case['logicalStates'] == 13
        assert case['managedExecutions'] == (0 if name == 'hit' else 1) and case['nativeSpecsExecuted'] == (1 if name == 'hit' else 2)
        assert len(read(roots['fixtures'] / name / 'logical-results.json')['rows']) == 13
        candidate = next(row for row in case['instances'] if row['candidate'])
        if name == 'hit':
            assert candidate['decision'] == 'replay' and not candidate.get('workerEvidenceDirectory')
            assert {r['instanceId'] for r in selected['omittedAsHits']} == {candidate['instanceId']}
        else:
            response = read(roots['fixtures'] / name / candidate['workerEvidenceDirectory'] / 'response.json')
        if code:
            assert case['recordedReceipts'] == 0 and not (roots['fixtures'] / name / 'receipts').exists()
        if name in ['assertion-failure', 'hook-failure']:
            assert case['stickyManagedFailure'] and candidate['supported'] and candidate['decision'] == 'supported-failure'
        if name == 'assertion-failure':
            assert any(row['state'] == 'fail' for row in response['rows'])
        if name == 'hook-failure':
            assert not response['ok'] and all(row['state'] == 'pass' for row in response['rows']) and response['files'][0]['errors']
        if name.startswith('unsupported-'):
            assert not case['stickyManagedFailure'] and not candidate['supported'] and candidate['decision'] == 'ordinary'
            assert not response['ok'] and 'global fetch' in response['boundary']['unsupported']
        if name == 'ordinary-failure':
            assert candidate['decision'] == 'managed' and response['ok'] and case['observedNativeFailure'] and not case['stickyManagedFailure']
    baseline = native(roots['baseline'] / '_temp/managed-matrix', baseline=True)
    original_rows = state_rows(baseline['executed'])
    assert len(baseline['executed']) == 120 and sum(original_rows.values()) == 4131
    sources = {name: sha(git_source(name)) for name in ['compiler.mjs', 'preparation.mjs', 'executor.mjs', 'prelude.mjs',
               'runner.mjs', 'matrix-native.mjs', 'matrix-dispatcher.mjs', 'matrix-candidates.mjs', 'matrix-receipt.mjs',
               'matrix-sdk-probe.mjs', 'cache-session-action/index.cjs', 'cache-session-action/action.yml']}
    roster_source = git_source('matrix-candidates.mjs').decode()
    roster = decode(roster_source.split('Object.freeze(', 1)[1].split('.map(row => Object.freeze(row))', 1)[0])
    candidate_ids = {row['instanceId'] for row in roster}
    assert len(candidate_ids) == 108
    sdk, dispatches, controls = {}, {}, {}
    for role in ['seed', 'control', 'consumer']:
        root = roots[role]; s = sdk[role] = read(root / 'sdk-result.json'); d = dispatches[role] = s['dispatch']
        assert s['passed'] and s['sourcePin'] == SOURCE and s['actionPin'] == ACTION
        assert s['scope']['sourcePin'] == SOURCE and s['scope']['actionPin'] == ACTION
        assert all(s['scope'][k] == v for k, v in ACTION_SOURCES.items())
        assert s['scopeSha256'] == sha(canonical(s['scope']).encode()) and s['namespace'] == 'matrix-gate-' + str(RUN) + '-'
        assert s['servicePrefix'] == 'quod-managed-collection-v1-' + s['namespace'] + s['scopeSha256'] + '-'
        assert s['privateSessionRemovedBeforeExecution'] and s['privateSessionRemoved'] and s['authAbsentOutsideSdk']
        assert not any('session' in p.name.lower() or p.name == 'quod-matrix-cache-auth.json' for p in root.rglob('*') if p.is_file())
        assert d['selectedScope'] == {'kind': 'complete-original-default-selection', 'files': None}
        assert d['exitCode'] == 0 and d['selectedInstances'] == 120 and d['candidateInstances'] == 108 and d['ordinaryInstances'] == 12
        assert d['logicalStates'] == 4131 and not d['fullJobHit'] and not d['stickyManagedFailure'] and not d['observedNativeFailure']
        assert {i['instanceId'] for i in d['instances'] if i['candidate']} == candidate_ids
        n = controls[role] = native(root / 'dispatch/native')
        assert {r['instanceId'] for r in n['originalSelected']} == {r['instanceId'] for r in baseline['originalSelected']}
        logical = read(root / 'dispatch/logical-results.json')
        actual = collections.Counter((r['project'], r['file'], r['pool'], r['name'], r['state']) for r in logical['rows'])
        assert actual == original_rows
        sorted_rows = sorted(([r['project'], r['file'], r['pool'], r['name'], r['state']] for r in logical['rows']), key=canonical)
        assert sha(canonical(sorted_rows).encode()) == d['logicalSha256'] == s['logicalSha256']
        assert len(d['phases']) == (1 if role == 'consumer' else 109)
        preflight = read(root / 'dispatch/preflight/response.json'); boundary(preflight, file_required=False)
        assert len(preflight['checks']) == 31 and all(r['pass'] for r in preflight['checks'])
        assert len({r['name'] for r in preflight['checks']}) == 31
        times[role].update({'dispatchSeconds': d['seconds'], 'compileSeconds': sum(i.get('compilationSeconds', 0) for i in d['instances']),
                           'nativeSeconds': d['native']['seconds'], 'managedWorkerSeconds': sum(i.get('workerSeconds', 0) for i in d['instances'])})
    assert sdk['seed']['sdkCalls'] == {'restore': 1, 'save': 1} and sdk['control']['sdkCalls'] == {'restore': 0, 'save': 0}
    assert sdk['consumer']['sdkCalls'] == {'restore': 1, 'save': 0}
    for role in ['seed', 'control']:
        d = dispatches[role]
        assert d['managedExecutions'] == 108 and d['replayedInstances'] == 0 and d['nativeSpecsExecuted'] == 120
        assert state_rows(controls[role]['executed']) == original_rows
    assert dispatches['seed']['recordedReceipts'] == 108 and dispatches['control']['recordedReceipts'] == 0
    consumer = dispatches['consumer']
    assert consumer['managedExecutions'] == 0 and consumer['replayedInstances'] == 108 and consumer['nativeSpecsExecuted'] == 12
    assert consumer['nativeConformanceInstances'] == 0 and consumer['recordedReceipts'] == 0 and not consumer['publication']['saved']
    assert not any(p.name.startswith('managed-') for p in (roots['consumer'] / 'dispatch').iterdir())
    assert len({s['scopeSha256'] for s in sdk.values()}) == 1 and len({s['logicalSha256'] for s in sdk.values()}) == 1
    seed_pack = (roots['seed'] / 'authenticated-collection.json').read_bytes()
    assert seed_pack == (roots['consumer'] / 'authenticated-collection.json').read_bytes()
    assert len(seed_pack) <= 32 * 1024 * 1024
    pack = decode(seed_pack); pack_digest = sha(seed_pack)
    assert pack['schema'] == 'quod-managed-collection-v1' and pack['scope'] == sdk['seed']['scopeSha256']
    assert len(pack['entries']) == 108 and [e['executionKey'] for e in pack['entries']] == sorted(e['executionKey'] for e in pack['entries'])
    saved = [e for e in sdk['seed']['transport'] if e['operation'] == 'collection-save']
    restored = [e for e in sdk['consumer']['transport'] if e['operation'] == 'collection-restore']
    assert len(saved) == len(restored) == 1
    service_key = sdk['consumer']['servicePrefix'] + pack_digest
    assert saved[0]['key'] == restored[0]['matchedServiceKey'] == service_key and restored[0]['reason'] == 'hit'
    assert saved[0]['entries'] == restored[0]['entries'] == 108 and saved[0]['cacheId'] > 0
    assert dispatches['seed']['publication']['saved'] and dispatches['seed']['publication']['cacheId'] == saved[0]['cacheId']
    assert sdk['seed']['collectionSha256'] == sdk['consumer']['collectionSha256'] == pack_digest
    assert [e for e in sdk['seed']['transport'] if e['operation'] == 'collection-restore'][0]['reason'] == 'miss'
    assert sdk['control']['transport'] == [] and not sdk['control']['actualSdkAuthenticationVerified']
    assert sdk['seed']['actualSdkAuthenticationVerified'] and sdk['consumer']['actualSdkAuthenticationVerified']
    entries = {e['executionKey']: e for e in pack['entries']}
    entry_events = [e for e in sdk['consumer']['transport'] if e['operation'] == 'collection-entry']
    assert len(entries) == len(entry_events) == 108
    events = {e['executionKey']: e for e in entry_events}; assert set(events) == set(entries)
    seed_native = {m['instanceId']: m for m in controls['seed']['executed']}
    by_role = {r: {i['instanceId']: i for i in d['instances']} for r, d in dispatches.items()}
    outcome_counts, project_counts = collections.Counter(), collections.Counter()
    for iid in candidate_ids:
        seed_row = by_role['seed'][iid]; current = by_role['consumer'][iid]
        assert seed_row['decision'] == 'managed' and current['decision'] == 'replay' and not current['nativeExecuted']
        assert not current.get('workerEvidenceDirectory')
        execution = read(roots['consumer'] / 'dispatch' / current['evidenceDirectory'] / 'execution-identity.json')
        assert execution == read(roots['seed'] / 'dispatch' / seed_row['evidenceDirectory'] / 'execution-identity.json')
        assert execution == read(roots['control'] / 'dispatch' / by_role['control'][iid]['evidenceDirectory'] / 'execution-identity.json')
        identity = execution['identity']; key = execution['key']
        assert key == domain('quod-managed-matrix-execution-v2-research', identity) == current['executionKey']
        assert identity['instance'] == {k: current[k] for k in ['instanceId', 'project', 'file', 'pool']}
        assert identity['compile']['compilerSha256'] == sources['compiler.mjs']
        assert identity['preparation']['preparerSha256'] == sources['preparation.mjs']
        assert identity['runtimePolicy']['supervisor']['sourceSha256'] == sources['executor.mjs']
        assert identity['supervisorPolicy'] == {'candidateSelectorSha256': sources['matrix-candidates.mjs'],
            'conformanceAdapterSha256': sources['matrix-native.mjs'], 'experimentSha256': sources['matrix-dispatcher.mjs'],
            'receiptValidatorSha256': sources['matrix-receipt.mjs']}
        assert identity['nativePolicy']['adapterSha256'] == sources['matrix-native.mjs']
        runtime = identity['runtimePolicy']; assert runtime['boundaryContract'] == CONTRACT and runtime['intrinsicPolicy'] == INTRINSICS
        assert runtime['services'] == ['main', 'deny'] and runtime['globalOutbound'] == 'deny' and runtime['bindings'] == []
        assert runtime['compatibilityFlags'] == ['nodejs_als'] and set(runtime['processEnv']) == {'PATH', 'TZ', 'LANG'}
        assert runtime['processEnv']['TZ'] == 'UTC' and runtime['processEnv']['LANG'] == 'C.UTF-8'
        policy = identity['compile']['policy']; assert policy['keepNames'] and policy['external'] == ['node:async_hooks']
        assert policy['dynamicCode'] == 'private-sticky-denial-plus-runtime-constructor-guards'
        assert policy['lazyGlob'] == 'actual-real-test-entry-only; all-other-matches-sticky-denied'
        alias = current['project'] if current['project'].startswith('zod-') else None
        assert identity['preparation']['zodAlias'] == alias and (policy.get('zodAlias') or None) == alias
        assert identity['preparation']['providedContext'] == ({'expectedZodAlias': alias} if alias else {})
        for row in identity['compile']['sources']:
            if row['source'] in ['/quod-adapter/prelude.mjs', '/quod-adapter/runner.mjs']:
                assert row['sourceSha256'] == sources[row['source'].split('/')[-1]]
        entry, event = entries[key], events[key]
        raw = base64.b64decode(entry['receiptBase64'], validate=True)
        assert 0 < len(raw) <= 2 * 1024 * 1024 and base64.b64encode(raw).decode() == entry['receiptBase64']
        assert sha(raw) == entry['receiptSha256'] == event['receiptSha256'] == current['receiptSha256']
        assert event['matchedServiceKey'] == service_key and event['collectionSha256'] == pack_digest
        receipt = decode(raw); assert canonical(receipt).encode() == raw and receipt['execution'] == execution
        assert receipt['schema'] == 'quod-managed-matrix-receipt-v2-research' and receipt['researchOnly']
        response = base64.b64decode(receipt['response']['bytes'], validate=True)
        assert sha(response) == receipt['response']['sha256']
        assert response == (roots['consumer'] / 'dispatch' / current['evidenceDirectory'] / 'replayed-response.json').read_bytes()
        assert response == (roots['seed'] / 'dispatch' / seed_row['workerEvidenceDirectory'] / 'response.json').read_bytes()
        data = decode(response); boundary(data)
        conformance = receipt['conformance']; original = seed_native[iid]
        assert conformance['executionKey'] == key and conformance['instance'] == identity['instance']
        assert conformance['nativeReportSha256'] == controls['seed']['reportSha256']
        assert conformance['nativeSelectionSha256'] == sha((roots['seed'] / 'dispatch/native/native-selection.json').read_bytes())
        assert conformance['nativePolicySha256'] == sha(canonical(identity['nativePolicy']).encode())
        expected = collections.Counter((r['name'], r['state']) for r in original['rows'])
        assert collections.Counter((r['name'], r['state']) for r in data['rows']) == expected
        assert collections.Counter((r['name'], r['state']) for r in conformance['rows']) == expected
        assert conformance['file'] == {k: data['files'][0][k] for k in ['name', 'state']}
        assert data['files'][0]['name'] == original['relativeModuleId'] and data['files'][0]['state'] == original['state']
        assert receipt['conformanceSha256'] == domain('quod-managed-matrix-conformance-v2-research', conformance)
        assert receipt['resultDigest'] == sha(('quod-managed-matrix-receipt-v2-research\n' + key + '\n' + receipt['conformanceSha256'] + '\n' + receipt['response']['bytes']).encode())
        for role in ['seed', 'control', 'consumer']:
            row = by_role[role][iid]; built = read(roots[role] / 'dispatch' / row['evidenceDirectory'] / 'test-build.json')
            assert built['identity'] == identity['compile'] and built['preparation'] == identity['preparation']
            assert sha(canonical(built['options']).encode()) == identity['compile']['configurationSha256']
            if role != 'consumer':
                worker = (roots[role] / 'dispatch' / row['workerEvidenceDirectory'] / 'worker.mjs').read_bytes()
                assert sha(worker) == identity['compile']['bundleSha256']
                actual_response = read(roots[role] / 'dispatch' / row['workerEvidenceDirectory'] / 'response.json')
                boundary(actual_response)
                actual_native = next(module for module in controls[role]['executed'] if module['instanceId'] == iid)
                assert collections.Counter((r['name'], r['state']) for r in actual_response['rows']) == collections.Counter(
                    (r['name'], r['state']) for r in actual_native['rows'])
                assert actual_response['files'][0]['name'] == actual_native['relativeModuleId']
                assert actual_response['files'][0]['state'] == actual_native['state']
                assert row['comparison']['equal'] and row['fileMatches'] and not row.get('disagreement', False)
        outcome_counts.update(r['state'] for r in data['rows']); project_counts[current['project']] += 1
    ordinary = controls['consumer']
    assert {r['instanceId'] for r in ordinary['executed']} == {r['instanceId'] for r in baseline['originalSelected']} - candidate_ids
    assert {r['instanceId'] for r in ordinary['omittedAsHits']} == candidate_ids
    assert len(ordinary['executed']) == 12 and sum(state_rows(ordinary['executed']).values()) == 184
    assert outcome_counts == {'pass': 3946, 'skip': 1}
    return {'schema': 'quod-managed-matrix-sdk-audit-v1', 'passed': True, 'issues': [], 'run': RUN,
        'sourcePin': SOURCE, 'actionPin': ACTION, 'baselineSourcePin': BASELINE_SOURCE, 'provenance': provenance,
        'reviewedSourceSha256': sources,
        'prerequisite': {'cases': 10, 'checks': 89, 'allPassed': True, 'originalSourcesRestored': True},
        'cache': {'positiveSeedCacheId': saved[0]['cacheId'], 'consumerReturnedServiceKey': service_key,
                  'packBytes': len(seed_pack), 'packSha256': pack_digest, 'receiptsVerified': 108,
                  'sdkCalls': {role: s['sdkCalls'] for role, s in sdk.items()}},
        'outcomes': {'selectedInstances': 120, 'replayedInstances': 108, 'managedTestWorkers': 0,
                     'nativeInstances': 12, 'nativeStates': 184, 'replayedStates': dict(outcome_counts),
                     'logicalStates': 4131, 'logicalSha256': sdk['consumer']['logicalSha256'],
                     'candidateProjects': dict(sorted(project_counts.items())), 'fullJobHit': False,
                     'fullNamesStatesMultiplicityEqualBothFreshControlsAndBaseline': True},
        'authLifecycle': {'sessionRemovedBeforeExecution': True, 'sessionRemovedFinally': True,
                          'cacheEnvironmentAbsentOutsideAwaitedSdkOperations': True,
                          'evidence': 'pinned controller assertions and retained source identities; no session/auth bytes in artifacts'},
        'timing': times,
        'limits': ['one fixed source profile; this does not update the original twelve-commit frozen benchmark',
                   'diagnostic component and complete-job times; no controlled speedup estimate',
                   'logical project/file/pool/full-name/state multiplicity; no native timing or reporter-metadata reconstruction',
                   'twelve native instances remain, so fullJobHit is false',
                   'all108 candidates are freshly compiled before lookup; the consumer has one preflight worker']}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('index', type=pathlib.Path)
    args = parser.parse_args()
    try:
        index = read(args.index)
        parent = args.index.resolve().parent
        for key in ['runJson', 'jobsJson', 'artifactsJson']:
            index[key] = str(parent / index[key])
        for descriptor in index['evidence'].values():
            for key in ['root', 'archive']:
                descriptor[key] = str(parent / descriptor[key])
        result = audit(index)
    except Exception as error:
        result = {'schema': 'quod-managed-matrix-sdk-audit-v1', 'passed': False,
                  'issues': [type(error).__name__ + ': ' + str(error)]}
    print(json.dumps(result, indent=2, ensure_ascii=False))
    return 0 if result['passed'] else 1


if __name__ == '__main__':
    sys.exit(main())

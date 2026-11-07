"""Independently check extracted managed-runtime evidence; no code execution."""
import hashlib
import json
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
read = lambda name: json.loads((root / name).read_text())
sha = lambda name: hashlib.sha256((root / name).read_bytes()).hexdigest()
result = read('result.json')
assert result['cacheEnabled'] is False
checks = []
random = None
check_count = 16 if result.get('boundaryChecksVersion', 1) >= 2 else 13
for phase in result['phases']:
    name = phase['name']
    response = read(name + '/response.json')
    assert phase['responseSha256'] == sha(name + '/response.json')
    assert phase['data'] == response
    config = (root / name / 'config.capnp').read_text()
    assert 'compatibilityDate = "2026-07-01"' in config
    assert 'compatibilityFlags = ["nodejs_als"]' in config
    assert 'globalOutbound = "deny"' in config
    assert 'nodejs_compat' not in config and 'disk =' not in config
    if name == 'native-outbound':
        assert result['nativeOutboundDenied'] is True
        assert response.get('nativeOutboundError') or response.get('nativeOutboundStatus', 0) >= 500
        assert 'QUOD_DENIED_OUTBOUND' in (root / name / 'runtime.log').read_text()
    else:
        assert response['boundary']['replayEligible'] is False
    if name.startswith('preflight-'):
        assert response['ok'] and len(response['checks']) == check_count
        assert all(c['pass'] for c in response['checks'])
        assert not response['boundary']['unguardable']
        assert not response['boundary']['timerErrors']
        assert response['boundary']['pendingTimers'] == 0
        assert response['boundary']['virtualTime'] == 5
        denied = {
            'global fetch', 'global crypto', 'global WeakRef',
            'global WebAssembly', 'AbortSignal.timeout',
        }
        if check_count == 16:
            denied |= {'global FinalizationRegistry', 'Event.timeStamp'}
        assert set(response['boundary']['unsupported']) == denied
        if random is None:
            random = response['random']
        assert random == response['random']
        checks.append(response['checks'])
    build_name = ('preflight-build.json' if name.startswith('preflight-') else
                  name + '-build.json' if name.startswith('fixture-') or name == 'native-outbound' else
                  'test-build.json')
    build = read(build_name)
    assert build['sha256'] == sha(name + '/worker.mjs')
    assert all(i['path'] == 'node:async_hooks'
               for o in build['metadata']['outputs'].values() for i in o['imports'])

assert len(checks) == 2 and checks[0] == checks[1]
for fixture in result.get('fixtureEvidence', []):
    mode = fixture['mode']
    name = 'fixture-' + mode
    response = read(name + '/response.json')
    assert fixture['managedSupported'] == (mode != 'unsupported')
    assert fixture['managedPassed'] == (mode != 'failure')
    assert fixture['fallbackExecuted'] == (mode == 'unsupported')
    assert fixture['ordinaryExit'] == (1 if mode == 'failure' else 0)
    assert fixture['ordinaryReportSha256'] == sha(name + '-ordinary.json')
    ordinary = sorted([test['fullName'], test['status']]
                      for file in read(name + '-ordinary.json')['testResults'] for test in file['assertionResults'])
    assert len(ordinary) == 4
    assert ordinary == fixture['rows']
    assert sum(state == 'failed' for _, state in ordinary) == (1 if mode == 'failure' else 0)
    states = {'pass': 'passed', 'fail': 'failed'}
    assert sorted([r['name'], states.get(r['state'], r['state'])] for r in response['rows']) == ordinary
    assert response['boundary']['unsupported'] == (['global fetch'] if mode == 'unsupported' else [])
    assert not response['boundary']['unguardable'] and not response['boundary']['timerErrors']
if result.get('fixtureChecksPassed'):
    assert [f['mode'] for f in result['fixtureEvidence']] == ['pass', 'failure', 'unsupported']
report = read('ordinary-report.json')
ordinary = sorted([test['fullName'], test['status']]
                  for file in report['testResults'] for test in file['assertionResults'])
assert len(ordinary) == 47 and all(state == 'passed' for _, state in ordinary)
assert ordinary == result['ordinary']['rows']
assert result['ordinary']['reportSha256'] == sha('ordinary-report.json')
assert result['ordinary']['exit'] == 0
assert result['fallbackExecuted'] == (not result['managedSupported'])
if result['managedSupported']:
    states = {'pass': 'passed', 'fail': 'failed', 'skip': 'skipped', 'todo': 'todo'}
    managed = sorted([r['name'], states.get(r['state'], r['state'])] for r in result['managedRows'])
    assert managed == ordinary and result['namesAndStatesAgree'] is True
    assert result['managedPassed'] is True
    assert result['testSourceSha256'] == '9798bab303233bca846a89ccb39ce28bd968329e0704f3e5d67adca934c11c75'

evidence = {
    'resultSha256': sha('result.json'),
    'preflightChecksPerProcess': check_count,
    'preflightFreshProcesses': 2,
    'preflightRandomValues': random,
    'ordinaryReportSha256': sha('ordinary-report.json'),
    'ordinaryTestStates': 47,
    'managedSupported': result['managedSupported'],
    'fallbackExecuted': result['fallbackExecuted'],
    'namesAndStatesAgree': result.get('namesAndStatesAgree'),
    'nativeOutboundDenied': result.get('nativeOutboundDenied', False),
    'fixtureEvidence': result.get('fixtureEvidence', []),
    'replays': 0,
    'verifiedFiles': {str(p.relative_to(root)): hashlib.sha256(p.read_bytes()).hexdigest()
                      for p in sorted(root.rglob('*')) if p.is_file()},
}
print(json.dumps(evidence, indent=2))

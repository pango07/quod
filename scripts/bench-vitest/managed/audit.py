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
    assert response['boundary']['replayEligible'] is False
    if name.startswith('preflight-'):
        assert response['ok'] and len(response['checks']) == 13
        assert all(c['pass'] for c in response['checks'])
        assert not response['boundary']['unguardable']
        assert not response['boundary']['timerErrors']
        assert response['boundary']['pendingTimers'] == 0
        assert response['boundary']['virtualTime'] == 5
        assert set(response['boundary']['unsupported']) == {
            'global fetch', 'global crypto', 'global WeakRef',
            'global WebAssembly', 'AbortSignal.timeout',
        }
        if random is None:
            random = response['random']
        assert random == response['random']
        checks.append(response['checks'])
    build_name = 'preflight-build.json' if name.startswith('preflight-') else 'test-build.json'
    build = read(build_name)
    assert build['sha256'] == sha(name + '/worker.mjs')
    assert all(i['path'] == 'node:async_hooks'
               for o in build['metadata']['outputs'].values() for i in o['imports'])

assert len(checks) == 2 and checks[0] == checks[1]
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
    'preflightChecksPerProcess': 13,
    'preflightFreshProcesses': 2,
    'preflightRandomValues': random,
    'ordinaryReportSha256': sha('ordinary-report.json'),
    'ordinaryTestStates': 47,
    'managedSupported': result['managedSupported'],
    'fallbackExecuted': result['fallbackExecuted'],
    'namesAndStatesAgree': result.get('namesAndStatesAgree'),
    'replays': 0,
    'verifiedFiles': {str(p.relative_to(root)): hashlib.sha256(p.read_bytes()).hexdigest()
                      for p in sorted(root.rglob('*')) if p.is_file()},
}
print(json.dumps(evidence, indent=2))

"""Remote-only, unmodified expensive test; no cache keys or replay eligibility."""
import collections
import hashlib
import json
import os
import pathlib
import shutil
import subprocess
import sys
import time

project, out = [pathlib.Path(p).resolve() for p in sys.argv[1:]]
assert not out.exists(), 'diagnostic output must be fresh'
out.mkdir(parents=True)
node = shutil.which('node')
assert node
target = 'packages/convex-helpers/server/stream.property.test.ts'
source_hash = hashlib.sha256((project / target).read_bytes()).hexdigest()
assert source_hash == '9798bab303233bca846a89ccb39ce28bd968329e0704f3e5d67adca934c11c75'
assert subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=project, text=True).strip() == '5eaa7e5160736054d53f8b71e2c522835eb427e0'
preload = pathlib.Path(__file__).with_name('stat-constructor-preload.mjs')
reference, phases = None, []
for phase in ['plain-before', 'observed', 'plain-after']:
    env = dict(os.environ)
    for key in ['NODE_OPTIONS', 'LD_PRELOAD', 'LD_AUDIT', 'CURSOR_TEST_SEED', 'CURSOR_TEST_CASES']:
        assert not env.get(key), 'unexpected ambient override: ' + key
    report = out / (phase + '-tests.json')
    if phase == 'observed':
        env.update(NODE_OPTIONS='--import=' + str(preload),
                   QUOD_STAT_CONSTRUCTOR_REPORT=str(out / 'constructor.json'))
    command = [node, 'node_modules/vitest/vitest.mjs', 'run', target,
               '--reporter=default', '--reporter=json', '--outputFile.json=' + str(report)]
    start = time.monotonic()
    with (out / (phase + '.log')).open('w') as log:
        completed = subprocess.run(command, cwd=project, env=env, stdout=log, stderr=subprocess.STDOUT)
    seconds = time.monotonic() - start
    row = {'phase': phase, 'seconds': seconds, 'exit': completed.returncode}
    phases.append(row)
    (out / 'times.json').write_text(json.dumps(phases, indent=2) + '\n')
    assert completed.returncode == 0, phase + ' failed; see its log'
    result = json.loads(report.read_text())
    files = result['testResults']
    assert len(files) == 1 and os.path.relpath(files[0]['name'], project) == target
    states = sorted([a['fullName'], a['status']] for a in files[0]['assertionResults'])
    assert len(states) == 47 and all(s[1] == 'passed' for s in states)
    if reference is None:
        reference = states
    assert states == reference, phase + ' changed test names or states'
    row.update(tests=len(states), reportSha256=hashlib.sha256(report.read_bytes()).hexdigest())
    print(json.dumps(row), flush=True)

observed = json.loads((out / 'constructor.json').read_text())
assert observed['pauses'] > 0 and observed['rows'], 'constructor observation did not run'
assert not observed['failures'], 'constructor instrumentation failed'
assert observed['replayEligible'] is False and observed['completeOwnership'] is False
fields = collections.Counter(f for row in observed['rows'] for f in row['fields'])
queries = collections.Counter(row['query']['function'] if row['query'] else 'unmapped' for row in observed['rows'])
summary = {
    'scope': 'main-process metadata observation during the unchanged expensive test; no replay',
    'projectCommit': '5eaa7e5160736054d53f8b71e2c522835eb427e0',
    'sourceSha256': source_hash, 'node': subprocess.check_output([node, '--version'], text=True).strip(),
    'image': os.environ.get('ImageOS', '') + '/' + os.environ.get('ImageVersion', ''),
    'phases': phases, 'statesMatch': True, 'replayEligible': False, 'replayed': 0,
    'pauses': observed['pauses'], 'rows': len(observed['rows']),
    'rejections': observed['rejections'], 'queries': dict(queries), 'fields': dict(fields),
    'intact': dict(collections.Counter(str(row['intact']) for row in observed['rows'])),
    'limitations': observed['limitations'] + ['Fixed phase order on one runner; no fresh-runner or whole-job savings claim'],
}
(out / 'result.json').write_text(json.dumps(summary, indent=2) + '\n')
print(json.dumps(summary), flush=True)

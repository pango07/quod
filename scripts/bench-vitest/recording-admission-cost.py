"""Remote-only cost probe on unchanged convex-helpers; no replay claim.

The small release fixture proves natural admission transitions. This probe
primes ONLY the economic hint to measure the actual backoff path without
spending three additional full-suite recordings learning the same condition.
It must not be reported as a naturally warmed or cold-window benchmark.
"""
import collections
import hashlib
import json
import os
import pathlib
import shutil
import subprocess
import sys
import time

project, binary, base = [pathlib.Path(p).resolve() for p in sys.argv[1:]]
assert sys.platform == 'linux' and not base.exists()
out = base / 'evidence'
out.mkdir(parents=True)
node = shutil.which('node')
assert node
state = base / 'state'
ledger_path = state / 'ledger.json'
env = dict(os.environ, QUOD_VITEST_RECORDING='adaptive', QUOD_VITEST_LINEAGE='0',
           QUOD_VITEST_TIMING='off', QUOD_VITEST_NAMES_PROOF='0', QUOD_VITEST_BOOLEAN_PROOF='0')
assert not env.get('QUOD_VITEST_JS_DIR') and not env.get('QUOD_VITEST_DISABLE')
sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
result = {
    'scope': 'Vitest command costs, one runner, fixed phase order, primed economic hint',
    'primedHint': True, 'productionReplayEnabled': False,
    'projectCommit': subprocess.check_output(['git', '-C', str(project), 'rev-parse', 'HEAD'], text=True).strip(),
    'sourceCommit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
    'binarySha256': sha(binary), 'node': subprocess.check_output([node, '--version'], text=True).strip(),
    'image': {k: os.environ.get(k) for k in ['ImageOS', 'ImageVersion']}, 'phases': []}
reference = None
proof_before = None
for phase in ['plain-before', 'recorded', 'adaptive-plain', 'plain-after']:
    report, summary = out / (phase + '-report.json'), out / (phase + '-summary.json')
    args = ['run', '--reporter=default', '--reporter=json', '--outputFile.json=' + str(report)]
    if phase == 'adaptive-plain':
        ledger = json.loads(ledger_path.read_text())
        # Reuse the existing metadata-only diagnostic, not a source edit.
        paths = sorted(set(r['file'] for r in ledger['files'].values()))
        mutated = []
        for name in paths:
            file = project / name
            before = sha(file)
            stat = file.stat()
            os.utime(file, ns=(stat.st_atime_ns, stat.st_mtime_ns + 60_000_000_000))
            assert sha(file) == before
            mutated.append({'path': name, 'sha256': before})
        assert mutated
        ledger['recording_admission'] = {'shared': 'diagnostic-previous-shared-input', 'misses': 2, 'skipped': 0}
        ledger_path.write_text(json.dumps(ledger))
        proof_before = {k: v for k, v in ledger.items() if k != 'recording_admission'}
        (out / 'proofs-before.json').write_text(json.dumps(proof_before, sort_keys=True))
        result['metadataOnlyMutations'] = mutated
    if phase.startswith('plain-'):
        cmd = [node, 'node_modules/vitest/vitest.mjs', *args]
    else:
        cmd = [str(binary), 'vitest', '--dir', str(state), '--no-fast', '--no-narrow',
               '--audits', '0', '--summary', str(summary), '--', *args]
    start = time.monotonic()
    with (out / (phase + '.log')).open('w') as log:
        run = subprocess.run(cmd, cwd=project, env=env, stdout=log, stderr=subprocess.STDOUT, timeout=180)
    seconds = time.monotonic() - start
    assert run.returncode == 0, phase + ': inspect retained log'
    data = json.loads(report.read_text())
    states, statuses = [], collections.Counter()
    for file in data['testResults']:
        tests = sorted([t['fullName'], t['status']] for t in file['assertionResults'])
        statuses.update(t[1] for t in tests)
        states.append([os.path.relpath(file['name'], project), tests])
    states.sort()
    assert len(states) == 120 and sum(statuses.values()) == 4131
    assert dict(statuses) == {'passed': 4130, 'skipped': 1}
    if reference is None:
        reference = states
    assert states == reference, phase + ': test names, multiplicity or states differ'
    row = {'phase': phase, 'seconds': seconds, 'files': len(states), 'states': sum(statuses.values()),
           'statuses': dict(statuses), 'reportSha256': sha(report)}
    if not phase.startswith('plain-'):
        s = json.loads(summary.read_text())
        assert s['replayed'] == 0 and s['ran'] == 120
        assert s['recording_policy'] == ('adaptive-plain' if phase == 'adaptive-plain' else 'record')
        if phase == 'adaptive-plain':
            assert s['shared_inputs_changed'] is True
        row['summary'] = s
    if phase == 'adaptive-plain':
        after = json.loads(ledger_path.read_text())
        proof_after = {k: v for k, v in after.items() if k != 'recording_admission'}
        assert proof_after == proof_before
        (out / 'proofs-after.json').write_text(json.dumps(proof_after, sort_keys=True))
        result['proofsPreserved'] = True
    result['phases'].append(row)
    (out / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({k: v for k, v in row.items() if k != 'summary'}), flush=True)
subprocess.run(['git', '-C', str(project), 'diff', '--exit-code', '--quiet'], check=True)
result.update(wrong=0, replayed=0, sourceContentsUnchanged=True,
              limitations=['Primed admission hint; not a natural commit window', 'One runner and fixed order',
                           'No GitHub job, install, Cloud or ledger-upload costs included'])
(out / 'result.json').write_text(json.dumps(result, indent=2) + '\n')

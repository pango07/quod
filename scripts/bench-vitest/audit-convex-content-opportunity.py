#!/usr/bin/env python3
"""Read-only content opportunity audit; never emits a replay eligibility decision.

Usage: SCRIPT PUBLIC_GIT_CLONE WORKER_INPUTS_JSON OUTPUT_JSON
Requires the recorded public Git objects, not an install, build or test run.
"""
import hashlib
import json
import math
import pathlib
import subprocess
import sys

repo, worker_path, output_path = map(pathlib.Path, sys.argv[1:])
root = pathlib.Path(__file__).resolve().parents[2]
window_path = root / 'docs/bench/convex-helpers-current-cloud-window.json'
origin_path = root / 'docs/bench/worker-module-bytes.json'
window = json.loads(window_path.read_text())
origin = json.loads(origin_path.read_text())
digest = lambda b: hashlib.sha256(b).hexdigest()
assert digest(worker_path.read_bytes()) == origin['artifactSha256']['before-inputs.json']
workers = json.loads(worker_path.read_text())
assert workers['replayEligible'] is False
plain = sorted((r for r in window['per'] if r['variant'] == 'plain'), key=lambda r: r['i'])
quod = {r['i']: r for r in window['per'] if r['variant'] == 'quod'}
assert [r['i'] for r in plain] == list(range(12))
test = 'packages/convex-helpers/server/stream.property.test.ts'


def git(*args):
    return subprocess.check_output(['git', '-C', str(repo), *args])


def tree(commit):
    result = {}
    for row in git('ls-tree', '-rz', commit).split(b'\0'):
        if not row:
            continue
        fields, path = row.split(b'\t', 1)
        mode, kind, oid = fields.decode().split()
        assert kind == 'blob', 'unexpected non-file tree entry'
        result[path.decode()] = {'mode': mode, 'oid': oid}
    return result


trees = [tree(r['commit']) for r in plain]
selected = [w for w in workers['workers'] if any(
    f.endswith('/' + test) for r in w['runs'] for f in r['files'])]
assert len(selected) == 1
worker = selected[0]
assert worker['errors'] == [] and worker['pending'] == 0
file = next(f for r in worker['runs'] for f in r['files'] if f.endswith('/' + test))
prefix = file[:-len(test)]
# This exactly reproduces fingerprint(string)'s encoding in worker-input-probe.mjs.
fingerprint = lambda s: digest(json.dumps(['string', s], separators=(',', ':'), ensure_ascii=False).encode())
by_hash = {fingerprint(prefix + path): path for path in trees[-1]}
observed = []
external = []
for request in worker['requests']:
    for module in request.get('response', {}).get('modules', []):
        if module['kind'] == 'external':
            external.append(module['fields'])
            continue
        assert module['kind'] == 'transformed' and module['available']
        field = module['fields']['file']
        assert field['available'] and field['hash'] in by_hash
        assert module['body']['available']
        observed.append({'path': by_hash[field['hash']], 'body': module['body']})
paths = sorted({m['path'] for m in observed})
assert test in paths

# Deliberately coarse, explicit comparison set. It is not an input certificate.
all_paths = set().union(*(t.keys() for t in trees))
coarse = sorted(p for p in all_paths if (
    pathlib.PurePosixPath(p).name in {'package.json', 'package-lock.json', 'tsconfig.json'}
    or 'vitest.config.' in p or 'vite.config.' in p or p == 'scripts/zodVersions.mjs'))
rows = []
previous_manifest = previous_lock = None
for i, (r, t) in enumerate(zip(plain, trees)):
    manifest = json.loads(git('show', r['commit'] + ':package.json'))
    lock = json.loads(git('show', r['commit'] + ':package-lock.json'))['packages']
    row = {'i': i, 'commit': r['commit'], 'observedSourceBlobs': {p: t[p] for p in paths},
           'coarseConfigurationBlobs': {p: t.get(p) for p in coarse},
           'plainTestStepSeconds': r['steps']['test'], 'quodTestStepSeconds': quod[i]['steps']['test']}
    if i:
        previous = trees[i-1]
        changed = sorted(p for p in set(t) | set(previous) if t.get(p) != previous.get(p))
        source_changes = [p for p in paths if p in changed]
        config_changes = [p for p in coarse if p in changed]
        row.update({'changedFiles': changed, 'observedSourceChanges': source_changes,
                    'coarseConfigurationChanges': config_changes,
                    'coarseContentUnchanged': not source_changes and not config_changes,
                    'manifestTopLevelChanges': sorted(k for k in set(manifest) | set(previous_manifest)
                        if manifest.get(k) != previous_manifest.get(k)),
                    'lockPackageEntryChanges': sorted(k for k in set(lock) | set(previous_lock)
                        if lock.get(k) != previous_lock.get(k))})
    previous_manifest, previous_lock = manifest, lock
    rows.append(row)

unchanged = [r for r in rows[1:] if r['coarseContentUnchanged']]
q_seconds = window['summary']['quod']['window']['runnerSeconds']
p_seconds = window['summary']['plain']['window']['runnerSeconds']
removed = sum(r['quodTestStepSeconds'] for r in unchanged)
unchanged_indices = {r['i'] for r in unchanged}
modeled_minutes = sum(math.ceil((r['runnerSeconds'] - (
    r['steps']['test'] if i in unchanged_indices else 0)) / 60) for i, r in quod.items())
result = {
    'replayEligible': False,
    'kind': 'historical-content-opportunity-analysis',
    'windowRun': 'https://github.com/pango07/ci-bench/actions/runs/' + str(window['coordinator']),
    'workerRun': origin['run'], 'workerSourceCommit': origin['commit'],
    'evidenceSha256': {'window': digest(window_path.read_bytes()), 'workerInputs': digest(worker_path.read_bytes())},
    'repository': 'https://github.com/get-convex/convex-helpers', 'test': test,
    'observedTransformedModules': observed, 'externalModuleRecordsNotExpanded': external,
    'summary': {
        'trackedSourcePaths': len(paths), 'unchangedObservedSourceTransitions': sum(not r['observedSourceChanges'] for r in rows[1:]),
        'coarseContentUnchangedCommits': [r['i'] for r in unchanged],
        'coarseContentUnchangedPlainTestStepSeconds': sum(r['plainTestStepSeconds'] for r in unchanged),
        'coarseContentUnchangedQuodTestStepSeconds': removed,
        'arithmeticOnlyRemovingEntireQuodTestSteps': {
            'remainingRunnerSeconds': q_seconds - removed,
            'ratioToPlain': (q_seconds - removed) / p_seconds,
            'furtherSecondsToHalfPlain': q_seconds - removed - p_seconds / 2,
            'modeledMinutes': modeled_minutes,
            'modeledMinutesRatioToPlain': modeled_minutes / window['summary']['plain']['window']['modeledMinutes'],
        },
    },
    'commits': rows,
    'limitations': [
        'The five tracked modules are observed at c11, not a complete dependency closure for all commits.',
        'External package internals, dynamic resolution, configuration consumption and non-code inputs are not certified.',
        'Equal Git blobs do not imply equal transforms, runtime configuration, filesystem metadata or test outcomes.',
        'The coarse comparison set is an analysis scenario, not an implemented or sound replay key.',
        'Removing entire measured test steps is an arithmetic scenario with all other timings held fixed, not measured savings or a performance bound.',
        'No trust warm-up, audits, validation, transfer or changed runtime behavior is modeled.',
        'No package.json field is excluded from any production unit key.',
    ],
}
output_path.write_text(json.dumps(result, indent=2) + '\n')
print(json.dumps(result['summary'], indent=2))

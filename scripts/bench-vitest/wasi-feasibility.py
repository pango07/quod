"""Remote-only feasibility probe; no narrowed inputs or reusable recordings.

Compare the locked native and WASI bundlers and the current recorded path.
All test reports, selected binding observations and durations are evidence;
none authorizes replay. Native controls bracket the two WASI phases.
"""
import collections
import hashlib
import json
import os
import pathlib
import shutil
import signal
import subprocess
import sys
import time

project, binary, packages, base = [pathlib.Path(x).resolve() for x in sys.argv[1:]]
assert not base.exists(), 'feasibility output must be fresh'
out = base / 'evidence'
out.mkdir(parents=True)
node = shutil.which('node')
assert node
observer = pathlib.Path(__file__).with_name('wasi-load-observer.cjs').resolve()
lock = json.loads((packages / 'package-lock.json').read_text())
wanted = {
    '@rolldown/binding-wasm32-wasi': '1.2.2', '@napi-rs/wasm-runtime': '1.2.2',
    '@emnapi/core': '2.0.0-alpha.3', '@emnapi/runtime': '2.0.0-alpha.3',
    '@emnapi/wasi-threads': '2.0.1', '@tybys/wasm-util': '0.10.3', 'tslib': '2.8.1',
}
installed = {key.removeprefix('node_modules/'): value for key, value in lock['packages'].items() if key}
assert {k: v['version'] for k, v in installed.items()} == wanted
assert installed['@rolldown/binding-wasm32-wasi']['integrity'] == 'sha512-lk8MaaL90mgObXaMtxEy7DbFLYBSkcyunn/s6bx3sCuAU+XW+DFMXAek+AUPrVaW3qfKClPptKpNH2gnfExkug=='
binding = packages / 'node_modules/@rolldown/binding-wasm32-wasi'
link = project / 'node_modules/@rolldown/binding-wasm32-wasi'
assert not link.exists(), 'do not replace an existing project binding'
assert json.loads((project / 'node_modules/rolldown/package.json').read_text())['version'] == '1.2.2'
link.symlink_to(binding, target_is_directory=True)
shutil.copyfile(packages / 'package-lock.json', out / 'wasi-package-lock.json')

def states(file):
    report = json.loads(file.read_text())
    assert isinstance(report.get('testResults'), list) and report['testResults']
    rows, statuses = [], collections.Counter()
    for module in report['testResults']:
        tests = []
        for test in module['assertionResults']:
            name, status = test['fullName'], test['status']
            assert isinstance(name, str) and isinstance(status, str)
            tests.append([name, status]); statuses[status] += 1
        rows.append([os.path.relpath(module['name'], project), module['status'], sorted(tests)])
    return sorted(rows), dict(statuses)

results, reference = [], None
for phase in ['native-before', 'plain-wasi', 'recorded-wasi', 'native-after']:
    report = out / (phase + '-report.json')
    observations = out / (phase + '-loads')
    observations.mkdir()
    env = dict(os.environ)
    for key in ['NODE_OPTIONS', 'LD_PRELOAD', 'LD_AUDIT', 'QUOD_VITEST_JS_DIR',
                'QUOD_VITEST_DISABLE', 'NAPI_RS_NATIVE_LIBRARY_PATH', 'NAPI_RS_FORCE_WASI', 'NAPI_RS_WASI_FLAVOR']:
        assert not env.get(key), f'unexpected ambient override: {key}'
    env.update(NODE_OPTIONS='--require=' + str(observer), QUOD_WASI_LOAD_REPORT=str(observations))
    wasi = phase.endswith('wasi')
    if wasi:
        env['NAPI_RS_FORCE_WASI'] = 'error'
    args = ['run', '--reporter=default', '--reporter=json', '--outputFile.json=' + str(report)]
    if phase == 'recorded-wasi':
        env.update(QUOD_VITEST_LINEAGE='1', QUOD_VITEST_TIMING='off',
                   QUOD_VITEST_NAMES_PROOF='1', QUOD_VITEST_BOOLEAN_PROOF='1')
        command = [str(binary), 'vitest', '--dir', str(base / 'state'),
                   '--summary', str(out / 'quod-summary.json'), '--full', '--', *args]
    else:
        command = [node, 'node_modules/vitest/vitest.mjs', *args]
    start = time.monotonic()
    with (out / (phase + '.log')).open('w') as log:
        process = subprocess.Popen(command, cwd=project, env=env, stdout=log,
                                   stderr=subprocess.STDOUT, start_new_session=True)
        try:
            status = process.wait(timeout=180)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait()
            raise AssertionError(f'{phase} exceeded the 180-second feasibility budget')
    elapsed = time.monotonic() - start
    row = {'phase': phase, 'seconds': elapsed, 'exit': status}
    results.append(row)
    (out / 'times.json').write_text(json.dumps(results, indent=2) + '\n')
    assert status == 0, f'{phase} failed; see its log'
    rows, statuses = states(report)
    assert len(rows) == 120 and sum(len(r[2]) for r in rows) == 4131
    assert statuses == {'passed': 4130, 'skipped': 1}
    if reference is None: reference = rows
    assert rows == reference, f'{phase} test states differ'
    loads = [json.loads(p.read_text()) for p in observations.glob('*.json')]
    assert loads, 'missing load observations'
    native = [e for r in loads for e in r['events']]
    initial = [p for r in loads for p in r['initial']]
    cached = [p for r in loads for p in r['nativeModules']]
    wasi_modules = sorted({p for r in loads for p in r['wasiModules']})
    if wasi:
        assert not native and not initial and not cached, 'WASI phase still observed a native addon'
        assert wasi_modules, 'strict WASI binding was not observed'
    else:
        assert any('rolldown' in e['path'] and e['completed'] for e in native), 'native control did not observe the locked binding'
        assert not wasi_modules, 'native control selected WASI'
    row.update(files=len(rows), states=4131, statuses=statuses, observedProcesses=len(loads),
               nativeLoads=len(native), wasiModules=wasi_modules,
               reportSha256=hashlib.sha256(report.read_bytes()).hexdigest())
    (out / 'times.json').write_text(json.dumps(results, indent=2) + '\n')
    print(json.dumps(row), flush=True)

summary = json.loads((out / 'quod-summary.json').read_text())
assert summary['replayed'] == 0
ledger = json.loads((base / 'state/ledger.json').read_text())
result = {
    'scope': 'WASI compatibility and observed addon loads; no replay or job-savings claim',
    'replayEligible': False, 'replayed': 0, 'semanticMatches': True,
    'binarySha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
    'projectCommit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=project, text=True).strip(),
    'node': subprocess.check_output([node, '--version'], text=True).strip(),
    'image': os.environ.get('ImageOS', '') + '/' + os.environ.get('ImageVersion', ''),
    'phases': results, 'packages': installed,
    'wasmFiles': [{'path': str(p.relative_to(packages)), 'bytes': p.stat().st_size,
                   'sha256': hashlib.sha256(p.read_bytes()).hexdigest()} for p in binding.glob('*.wasm')],
    'metadataKeys': {kind: sum(k.startswith(kind + ':') for k in ledger['keys']) for kind in ['sm', 'sl', 'st']},
    'limitations': ['Four fixed-order phases on one runner', 'Load observer is not complete native provenance',
                    'WASI host imports and filesystem access still require an independent capability argument',
                    'No metadata keys were removed and no reusable ledger was published'],
}
(out / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
print(json.dumps({k: v for k, v in result.items() if k != 'packages'}), flush=True)

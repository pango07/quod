"""Short diagnostic only: disabled observers never produce reusable records."""
import json
import os
import pathlib
import subprocess
import sys
import time

project, mode = sys.argv[1:]
base = pathlib.Path('/tmp/quod-profile')
out = base / 'evidence'
out.mkdir(parents=True, exist_ok=True)
files = subprocess.check_output(['git', 'ls-files', '*stream.property.test.ts'], cwd=project, text=True).splitlines()
assert len(files) == 1, files
filters = [] if mode in ['full', 'caller', 'native'] else files
disabled = {'native': '', 'caller': '', 'full': '', 'all': '', 'preload': 'fs,env,loader,escapes,main', 'env': 'fs,loader,escapes,main,preload'}[mode]
rows = []
source_js = pathlib.Path(__file__).resolve().parents[2] / 'crates/quod-cli/src/vitest'
if mode == 'native':
    import shutil
    native_js = base / 'native-js'
    shutil.copytree(source_js, native_js)
    probe = pathlib.Path(__file__).with_name('native-stack-probe.mjs').read_text().replace('export function nativeCaller', 'function nativeCaller')
    recorder = native_js / 'record.mjs'
    text = recorder.read_text()
    assert text.count('function toolCaller(npm = false) {') == 1
    assert text.count('export function patchEnv(r) {') == 1
    text = text.replace('function toolCaller(npm = false) {', 'function toolCaller(npm = false) {\n  return nativeCaller(npm, SELF, HELPERS, RESOLVERS, CHILDREN, NPM_HELPERS);\n')
    text = text.replace('export function patchEnv(r) {', "export function patchEnv(r) {\n  r.unobservable.add('native-stack attribution diagnostic');")
    recorder.write_text(probe + '\n' + text)
if mode == 'full':
    import shlex
    node = subprocess.check_output(['which', 'node'], text=True).strip()
    (out / 'cpu').mkdir(exist_ok=True)
    wrapper = base / 'profile-node'
    wrapper.write_text('#!/bin/sh\nexec ' + shlex.quote(node) + ' --cpu-prof --cpu-prof-dir=' + shlex.quote(str(out / 'cpu')) + ' "$@"\n')
    wrapper.chmod(0o755)
for name in (['uncached', 'cached'] if mode == 'caller' else ['current', 'native'] if mode == 'native' else ['plain', mode]):
    env = dict(os.environ)
    if name == 'plain':
        cmd = ['node', 'node_modules/vitest/vitest.mjs', 'run', *filters]
    else:
        env['QUOD_VITEST_DISABLE'] = disabled
        env['QUOD_VITEST_JS_DIR'] = str(native_js if mode == 'native' and name == 'native' else source_js)
        env['QUOD_VITEST_CALLER_CACHE'] = '0' if name == 'uncached' else '1'
        cmd = [str(base / 'quod'), 'vitest', *(['--node', str(wrapper)] if mode == 'full' else []), '--dir', str(base / ('state-' + name)), '--report', str(out / (name + '-report.json')), '--full', '--', 'run', *filters]
    start = time.monotonic()
    with (out / (name + '.log')).open('w') as log:
        result = subprocess.run(cmd, cwd=project, env=env, stdout=log, stderr=subprocess.STDOUT)
    rows.append(dict(mode=name, seconds=time.monotonic() - start, exit=result.returncode))
    print(json.dumps(rows[-1]), flush=True)
(out / 'times.json').write_text(json.dumps(rows, indent=2))

if mode in ['caller', 'native']:
    def states(name):
        report = json.loads((out / (name + '-report.json')).read_text())
        return sorted((f['name'], a.get('fullName', a.get('title')), a['status'])
                      for f in report['testResults'] for a in f['assertionResults'])
    first, second = ('uncached', 'cached') if mode == 'caller' else ('current', 'native')
    a, b = states(first), states(second)
    assert a and a == b, 'attribution per-test results differed'
    assert all(r['exit'] == 0 for r in rows), 'a diagnostic run failed'
    print(f'{mode} attribution: {len(a)} per-test states agree', flush=True)

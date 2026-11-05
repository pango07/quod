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
filters = [] if mode == 'full' else files
disabled = {'full': '', 'all': '', 'preload': 'fs,env,loader,escapes,main', 'env': 'fs,loader,escapes,main,preload'}[mode]
rows = []
if mode == 'full':
    import shlex
    node = subprocess.check_output(['which', 'node'], text=True).strip()
    (out / 'cpu').mkdir(exist_ok=True)
    wrapper = base / 'profile-node'
    wrapper.write_text('#!/bin/sh\nexec ' + shlex.quote(node) + ' --cpu-prof --cpu-prof-dir=' + shlex.quote(str(out / 'cpu')) + ' "$@"\n')
    wrapper.chmod(0o755)
for name in ['plain', mode]:
    env = dict(os.environ)
    if name == 'plain':
        cmd = ['node', 'node_modules/vitest/vitest.mjs', 'run', *filters]
    else:
        env['QUOD_VITEST_DISABLE'] = disabled
        cmd = [str(base / 'quod'), 'vitest', *(['--node', str(wrapper)] if mode == 'full' else []), '--dir', str(base / 'state'), '--full', '--', 'run', *filters]
    start = time.monotonic()
    with (out / (name + '.log')).open('w') as log:
        result = subprocess.run(cmd, cwd=project, env=env, stdout=log, stderr=subprocess.STDOUT)
    rows.append(dict(mode=name, seconds=time.monotonic() - start, exit=result.returncode))
    print(json.dumps(rows[-1]), flush=True)
(out / 'times.json').write_text(json.dumps(rows, indent=2))

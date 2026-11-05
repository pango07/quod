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
disabled = {'all': '', 'preload': 'fs,env,loader,escapes,main', 'env': 'fs,loader,escapes,main,preload'}[mode]
rows = []
for name in ['plain', mode]:
    env = dict(os.environ)
    if name == 'plain':
        cmd = ['node', 'node_modules/vitest/vitest.mjs', 'run', files[0]]
    else:
        env['QUOD_VITEST_DISABLE'] = disabled
        cmd = [str(base / 'quod'), 'vitest', '--dir', str(base / 'state'), '--full', '--', 'run', files[0]]
    start = time.monotonic()
    with (out / (name + '.log')).open('w') as log:
        result = subprocess.run(cmd, cwd=project, env=env, stdout=log, stderr=subprocess.STDOUT)
    rows.append(dict(mode=name, seconds=time.monotonic() - start, exit=result.returncode))
    print(json.dumps(rows[-1]), flush=True)
(out / 'times.json').write_text(json.dumps(rows, indent=2))

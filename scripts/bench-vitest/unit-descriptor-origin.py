#!/usr/bin/env python3
"""Preserve actual unit traces for two public npm scripts; diagnostic only."""
import hashlib
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile

project, binary, output = [pathlib.Path(p).resolve() for p in sys.argv[1:]]
assert sys.platform == 'linux'
output.mkdir(parents=True, exist_ok=True)
base = pathlib.Path(tempfile.mkdtemp(prefix='.unit-descriptor-origin-', dir=pathlib.Path.cwd()))
wrappers = base / 'bin'
wrappers.mkdir()
traces = base / 'traces'
traces.mkdir()
real_strace = shutil.which('strace')
node = shutil.which('node')
npm = pathlib.Path(shutil.which('npm')).resolve()
assert real_strace and node
# This wrapper is outside the traced command. Preserve the trace after strace
# exits and before the unmodified Quod binary parses and deletes its original.
wrapper = '''#!/usr/bin/python3
import os,pathlib,shutil,subprocess,sys
args=sys.argv[1:]
r=subprocess.run([REAL_STRACE,*args])
if '-o' in args:
 p=pathlib.Path(args[args.index('-o')+1])
 if p.is_file(): shutil.copyfile(p,pathlib.Path(TRACE_DIR)/('trace-'+str(os.getpid())+'.log'))
sys.exit(r.returncode)
'''.replace('REAL_STRACE', repr(real_strace)).replace('TRACE_DIR', repr(str(traces)))
(wrappers / 'strace').write_text(wrapper)
(wrappers / 'strace').chmod(0o755)
env = dict(os.environ, PATH=str(wrappers) + ':' + os.environ['PATH'], QUOD_DISABLE='1')
sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
result = {'replayEligible': False, 'projectCommit': subprocess.check_output(
    ['git', '-C', str(project), 'rev-parse', 'HEAD'], text=True).strip(),
    'binarySha256': sha(binary), 'node': subprocess.check_output([node, '--version'], text=True).strip(),
    'strace': subprocess.check_output([real_strace, '-V'], text=True).splitlines()[0],
    'image': {k: os.environ.get(k) for k in ['ImageOS', 'ImageVersion']}, 'units': []}
for script in ['build', 'test:release']:
    for sample in [1, 2]:
        name = script.replace(':', '-') + '-' + str(sample)
        before = set(traces.iterdir())
        cmd = [str(binary), 'unit', '--dir', str(base / ('ledger-' + name)),
               '--root', str(project), '--', node, str(npm), 'run', script]
        run = subprocess.run(cmd, cwd=project, env=env, text=True, capture_output=True, timeout=90)
        (output / (name + '.stdout')).write_text(run.stdout)
        (output / (name + '.stderr')).write_text(run.stderr)
        created = set(traces.iterdir()) - before
        assert run.returncode == 0, name + ': unit failed; inspect retained logs'
        assert len(created) == 1, name + ': expected one actual first-witness trace'
        trace = next(iter(created))
        target = output / (name + '.trace')
        shutil.copyfile(trace, target)
        text = target.read_text()
        # Retain raw evidence too. This filter only inventories descriptor
        # queries; it is not a replacement for the production trace parser.
        metadata = [line for line in text.splitlines() if re.search(
            r'\b(?:fstat|fstat64)\(|\b(?:newfstatat|fstatat64)\([^,]+, "",', line)]
        anonymous = [line for line in metadata if re.search(
            r'\b(?:fstat|fstat64|newfstatat|fstatat64)\(\d+<(?!/)', line)]
        undecoded = [line for line in metadata if re.search(
            r'\b(?:fstat|fstat64|newfstatat|fstatat64)\(\d+,', line)]
        unit = {'script': script, 'sample': sample, 'exit': run.returncode,
                'quodLines': [line for line in run.stderr.splitlines() if line.startswith('quod:')],
                'traceSha256': sha(target), 'traceBytes': target.stat().st_size,
                'descriptorMetadataCalls': len(metadata),
                'anonymousDescriptorCalls': anonymous, 'undecodedDescriptorCalls': undecoded}
        result['units'].append(unit)
        (output / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
        print(json.dumps({'unit': name, 'descriptorCalls': len(metadata),
            'anonymous': len(anonymous), 'undecoded': len(undecoded), 'quod': unit['quodLines']}), flush=True)
assert len(result['units']) == 4

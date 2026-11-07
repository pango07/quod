#!/usr/bin/env python3
"""Bounded native-caller diagnostic, deliberately ineligible for recording.

Run each real unit once with its normal Quod wrapper and stdio. Narrow
strace to metadata syscalls to bound unwinding cost. Preserve that incomplete
trace, then empty Quod's copy so the production parser MUST reject it.
"""
import collections
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
base = pathlib.Path(tempfile.mkdtemp(prefix='.unit-descriptor-stacks-', dir=pathlib.Path.cwd()))
wrappers = base / 'bin'
wrappers.mkdir()
traces = base / 'traces'
traces.mkdir()
real_strace = shutil.which('strace')
node = shutil.which('node')
npm = pathlib.Path(shutil.which('npm')).resolve()
assert real_strace and node
wrapper = '''#!/usr/bin/python3
import json,os,pathlib,shutil,subprocess,sys,tempfile
args=sys.argv[1:]
if '-o' not in args:
 sys.exit(subprocess.run([REAL_STRACE,*args]).returncode)
original=list(args)
index=next(i for i,a in enumerate(args) if a.startswith('trace='))
assert args[index-1]=='-e'
args[index]='trace=fstat,newfstatat,statx,io_uring_setup'
args=['-k','--stack-trace-frame-limit=8',*args]
p=pathlib.Path(args[args.index('-o')+1])
r=subprocess.run([REAL_STRACE,*args])
try:
 fd,dest=tempfile.mkstemp(prefix='trace-',suffix='.log',dir=TRACE_DIR)
 os.close(fd)
 shutil.copyfile(p,dest)
 pathlib.Path(dest+'.json').write_text(json.dumps({
  'replayEligible':False,'completeTrace':False,'exit':r.returncode,
  'originalArgs':original,'diagnosticArgs':args}))
finally:
 # A partial trace is never evidence of a complete input set. The existing
 # parser explicitly rejects empty text before it can create any record.
 p.write_text('')
sys.exit(r.returncode)
'''.replace('REAL_STRACE', repr(real_strace)).replace('TRACE_DIR', repr(str(traces)))
(wrappers / 'strace').write_text(wrapper)
(wrappers / 'strace').chmod(0o755)
env = dict(os.environ, PATH=str(wrappers) + ':' + os.environ['PATH'], QUOD_DISABLE='1')
sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
result = {
    'replayEligible': False, 'completeTrace': False, 'frameLimit': 8,
    'projectCommit': subprocess.check_output(
        ['git', '-C', str(project), 'rev-parse', 'HEAD'], text=True).strip(),
    'engineSource': 'c12a7fe32f4957b6bd1cc09edbe63c7292b1c1d3',
    'binarySha256': sha(binary), 'nodeBinarySha256': sha(pathlib.Path(node).resolve()),
    'node': subprocess.check_output([node, '--version'], text=True).strip(),
    'strace': subprocess.check_output([real_strace, '-V'], text=True).splitlines()[0],
    'image': {k: os.environ.get(k) for k in ['ImageOS', 'ImageVersion']}, 'units': []}
(output / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
for script in ['build', 'test:release']:
    name = script.replace(':', '-')
    before = set(traces.glob('*.log'))
    ledger_dir = base / ('ledger-' + name)
    cmd = [str(binary), 'unit', '--dir', str(ledger_dir),
           '--root', str(project), '--', node, str(npm), 'run', script]
    try:
        run = subprocess.run(cmd, cwd=project, env=env, text=True,
                             capture_output=True, timeout=120)
    except subprocess.TimeoutExpired as error:
        for stream in ['stdout', 'stderr']:
            data = getattr(error, stream) or b''
            (output / (name + '.' + stream)).write_bytes(
                data.encode() if isinstance(data, str) else data)
        for trace in ledger_dir.glob('*/trace.log'):
            shutil.copyfile(trace, output / (name + '.incomplete.trace'))
        raise
    finally:
        # Preserve evidence even if the command times out or fails. The
        # outer GitHub job bounds the whole diagnostic; never retry here.
        for trace in set(traces.glob('*.log')) - before:
            shutil.copyfile(trace, output / (name + '.trace'))
            shutil.copyfile(str(trace) + '.json', output / (name + '.trace.json'))
    (output / (name + '.stdout')).write_text(run.stdout)
    (output / (name + '.stderr')).write_text(run.stderr)
    assert run.returncode == 0, name + ': unit failed; inspect retained logs'
    assert len(set(traces.glob('*.log')) - before) == 1, name + ': expected one trace'
    ledgers = list(ledger_dir.glob('*/ledger.json'))
    assert len(ledgers) == 1, name + ': expected one ledger'
    ledger = json.loads(ledgers[0].read_text())
    assert ledger['always'] == 'its trace is empty', name + ': partial trace was not rejected'
    assert ledger['records'] == [], name + ': diagnostic must never create a record'
    assert 'runs again plain' not in run.stderr, name + ': unexpected fallback execution'
    target = output / (name + '.trace')
    groups = []
    for line in target.read_text().splitlines():
        if re.match(r'^\s*(?:\d+\s+)?\s*>\s+', line):
            if groups:
                groups[-1]['frames'].append(line.strip())
        elif re.match(r'^\d+\s+', line):
            groups.append({'syscall': line, 'frames': []})
    # Inventory only. Resumed/unfinished calls remain verbatim in the raw
    # trace and are counted separately; this does not certify consumption.
    metadata = [g for g in groups if re.search(
        r'\bfstat\(|\bnewfstatat\([^,]+, "",|\bstatx\([^,]+, "",', g['syscall'])]
    anonymous = [g for g in metadata if re.search(
        r'\b(?:fstat|newfstatat|statx)\(\d+<(?!/)', g['syscall'])]
    counts = collections.Counter(tuple(g['frames']) for g in anonymous)
    unit = {
        'script': script, 'exit': run.returncode,
        'quodLines': [line for line in run.stderr.splitlines() if line.startswith('quod:')],
        'recordingRejected': ledger['always'], 'records': len(ledger['records']),
        'traceSha256': sha(target), 'traceBytes': target.stat().st_size,
        'descriptorEntries': len(metadata), 'anonymousEntries': len(anonymous),
        'anonymousWithoutImmediateFrames': sum(not g['frames'] for g in anonymous),
        'unfinishedEntries': sum('<unfinished ...>' in g['syscall'] for g in metadata),
        'anonymousStacks': [{'count': count, 'frames': list(frames),
            'example': next(g['syscall'] for g in anonymous if tuple(g['frames']) == frames)}
            for frames, count in counts.most_common()]}
    result['units'].append(unit)
    (output / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({k: unit[k] for k in ['script', 'exit', 'recordingRejected',
        'records', 'descriptorEntries', 'anonymousEntries',
        'anonymousWithoutImmediateFrames', 'unfinishedEntries']}), flush=True)
assert len(result['units']) == 2

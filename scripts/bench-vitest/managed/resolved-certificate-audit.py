"""Trusted data-only reader; never imports or executes retained artifact code."""
import argparse
import collections
import hashlib
import json
import os
import pathlib
import stat
import subprocess
import sys
import unicodedata
import zipfile

parser = argparse.ArgumentParser(description=__doc__)
for flag in ['archive', 'extracted', 'run', 'jobs', 'artifacts', 'baseline-archive', 'output']:
    parser.add_argument('--' + flag, required=True, type=pathlib.Path)
parser.add_argument('--repository', type=pathlib.Path, default=pathlib.Path(__file__).resolve().parents[3])
args = parser.parse_args()
extraction, repo, archive = args.extracted, args.repository, args.archive
source = '766ff388f1dcc3305d43a14e99655404dc01b596'
base = extraction / '_temp'
probe = base / 'resolved-compile-certificate'
matrix = base / 'resolved-certificate-matrix'
paired = base / 'resolved-certificate-matrix-inputs'
issues = []
sha = lambda b: hashlib.sha256(b).hexdigest()
canon = lambda x: json.dumps(x, sort_keys=True, ensure_ascii=False, separators=(',', ':')).encode()
def regular(p, maximum=16 * 1024 * 1024):
    p = pathlib.Path(p)
    before = p.lstat()
    assert stat.S_ISREG(before.st_mode) and before.st_size <= maximum, 'bounded regular data required: ' + str(p)
    raw = p.read_bytes()
    after = p.lstat()
    assert len(raw) == before.st_size == after.st_size and (before.st_ino, before.st_dev, before.st_mtime_ns, before.st_ctime_ns) == (after.st_ino, after.st_dev, after.st_mtime_ns, after.st_ctime_ns)
    return raw
def unique_object(pairs):
    value = {}
    for key, entry in pairs:
        assert key not in value, 'duplicate JSON field: ' + key
        value[key] = entry
    return value
def parse(raw):
    return json.loads(raw, object_pairs_hook=unique_object, parse_constant=lambda value: (_ for _ in ()).throw(ValueError('non-finite JSON number')))
read = lambda p: parse(regular(p))
source_blobs = {}
def check(name, passed):
    if not passed:
        issues.append(name)
def blob(name):
    if name not in source_blobs:
        assert name in ['compiler.mjs', 'resolved-compile-certificate.mjs']
        source_blobs[name] = subprocess.check_output(['git', 'show', source + ':scripts/bench-vitest/managed/' + name], cwd=repo, env={**os.environ, 'GIT_NO_LAZY_FETCH': '1'})
        assert len(source_blobs[name]) <= 16 * 1024 * 1024
    return source_blobs[name]
def build(directory, name, bundle):
    value = read(directory / name)
    raw = regular(directory / bundle)
    check(str(directory / bundle) + ':bundle', sha(raw) == value['sha256'] == value['identity']['bundleSha256'])
    check(str(directory / name) + ':identity', sha(canon(value['identity'])) == value['identitySha256'])
    return value, raw

run, jobs, artifacts = read(args.run), read(args.jobs), read(args.artifacts)
named_artifacts = [a for a in artifacts['artifacts'] if a['name'] == 'managed-resolved-certificate']
assert len(named_artifacts) == 1, 'ambiguous named artifact'
artifact = named_artifacts[0]
archive_sha = sha(regular(archive, 128 * 1024 * 1024))
assert artifact['id'] == 13591328320 and archive.stat().st_size == 89838252
assert archive_sha == '0b84d4d439353b4d62c19260f612f4aedafb9ca66df66ab1e2a7d81a3542bfd3'
check('API source and terminal first attempt', run['id'] == 42415327673 and run['head_sha'] == source and run['conclusion'] == 'success' and run['run_attempt'] == 1 and run['repository']['full_name'] == 'pango07/quod')
check('API archive exact size/digest', archive.stat().st_size == artifact['size_in_bytes'] and artifact['digest'] == 'sha256:' + archive_sha)
check('API archive belongs to frozen run/source', artifact['workflow_run']['id'] == 42415327673 and artifact['workflow_run']['head_sha'] == source and not artifact['expired'])
job = next(j for j in jobs['jobs'] if j['name'] == 'benchmark-managed-resolved-certificate')
check('exact actual focused job identity', job['id'] == 128532047532 and job['run_id'] == 42415327673 and job['head_sha'] == source)
step_names = ['Retain all original closed-compiler regressions', 'Fresh resolved input equivalence and actual resolution mutations',
              'Exact isolated multi-project managed and native controls', 'Validate all 108 current matrix certificates and retain exact payload sizes']
check('exact real gate steps', job['conclusion'] == 'success' and
      [next(s for s in job['steps'] if s['name'] == name)['conclusion'] for name in step_names] ==
      ['success'] * 4)
archive_rows = []
with zipfile.ZipFile(archive) as z:
    entries = z.infolist()
    assert len(entries) <= 20000 and sum(i.file_size for i in entries) <= 1024 ** 3
    expected_files, expected_dirs, seen = set(), set(), {}
    for info in entries:
        name = info.filename[:-1] if info.is_dir() else info.filename
        p = pathlib.PurePosixPath(name)
        assert name and not p.is_absolute() and not any(part in ('', '.', '..') for part in name.split('/'))
        assert '\\' not in name and '\x00' not in name
        alias = unicodedata.normalize('NFC', name).casefold()
        assert alias not in seen, 'ZIP duplicate/casefold/normalization alias'
        seen[alias] = info.is_dir()
        assert stat.S_IFMT(info.external_attr >> 16) in (0, stat.S_IFDIR if info.is_dir() else stat.S_IFREG)
        assert info.file_size <= 16 * 1024 * 1024 and not info.flag_bits & 1
        if info.is_dir():
            expected_dirs.add(name)
        else:
            expected_files.add(name)
        expected_dirs.update(str(parent) for parent in p.parents if str(parent) != '.')
    directory_aliases = {}
    for name in expected_dirs:
        alias = unicodedata.normalize('NFC', name).casefold()
        assert alias not in seen or seen[alias], 'ZIP file/directory collision'
        assert alias not in directory_aliases or directory_aliases[alias] == name, 'implicit ZIP directory alias'
        directory_aliases[alias] = name
    if not extraction.exists() and not extraction.is_symlink():
        extraction.mkdir(mode=0o700, parents=True)
        z.extractall(extraction)  # Only after exact SHA and complete ZIP inventory checks.
    actual_files, actual_dirs, actual_bytes = set(), set(), 0
    assert stat.S_ISDIR(extraction.lstat().st_mode)
    for directory, dirs, files in os.walk(extraction, followlinks=False):
        for name in dirs:
            p = pathlib.Path(directory) / name
            assert stat.S_ISDIR(p.lstat().st_mode), 'non-directory/symlink in extracted tree'
            actual_dirs.add(p.relative_to(extraction).as_posix())
            assert len(actual_files) + len(actual_dirs) <= 20000
        for name in files:
            p = pathlib.Path(directory) / name
            observed = p.lstat()
            assert stat.S_ISREG(observed.st_mode) and observed.st_size <= 16 * 1024 * 1024, 'special/symlink/large file in extracted tree'
            actual_files.add(p.relative_to(extraction).as_posix())
            actual_bytes += observed.st_size
            assert len(actual_files) + len(actual_dirs) <= 20000 and actual_bytes <= 1024 ** 3
    check('exact extracted file and directory sets', expected_files == actual_files and expected_dirs == actual_dirs)
    for info in entries:
        if info.is_dir():
            continue
        raw = z.read(info)
        check('extracted bytes:' + info.filename, raw == regular(extraction / info.filename))
        archive_rows.append({'name': info.filename, 'bytes': len(raw), 'sha256': sha(raw)})
check('exact expanded bounded retention', len(archive_rows) == 2049 and sum(r['bytes'] for r in archive_rows) == 562638286)
compiler = read(base / 'resolved-certificate-compiler-checks/result.json')
check('55 compiler checks', len(compiler['checks']) == 55 and all(x['passed'] is True for x in compiler['checks']))
result = read(probe / 'result.json')
check('diagnostic never enabled', result['passed'] is True and result['productionEnabled'] is False and result['receiptAdmission'] is False and result['nativePreparationAlwaysRuns'] is True)
check('exact targeted controls', len(result['checks']) == 213 and
      all(x['pass'] is True for x in result['checks']))
expected_environment = {'TEST': 'true', 'VITEST': 'true', 'NODE_ENV': 'test'}
check('bootstrap before loaded compiler and stable preparation', result['bootstrapEnvironment']['afterInitialization'] ==
      result['bootstrapEnvironment']['afterPreparation'] == expected_environment)
primary = {
    '/project/node_modules/typescript/package.json': (841, 'd9b8fb53c67c947fece83bcb73fa8512227ea6a12b110e096fdab8ecdc02b655'),
    '/project/node_modules/typescript/lib/typescript.js': (45, 'd3f3cd2b04b7f466f4484df921b744223f7bd1f3e353ec9110bdf52695b983d5'),
    '/project/node_modules/@typescript/old/package.json': (3527, '9332e97c30d3e53ed54910b89207ed657fb444066484df6e5b6965bf130865e9'),
    '/project/node_modules/@typescript/old/lib/typescript.js': (9144216, '569177652966bd528c319171c7dd22860dbf72bde116cbc4f644f1d02bb12e39'),
}
tool_hashes = set()
def certificate(file, built):
    raw = regular(file)
    c = parse(raw)
    check(str(file) + ':canonical', raw == canon(c) + b'\n')
    check(str(file) + ':compile', c['compile'] == built['identity'])
    check(str(file) + ':reviewed compiler algorithm', c['compile']['compilerSha256'] == 'a87f1f8fafea2e0f1b4a1e299199f5581172a2fe90f59882d3c64eb50f68acaf')
    t = c['tools']
    tool_hashes.add(sha(canon(t)))
    expected_roots = ['/quod-adapter/node_modules/esbuild', '/quod-adapter/node_modules/@esbuild/linux-x64',
                      '/project/node_modules/typescript', '/project/node_modules/@typescript/old']
    check(str(file) + ':four actual roots', t['packageRoots'] == expected_roots)
    expected_entries = {'esbuildESM': expected_roots[0] + '/lib/main.js', 'esbuildCJS': expected_roots[0] + '/lib/main.js',
                        'typescriptCJS': expected_roots[2] + '/lib/typescript.js',
                        'typescriptDelegatedCJS': expected_roots[3] + '/lib/typescript.js', 'esbuildBinary': expected_roots[1] + '/bin/esbuild'}
    check(str(file) + ':actual API routes', t['entries'] == expected_entries)
    files = {r['name']: r for r in t['files']}
    check(str(file) + ':tool inventory uniqueness', len(files) == len(t['files']))
    check(str(file) + ':reviewed complete four-tree bytes/modes/path roster', sha(canon(t['files'])) == '7b7ae881ef3186e4b564c26ffa6e8d40b3d3875a15e41b10d93a6bb24cb3a354' and t['counts'] == {'paths': 183, 'bytes': 47351803})
    check(str(file) + ':four primary installed byte anchors', all(files[n]['bytes'] == size and files[n]['sha256'] == digest for n, (size, digest) in primary.items()))
    check(str(file) + ':host source pins', [r['name'] for r in t['hostSources']] == ['compiler.mjs', 'resolved-compile-certificate.mjs'] and all(r['sha256'] == sha(blob(r['name'])) for r in t['hostSources']))
    check(str(file) + ':Node/API policy', t['node'] == 'v24.18.0' and t['platform'] == 'linux' and t['arch'] == 'x64' and
          t['typescript'] == {'wrapper': {'name': '@typescript/typescript6', 'version': '6.0.2'}, 'implementation': {'name': 'typescript', 'version': '6.0.3'}})
    coverage = c['compile']['inputCoverage']
    check(str(file) + ':physical/onLoad bijection', coverage['physicalSources'] == sorted(s['source'] for s in c['compile']['sources']))
    check(str(file) + ':finite virtual inputs', coverage['schema'] == 'quod-managed-physical-input-coverage-v1' and set(coverage['virtualInputs']) <= {'<define:__QUOD_CONFIG__>', 'quod-managed-als:public-async-local-storage'})
    check(str(file) + ':raw source inventory exact', sorted(r['name'] for r in c['sourceFiles']) == sorted(set(r['source'] for r in c['compile']['sources'])))
    return c, raw

pairs = []
for row in result['pairs']:
    directory = probe / row['instanceId']
    seed, sb = build(directory, 'seed-build.json', 'seed.mjs')
    fresh, fb = build(directory, 'fresh-build.json', 'fresh.mjs')
    validated = read(directory / 'validated.json')
    c, cb = certificate(directory / 'certificate.json', seed)
    check(row['instanceId'] + ':12 exact pairs', sb == fb and seed['identity'] == fresh['identity'] == validated['identity'] and validated['hit'] is True and
          validated['identitySha256'] == seed['identitySha256'] and validated['bundleSha256'] == seed['sha256'])
    check(row['instanceId'] + ':12 certificate digest', sha(cb) == row['certificateSha256'] and len(cb) == row['certificateBytes'])
    pairs.append({'instanceId': row['instanceId'], 'identitySha256': seed['identitySha256'], 'bundleSha256': seed['sha256']})
check('twelve unique actual pairs', len(pairs) == len({x['instanceId'] for x in pairs}) == 12)
seed, seed_raw = build(probe, 'fixture-seed-build.json', 'fixture-seed.mjs')
certificate(probe / 'fixture-certificate.json', seed)
hit_mutations = {'package sideEffects metadata ignored by path-only plugin', 'package type metadata ignored by path-only plugin', 'tsconfig bytes changed',
                 'unrelated path added allows equal input candidate', 'unrelated JSON added allows equal input candidate'}
check('18 actual mutations', len(result['mutations']) == 18)
for row in result['mutations']:
    directory = next(p for p in (probe / 'mutations').iterdir() if read(p / 'mutation.json')['name'] == row['name'])
    decision = read(directory / 'decision.json')
    accepted = row['name'] in hit_mutations
    check('mutation decision:' + row['name'], decision['hit'] is accepted and (accepted or decision['next'] == 'full-compile'))
    if (directory / 'fresh-build.json').exists():
        fresh, fb = build(directory, 'fresh-build.json', 'fresh.mjs')
        if accepted:
            check('accepted mutation actual identity equality:' + row['name'], seed_raw == fb and seed['identity'] == fresh['identity'] == decision['identity'])
    else:
        check('rejected mutant retained error:' + row['name'], row['rejected'] and (directory / 'fresh-error.json').exists())
    if (row.get('observation') or {}).get('sameMetadataRequired'):
        check('same metadata:' + row['name'], row['observation']['sizeUnchanged'] and row['observation']['mtimeUnchanged'])
check('seven warm tool/environment changes', {r['name'] for r in result['warmTools']} == {'compiler-source', 'esbuild-api', 'typescript-wrapper-api', 'typescript-delegated-api', 'typescript-exports', 'typescript-delegated-exports', 'compiler-environment-NODE_ENV'})
for row in result['warmTools']:
    check('warm epoch rejection:' + row['name'], bool(row['captureError'] and row['buildError'] and row['factoryError']))
    check('ordinary cannot acquire certificate:' + row['name'], bool(row.get('plainCaptureError') or row.get('plainBuildError')))
    if row['name'] == 'compiler-source':
        directory = probe / 'warm-tools/compiler-source'
        child = read(directory / 'child-process.json')
        error = read(directory / 'separate-node/error.json')
        check('fresh compiler mutation rejected before first load', child['status'] == 1 and child['signal'] is None and child['error'] is None and
              error['message'] == 'unreviewed compiler source bytes' and not (directory / 'separate-node/certificate.json').exists())
cold = result['coldTools']
check('five same-version fresh tool mutation cases', {r['name'] for r in cold} ==
      {'esbuild-api', 'typescript-wrapper-api', 'typescript-delegated-api', 'platform-tree-extra-file', 'typescript-wrapper-manifest'})
for row in cold:
    directory = probe / 'cold-tools' / row['name']
    mutant = regular(directory / 'mutated-tool.data')
    child, error = read(directory / 'child-process.json'), read(directory / 'separate-node/error.json')
    expected_reason = 'unreviewed TypeScript delegation wrapper' if row['name'] == 'typescript-wrapper-api' else 'unreviewed compiler tool file inventory'
    check('cold actual mutated bytes:' + row['name'], sha(mutant) == row['changedSha256'] and row['changedSha256'] != row['originalSha256'] and row['packageVersionUnchanged'] is True)
    check('cold before-load pin rejection:' + row['name'], child['status'] == 1 and child['signal'] is None and child['error'] is None and error['message'] == expected_reason and
          not (directory / 'separate-node/certificate.json').exists())
    if row['name'] == 'typescript-wrapper-manifest':
        check('manifest actual unchanged version', parse(mutant)['version'] == '6.0.2' and parse(mutant)['quodUnreviewedFixture'] is True)
rel = probe / 'adapter-relocation'
inventories = [read(rel / n) for n in ['source-inventory.json', 'A-inventory.json', 'B-inventory.json']]
rr = read(rel / 'result.json')
check('A/B complete byte/mode/link equality', inventories[0] == inventories[1] == inventories[2] and sha(canon(inventories[0])) == rr['sourceInventorySha256'])
for index, label in enumerate(['A', 'B']):
    built, raw = build(rel, label + '-build.json', label + '.mjs')
    c, cb = certificate(rel / (label + '-certificate.json'), built)
    check(label + ':retained exact certificate', sha(cb) == rr['certificates'][index]['sha256'] and len(cb) == rr['certificates'][index]['bytes'])
check('actual geometry sensitivity', rr['positive']['hit'] is True and rr['relocated']['hit'] is False and rr['relocated']['next'] == 'full-compile' and regular(rel / 'A.mjs') != regular(rel / 'B.mjs'))

mr = read(matrix / 'result.json')
n = read(matrix / 'native-selection.json')
native = {x['instanceId']: x for x in n['executed']}
check('native exact120 complete', n['matched'] and n['completed'] and n['exitCode'] == 0 and n['failedModules'] == 0 and n['unhandledErrors'] == [] and len(native) == len(n['selected']) == 120)
check('matrix disabled and passed', mr['passed'] and not mr['productionEnabled'] and not mr['cacheEnabled'] and not mr['receiptAdmission'] and all(x['pass'] for x in mr['checks']))
states = collections.Counter()
managed = []
for row in mr['instances']:
    actual = native[row['instanceId']]
    check('native passing:' + row['instanceId'], actual['ok'] and actual['state'] in ['pass', 'skip', 'todo'] and all(t['state'] in ['pass', 'skip', 'todo'] for t in actual['rows']))
    if row['decision'] != 'managed':
        continue
    managed.append(row)
    directory = matrix / 'instances' / row['instanceId']
    raw = regular(directory / 'managed/response.json')
    data = parse(raw)
    counter = lambda rows: collections.Counter((t['name'], t['state']) for t in rows)
    check('native actual multiset:' + row['instanceId'], counter(data['rows']) == counter(actual['rows']) and data['files'][0]['name'] == actual['relativeModuleId'] and data['files'][0]['state'] == actual['state'])
    b = data['boundary']
    check('closed worker:' + row['instanceId'], sha(raw) == row['responseSha256'] and data['ok'] and all(b[k] == [] for k in ['unsupported', 'unguardable', 'symbolUnguardable', 'timerErrors', 'unhandledErrors']) and b['completion']['sealed'] and b['completion']['intrinsicLockdown'])
    states.update(t['state'] for t in data['rows'])
def full_rows(modules):
    return collections.Counter((m['file'], m['project'], m['pool'], t['name'], t['state']) for m in modules for t in m['rows'])
baseline_zip = regular(args.baseline_archive, 128 * 1024 * 1024)
assert len(baseline_zip) == 36186785 and sha(baseline_zip) == 'f8e20f3a68cc3fbec7ce01a5ea90e8b50a1d558cbf08146a7fb049e242adc282'
with zipfile.ZipFile(args.baseline_archive) as z:
    entries = z.infolist()
    assert len(entries) <= 20000 and sum(e.file_size for e in entries) <= 1024 ** 3
    seen = set()
    for entry in entries:
        name = entry.filename[:-1] if entry.is_dir() else entry.filename
        p = pathlib.PurePosixPath(name)
        alias = unicodedata.normalize('NFC', name).casefold()
        assert name and not p.is_absolute() and not any(part in ('', '.', '..') for part in name.split('/'))
        assert '\\' not in name and '\x00' not in name and alias not in seen
        seen.add(alias)
        assert stat.S_IFMT(entry.external_attr >> 16) in (0, stat.S_IFDIR if entry.is_dir() else stat.S_IFREG)
        assert entry.file_size <= 16 * 1024 * 1024 and not entry.flag_bits & 1
    baseline_raw = z.read('_temp/managed-matrix/native-selection.json')
assert len(baseline_raw) == 1903888 and sha(baseline_raw) == '1d3ceb0a7520632c23474769721d608875eb4b9cfed6c6d248f61b89c946ced1'
old = parse(baseline_raw)
check('original full4131 logical states', full_rows(n['executed']) == full_rows(old['executed']) and sum(len(x['rows']) for x in n['executed']) == 4131)
check('actual108 admitted', len(managed) == 108)
pr = read(paired / 'result.json')
check('full108 fresh input diagnostic passed', pr['passed'] and not pr['productionEnabled'] and not pr['receiptAdmission'] and pr['nativeSelected'] == 120 and pr['candidates'] == 108 and not pr['issues'])
check('full108 stable bootstrap', pr['bootstrapEnvironment']['afterInitialization'] ==
      pr['bootstrapEnvironment']['afterPreparation'] == expected_environment and read(paired / 'bootstrap-environment.json') == pr['bootstrapEnvironment'])
sizes = {r['instanceId']: r for r in pr['sizes']}
for row in managed:
    key = row['instanceId']
    old_build = read(matrix / 'instances' / key / 'test-build.json')
    fresh, fb = build(paired, key + '-build.json', key + '.mjs')
    c, cb = certificate(paired / (key + '.json'), fresh)
    check('full108 actual raw pair:' + key, fb == regular(matrix / 'instances' / key / 'managed/worker.mjs') and old_build['identity'] == fresh['identity'] == c['compile'])
    check('full108 current prep/options:' + key, old_build['options'] == fresh['options'] and old_build['preparation'] == fresh['preparation'])
    check('full108 retained sizes:' + key, len(cb) == sizes[key]['certificateBytes'] and sha(cb) == sizes[key]['certificateSha256'] and len(canon(fresh['identity'])) == sizes[key]['compileIdentityBytes'])
check('one stable four-package tool identity', len(tool_hashes) == 1)
check('exact payload sums', sum(r['certificateBytes'] for r in sizes.values()) == pr['certificateBytes'] and sum(r['compileIdentityBytes'] for r in sizes.values()) == pr['compileIdentityBytes'])
report = {'schema': 'quod-managed-resolved-certificate-complete-data-audit-v1', 'passed': not issues, 'issues': issues,
          'run': run['id'], 'source': source, 'artifact': {'id': artifact['id'], 'name': artifact['name'], 'archiveSizeBytes': archive.stat().st_size,
          'archiveSha256': archive_sha, 'expandedBytes': sum(x['bytes'] for x in archive_rows), 'files': len(archive_rows)},
          'executedArtifactCode': False, 'compilerChecks': 55, 'targetedChecks': len(result['checks']), 'pairs': pairs,
          'toolEvidenceScope': 'Producer-observed complete four-package inventory/API routes, bound to exact four primary installed TypeScript byte anchors and current compiler/verifier source blobs; full installed tool trees are not copied into this artifact.',
          'nativeBaseline': {'bytes': len(baseline_raw), 'sha256': sha(baseline_raw), 'artifactId': 13587034973, 'archiveBytes': 36186785, 'archiveSha256': 'f8e20f3a68cc3fbec7ce01a5ea90e8b50a1d558cbf08146a7fb049e242adc282'},
          'mutations': len(result['mutations']), 'warmVariants': len(result['warmTools']), 'toolIdentitySha256': sorted(tool_hashes),
          'coldVariants': len(cold),
          'matrix': {'managedPassing': len(managed), 'ordinary': 120 - len(managed), 'managedStates': dict(states), 'nativeStates': dict(collections.Counter(t['state'] for m in n['executed'] for t in m['rows']))},
          'fullFreshPairs': len(sizes), 'payload': {k: pr[k] for k in ['certificateBytes', 'compileIdentityBytes', 'maxCertificateBytes', 'maxCompileIdentityBytes', 'captureSeconds', 'validationSeconds', 'evidenceBounds']},
          'files': archive_rows}
report['auditorSha256'] = sha(regular(__file__))
output = args.output
assert not output.exists() and not output.is_symlink(), 'new explicit audit output required'
with output.open('x') as handle:
    handle.write(json.dumps(report, indent=2) + '\n')
print(json.dumps({'output': str(output), 'sha256': sha(output.read_bytes()), 'passed': not issues, 'issues': issues,
                  'matrix': report['matrix'], 'payload': report['payload']}))
sys.exit(0 if not issues else 1)

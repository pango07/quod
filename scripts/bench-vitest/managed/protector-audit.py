"""Audit verified fixed-order protector diagnostic artifacts as data only.

Usage: protector-audit.py CANDIDATE_ROOT LEGACY_ROOT COMPARISON_ROOT
Archive authentication and GitHub job/step provenance belong to the caller.
Only trusted local audit code runs; retained worker/source files are never run.
"""
import hashlib
import json
import math
import os
import pathlib
import subprocess
import sys
import urllib.parse

from frozen_policy_audit import CONTRACT, POLICY, verify_protector, verify_policy_receipts

HERE = pathlib.Path(__file__).resolve().parent
SOURCE = HERE.parents[2]
CANDIDATE_SOURCE = 'eb2864ebb9c49a1bf7939a47e8f62d59a717112e'
LEGACY_SOURCE = '2a4d6f2f7547dd00e720a32d5b451229784ba8fb'
LEGACY_CONTRACT = 'quod-managed-v0-investigation'
TEST = 'packages/convex-helpers/server/stream.property.test.ts'
TEST_SHA = '9798bab303233bca846a89ccb39ce28bd968329e0704f3e5d67adca934c11c75'
WORKERD_SHA = 'a4d954a9ec17a367cf7d5509a2282785e84dacca6fc13b69c532ab25d157ef48'
NODE_SHA = '41a74efb34cbde5c7632cdac0cf8bd1a14d0b8d73dc1e82755014d9a9ce70f5c'
SOURCE_FILES = ('prelude.mjs', 'compiler.mjs', 'runner.mjs', 'executor.mjs',
                'receipt.mjs', 'preparation.mjs', 'package-lock.json')
RAW_FILES = ('result.json', 'test-build.json', 'managed/invocation.json', 'managed/execution.json',
             'managed/worker.mjs', 'managed/config.capnp', 'managed/response.json')
COMPARE_CHECKS = {
    'legacy source is the frozen measured implementation',
    'project retains the exact unchanged 47-test source',
    'candidate full hardening and unchanged ordinary comparison completed',
    'legacy executes an actual uncached control worker',
    'both actual workers pass all 47 original assertions',
    'policy differences are explicit', 'same actual host class',
    'same actual Node and workerd binary bytes',
    'same native capabilities environment and request policy',
    'both builds retain the same source and declared seed/epoch/environment',
    'complete logical test effects agree',
    'both retained worker timings are finite positive durations',
}
issues, hashes, verified = [], {}, {}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def attempt(label, action):
    try:
        return action()
    except (AssertionError, ValueError, TypeError, KeyError, IndexError, OSError, UnicodeError,
            StopIteration, OverflowError, RecursionError, subprocess.SubprocessError) as error:
        issues.append({'check': label, 'message': str(error)})
        return None


def pairs(rows):
    data = {}
    for key, value in rows:
        require(key not in data, 'duplicate JSON field: ' + key)
        data[key] = value
    return data


def decode(raw):
    def reject(value):
        raise ValueError('non-finite JSON value: ' + value)
    return json.loads(raw.decode('utf8'), object_pairs_hook=pairs, parse_constant=reject)


def raw(file, maximum=32 * 1024 * 1024):
    require(file.is_file() and not file.is_symlink() and file.stat().st_size <= maximum,
            'missing, symbolic, or oversized evidence: ' + str(file))
    return file.read_bytes()


def read(file):
    return decode(raw(file))


def sha(data):
    return hashlib.sha256(data if isinstance(data, bytes) else data.encode('utf8')).hexdigest()


def file_sha(file):
    if str(file) not in hashes:
        hashes[str(file)] = sha(raw(file))
    return hashes[str(file)]


def encode(data, sort=False):
    # The compared worker effects/options and compiler identities contain only
    # finite integral numbers, strings, booleans, null, arrays and own data.
    return json.dumps(data, sort_keys=sort, separators=(',', ':'), ensure_ascii=False, allow_nan=False).encode()


def exact(data, keys, label):
    require(isinstance(data, dict) and set(data) == set(keys), label + ' fields differ')


def inventory(root):
    require(root.is_dir() and not root.is_symlink(), 'not a real artifact directory')
    result, total = [], 0
    for directory, dirs, files in os.walk(root, followlinks=False):
        require(len(pathlib.Path(directory).relative_to(root).parts) <= 10, 'artifact discovery depth exceeded')
        for name in dirs + files:
            require(not pathlib.Path(directory, name).is_symlink(), 'artifact contains symlink')
        for name in files:
            file = pathlib.Path(directory, name)
            require(file.is_file(), 'artifact contains nonregular evidence')
            total += file.stat().st_size
            result.append(file)
            require(len(result) <= 5000 and total <= 512 * 1024 * 1024, 'artifact size/count bound exceeded')
    return result


def committed(commit, name):
    return subprocess.run(['git', '-C', str(SOURCE), 'show',
                           commit + ':scripts/bench-vitest/managed/' + name],
                          capture_output=True, timeout=15, check=True).stdout


def sources(comparison, result):
    require(result['candidateSource'] == CANDIDATE_SOURCE and result['legacySource'] == LEGACY_SOURCE,
            'comparison source pins differ')
    expected = {}
    for label, commit in (('candidate', CANDIDATE_SOURCE), ('legacy', LEGACY_SOURCE)):
        copied = comparison / ('source-' + label)
        require(copied.is_dir() and {p.name for p in copied.iterdir()} == set(SOURCE_FILES), 'source copy inventory differs')
        expected[label] = {}
        for name in SOURCE_FILES:
            identity = sha(committed(commit, name))
            require(file_sha(copied / name) == identity, 'retained source differs from trusted commit: ' + label + '/' + name)
            expected[label][name] = identity
    require(result['sourceFiles'] == expected, 'comparison source SHA map differs')
    workflow = subprocess.run(['git', '-C', str(SOURCE), 'show', CANDIDATE_SOURCE + ':.github/workflows/gate.yml'],
                              capture_output=True, timeout=15, check=True).stdout.decode()
    ordered = [workflow.index(text) for text in (
        'node scripts/bench-vitest/managed/probe.mjs',
        'node protector-legacy-source/scripts/bench-vitest/managed/replay-probe.mjs control',
        'node scripts/bench-vitest/managed/protector-compare.mjs')]
    require(ordered == sorted(ordered), 'pinned workflow order differs')
    return expected


def complete(data, candidate):
    exact(data, ['ok', 'rows', 'files', 'boundary'], 'managed response')
    require(data['ok'] is True and len(data['rows']) == 47 and len({r['name'] for r in data['rows']}) == 47,
            '47 unique actual passing rows required')
    for row in data['rows']:
        exact(row, ['name', 'state', 'errors'], 'managed test')
        require(isinstance(row['name'], str) and row['name'] and row['state'] == 'pass' and row['errors'] == [], 'managed assertion did not pass')
    require(data['files'] == [{'name': 'server/stream.property.test.ts', 'state': 'pass', 'errors': []}], 'managed file result differs')
    b, c = data['boundary'], data['boundary']['completion']
    require(b['replayEligible'] is False and b['contract'] == (CONTRACT if candidate else LEGACY_CONTRACT), 'boundary policy differs')
    require(b.get('intrinsicPolicy') == POLICY if candidate else 'intrinsicPolicy' not in b, 'intrinsic policy differs')
    for key in ('unsupported', 'unguardable', 'symbolUnguardable', 'timerErrors', 'unhandledErrors', 'pendingTimerDetails'):
        require(b[key] == [], 'unsupported or pending boundary effects: ' + key)
    require(b['pendingTimers'] == 0 and c['sealed'] is True and c['intrinsicLockdown'] is True and c['reason'] == '' and
            all(c[k] == 0 for k in ('pendingTimerCallbacks', 'pendingMicrotasks', 'pendingEventCallbacks')), 'unsealed managed completion')
    for method, values in b['logs']:
        require(isinstance(method, str) and isinstance(values, list) and all(isinstance(v, str) for v in values), 'nonprimitive console effects')
    require(type(b['virtualTime']) in (int, float) and math.isfinite(b['virtualTime']) and b['virtualTime'] >= 0, 'invalid virtual clock')


def actual_worker(root, candidate, source_hashes):
    label = 'candidate' if candidate else 'legacy'
    result, build = read(root / 'result.json'), read(root / 'test-build.json')
    directory = root / 'managed'
    response_bytes = raw(directory / 'response.json')
    data = decode(response_bytes)
    complete(data, candidate)
    require(encode(data) == response_bytes, 'actual response was not exact compact JSON')
    phase = [r for r in result['phases'] if r['name'] == 'managed']
    require(len(phase) == 1 and phase[0]['data'] == data and phase[0]['responseSha256'] == sha(response_bytes),
            'phase differs from actual retained worker response')
    require(type(phase[0]['seconds']) in (int, float) and math.isfinite(phase[0]['seconds']) and phase[0]['seconds'] > 0,
            'invalid actual worker timing')
    invocation, execution = read(directory / 'invocation.json'), read(directory / 'execution.json')
    require(invocation['name'] == execution['name'] == 'managed' and execution['completed'] is True and
            execution['timedOut'] is False and execution['error'] is None and
            execution['responseSha256'] == sha(response_bytes), 'actual managed execution did not complete')
    require(invocation['bundleSha256'] == file_sha(directory / 'worker.mjs') == build['sha256'] and
            invocation['configSha256'] == file_sha(directory / 'config.capnp'), 'actual worker/config byte identities differ')
    ident = build['identity']
    require(ident['schema'] == 'quod-managed-compile-v1-research' and ident['entry'] == '/quod-adapter/runner.mjs' and
            ident['compilerSha256'] == source_hashes['compiler.mjs'] and ident['bundleSha256'] == build['sha256'] and
            build['identitySha256'] == sha(encode(ident, True)), 'compiler source/identity differs')
    options = build['options']
    require(ident['configurationSha256'] == sha(encode(options, True)) and
            {k: options[k] for k in ('epoch', 'seed', 'env', 'file')} == {
                'epoch': 1700000000000, 'seed': 42, 'env': {'NODE_ENV': 'test'}, 'file': '/project/' + TEST} and
            options['vitest']['sequence']['seed'] == 42, 'actual declared input/configuration differs')
    require(any(s['source'] == '/project/' + TEST and s['sourceSha256'] == TEST_SHA for s in ident['sources']),
            'original 47-test source pin is absent')
    for name in ('prelude.mjs', 'runner.mjs'):
        require(any(s['source'] == '/quod-adapter/' + name and s['sourceSha256'] == source_hashes[name]
                    for s in ident['sources']), 'reviewed managed adapter source pin is absent: ' + name)
    closure = build['closure']
    require(closure['finalBundleParsed'] is True and closure['remainingDynamicImports'] == 0 and
            closure['remainingDynamicRequires'] == 0 and all(i == 'node:async_hooks' for i in closure['externalImports']), 'compiled module closure is open')
    require(build['preparation']['preparerSha256'] == source_hashes['preparation.mjs'] and
            build['preparation']['configurationSha256'] == sha(encode(options['vitest'], True)), 'resolved configuration identity differs')
    runtime, host = invocation['runtimePolicy'], invocation['host']
    require(runtime['workerdSha256'] == WORKERD_SHA and runtime['supervisor']['nodeExecutableSha256'] == NODE_SHA and
            runtime['supervisor']['nodeVersion'] == host['nodeVersion'] == 'v24.18.0' and
            runtime['supervisor']['sourceSha256'] == source_hashes['executor.mjs'], 'actual native/supervisor byte identities differ')
    require(host['platform'] == 'linux' and host['arch'] == 'x64' and host['ImageOS'] and host['ImageVersion'] and
            host['kernel']['release'] and host['kernel']['version'] and host['libc']['family'] == 'glibc', 'actual host class is incomplete')
    require(runtime.get('boundaryContract') == CONTRACT and runtime.get('intrinsicPolicy') == POLICY if candidate else
            'boundaryContract' not in runtime and 'intrinsicPolicy' not in runtime, 'keyed native policy differs')
    require(runtime['moduleName'] == 'worker.mjs' and runtime['services'] == ['main', 'deny'] and
            runtime['globalOutbound'] == 'deny' and runtime['bindings'] == [] and
            runtime['compatibilityDate'] == '2026-07-01' and runtime['compatibilityFlags'] == ['nodejs_als'] and
            runtime['request'] == {'method': 'GET', 'path': '/run'} and runtime['denyModuleSha256'] == file_sha(directory / 'deny.mjs'),
            'native capability policy differs')
    require(invocation['env'] == runtime['processEnv'] and set(invocation['env']) == {'PATH', 'TZ', 'LANG'} and
            invocation['env']['TZ'] == 'UTC' and invocation['env']['LANG'] == 'C.UTF-8', 'worker ambient environment differs')
    url = urllib.parse.urlsplit(invocation['request']['url'])
    require(invocation['request']['method'] == 'GET' and url.scheme == 'http' and url.hostname == '127.0.0.1' and
            url.port and url.path == '/run' and not url.query and not url.fragment, 'native request differs')
    require(invocation['args'] == ['serve', invocation['args'][1]] and invocation['args'][1].endswith('/managed/config.capnp'), 'native runtime flags differ')
    config = raw(directory / 'config.capnp').decode()
    expected_config = '''using Workerd = import "/workerd/workerd.capnp";
const config :Workerd.Config = (
  services = [
    (name = "main", worker = (modules = [(name = "worker.mjs", esModule = embed "worker.mjs")],
      compatibilityDate = "2026-07-01", compatibilityFlags = ["nodejs_als"], globalOutbound = "deny")),
    (name = "deny", worker = (modules = [(name = "deny.mjs", esModule = embed "deny.mjs")],
      compatibilityDate = "2026-07-01", globalOutbound = "deny"))
  ],
  sockets = [(name = "http", address = "127.0.0.1:PORT", http = (), service = "main")]
);
'''.replace('PORT', str(url.port))
    require(config == expected_config, 'retained config grants different capabilities')
    if candidate:
        require(result['boundaryChecksVersion'] == 7 and result['managedSupported'] is True and result['managedPassed'] is True and
                result['fallbackExecuted'] is False and result['namesAndStatesAgree'] is True and result['ordinary']['exit'] == 0,
                'candidate full-hardening result is incomplete')
    else:
        require(result['role'] == 'control' and result['passed'] is True and result['productionEnabled'] is False and
                result['actualManagedExecutions'] == 1 and result['replayed'] is False and
                [r['name'] for r in result['phases']] == ['preflight', 'managed'], 'legacy was not an actual uncached control')
        execution_identity = read(root / 'execution-identity.json')
        keyed = execution_identity['identity']
        require(execution_identity['key'] == result['executionKey'] ==
                sha(b'quod-managed-execution-v1-research\n' + encode(keyed, True)), 'legacy execution domain digest differs')
        require(keyed['compile'] == ident and keyed['preparation'] == build['preparation'] and keyed['host'] == host,
                'legacy keyed compiler/preparation/host differs from actual execution')
        preflight = read(root / 'preflight-build.json')
        capability = read(root / 'preflight' / 'response.json')
        native = sha(encode({'preflightIdentity': preflight['identitySha256'],
                            'inventory': capability['boundary']['inventory'],
                            'symbols': capability['boundary']['symbolInventory']}, True))
        require(keyed['runtimePolicy'] == {**runtime, 'nativeFingerprintSha256': native},
                'legacy keyed runtime/native fingerprint differs from actual execution')
        require(keyed['supervisorPolicy'] == {'receiptValidatorSha256': source_hashes['receipt.mjs'],
                'experimentSha256': sha(committed(LEGACY_SOURCE, 'replay-probe.mjs'))}, 'legacy receipt/supervisor policy differs')
    effect = {'ok': data['ok'], 'rows': data['rows'], 'files': data['files'],
              'logs': data['boundary']['logs'], 'virtualTime': data['boundary']['virtualTime']}
    if not candidate:
        require(result['effects'] == effect and result['effectSha256'] == sha(encode(effect, True)),
                'legacy complete logical effects differ from actual worker response')
    verified[label] = {'result': result, 'build': build, 'invocation': invocation, 'data': data,
                       'effects': effect, 'seconds': phase[0]['seconds']}
    return verified[label]


def hardening(root):
    # This is trusted repository audit code, with a fixed command and sanitized
    # Python environment. No command/source is read from an artifact.
    auditor, helper = HERE / 'audit.py', HERE / 'frozen_policy_audit.py'
    before = {str(p): file_sha(p) for p in (auditor, helper)}
    done = subprocess.run([sys.executable, str(auditor), str(root)], cwd=HERE,
                          env={'PATH': os.environ.get('PATH', ''), 'LANG': 'C.UTF-8', 'TZ': 'UTC'},
                          capture_output=True, timeout=120)
    data = decode(done.stdout)
    require(done.returncode == 0 and data['auditPassed'] is True and data['integrityPassed'] is True and
            data['artifactComplete'] is True and data['issues'] == [], 'independent original/candidate hardening audit failed: ' + str(data.get('issues')))
    require(data['preflightChecksPerProcess'] == 31 and data['preflightFreshProcesses'] == 2 and
            data['compilerRegressionChecks'] == 40 and len(data['boundaryFixtureModes']) == 18 and
            data['protectorChecksPerProcess'] == 51 and data['protectorFreshProcesses'] == 2 and
            data['ordinaryTestStates'] == 47 and len(data['fixtureEvidence']) == 3 and len(data['moduleFixtureEvidence']) == 4,
            'original or protector matrix counts differ')
    protectors = [read(root / phase / 'response.json') for phase in ('protector-one', 'protector-two')]
    require(all(verify_protector(value) == 51 for value in protectors) and protectors[0]['checks'] == protectors[1]['checks'],
            'fresh 51-check protector semantics differ')
    receipts = verify_policy_receipts(root / 'receipt-checks')
    require(receipts['originalChecks'] == 107 and receipts['policyChecks'] == 16 and data['policyReceiptChecks'] == receipts,
            'original107 or cross-policy16 matrix differs')
    require(all(sha(raw(pathlib.Path(p))) == h for p, h in before.items()), 'trusted audit code changed during analysis')
    return {'sourceSha256': before, 'stdoutSha256': sha(done.stdout), 'issues': data['issues'],
            'compilerChecks': 40, 'capabilityChecksPerProcess': 31, 'capabilityProcesses': 2,
            'boundaryModes': 18, 'ordinaryFixtureStates': 12, 'moduleFixtureStates': 7,
            'originalTests': 47, 'protectorChecksPerProcess': 51, 'protectorProcesses': 2,
            'originalReceiptChecks': 107, 'crossPolicyReceiptChecks': 16}


def comparison(root, report, current, old):
    require(report['schema'] == 'quod-managed-protector-fixed-order-diagnostic-v1' and report['passed'] is True and
            not report.get('error') and report['order'] == ['candidate-full-hardening', 'legacy-control'] and
            report['interpretation'] == 'same-runner fixed-order diagnostic; no causal or general performance claim',
            'comparison completion, order, or claim scope differs')
    require(len(report['checks']) == len(COMPARE_CHECKS) and {r['name'] for r in report['checks']} == COMPARE_CHECKS and
            all(r['pass'] is True for r in report['checks']), 'comparison check inventory differs')
    require(current['effects'] == old['effects'] and report['effectsSha256'] == sha(encode(current['effects'])),
            'complete47 test/files/console/virtual-time effects differ')
    require(current['invocation']['host'] == old['invocation']['host'] and
            current['build']['options'] == old['build']['options'], 'actual host or declared complete configuration differs')
    capabilities = ('moduleName', 'services', 'compatibilityDate', 'compatibilityFlags', 'globalOutbound',
                    'denyModuleSha256', 'bindings', 'processEnv', 'request')
    require({k: current['invocation']['runtimePolicy'][k] for k in capabilities} ==
            {k: old['invocation']['runtimePolicy'][k] for k in capabilities}, 'actual native capabilities differ')
    require(current['build']['sha256'] != old['build']['sha256'], 'distinct policies unexpectedly share a worker bundle')
    for label, observed in (('candidate', current), ('legacy', old)):
        directory = candidate_root if label == 'candidate' else legacy_root
        require(report['rawEvidence'][label] == {name: file_sha(directory / name) for name in RAW_FILES}, 'comparison raw-byte map differs')
    expected = {'candidateWorker': current['seconds'], 'legacyWorker': old['seconds'],
                'candidateOrdinary': current['result']['ordinary']['seconds']}
    require(report['seconds'] == expected and report['workerRatio'] == expected['candidateWorker'] / expected['legacyWorker'],
            'fixed-order worker timing ratio differs')
    return {'seconds': expected, 'workerRatio': report['workerRatio'], 'effectsSha256': report['effectsSha256'],
            'host': current['invocation']['host'], 'order': report['order'],
            'interpretation': report['interpretation']}


def main():
    global candidate_root, legacy_root
    require(len(sys.argv) == 4, 'usage: protector-audit.py CANDIDATE_ROOT LEGACY_ROOT COMPARISON_ROOT')
    candidate_root, legacy_root, compare_root = [pathlib.Path(p).absolute() for p in sys.argv[1:]]
    inventories = [attempt(label + ' inventory', lambda root=root: inventory(root))
                   for label, root in (('candidate', candidate_root), ('legacy', legacy_root), ('comparison', compare_root))]
    report = attempt('comparison result', lambda: read(compare_root / 'result.json'))
    source_hashes = attempt('trusted committed source copies and declared workflow order', lambda: sources(compare_root, report)) if report else None
    matrices = attempt('original matrices,51 fresh protector checks and16 cross-policy receipts', lambda: hardening(candidate_root))
    if source_hashes:
        current = attempt('actual candidate47 worker', lambda: actual_worker(candidate_root, True, source_hashes['candidate']))
        old = attempt('actual legacy47 worker', lambda: actual_worker(legacy_root, False, source_hashes['legacy']))
    else:
        current, old = None, None
    diagnostic = attempt('independent complete effects, host and fixed-order timing comparison',
                         lambda: comparison(compare_root, report, current, old)) if report and current and old else None
    for paths in inventories:
        for file in paths or []:
            attempt('file ' + str(file), lambda file=file: file_sha(file))
    evidence = {'schema': 'quod-managed-protector-independent-audit-v1',
                'auditPassed': not issues and diagnostic is not None and matrices is not None,
                'issues': issues, 'candidateSource': CANDIDATE_SOURCE, 'legacySource': LEGACY_SOURCE,
                'matrices': matrices, 'diagnostic': diagnostic,
                'method': {'execution': 'trusted local data auditors only; no retained artifact source or commands execute',
                           'provenance': 'caller authenticates GitHub archive bytes and job/step provenance',
                           'order': 'candidate first, legacy second in pinned workflow; absolute candidate timestamps are not retained',
                           'claim': 'same-runner fixed-order diagnostic; no causal or general performance claim'},
                'verifiedFiles': dict(sorted(hashes.items()))}
    print(json.dumps(evidence, indent=2))
    return 0 if evidence['auditPassed'] else 1


if __name__ == '__main__':
    try:
        sys.exit(main())
    except (AssertionError, ValueError, TypeError, KeyError, IndexError, OSError, UnicodeError) as error:
        print(json.dumps({'schema': 'quod-managed-protector-independent-audit-v1', 'auditPassed': False,
                          'issues': issues + [{'check': 'audit input', 'message': str(error)}]}, indent=2))
        sys.exit(1)

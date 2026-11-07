"""Independently audit extracted managed-runtime evidence without executing it.

Both completed gates and interrupted/failed gates produce a JSON diagnosis.
Exit zero means a complete passing gate was independently checked. The
integrityPassed field separately identifies internally consistent partial data.
"""
import hashlib
import json
import pathlib
import re
import sys
import urllib.parse
from frozen_policy_audit import CONTRACT, POLICY, verify_protector, verify_policy_receipts

root = pathlib.Path(sys.argv[1])
issues = []
verified_builds = {}
verified_phases = {}
verified_invocations = {}
preflight_checks = []
preflight_random = []
protector_checks = []
sha_pattern = re.compile(r'^[0-9a-f]{64}$')


def require(condition, message):
    if not condition:
        raise ValueError(message)


def attempt(category, label, action):
    try:
        return action()
    except (AssertionError, KeyError, TypeError, ValueError, IndexError, OSError) as error:
        issues.append({'category': category, 'check': label, 'message': str(error)})
        return None


def read(name):
    path = root / name
    require(not path.is_symlink(), 'symlink is not evidence: ' + name)
    return json.loads(path.read_text())


def sha(name):
    path = root / name
    require(not path.is_symlink(), 'symlink is not evidence: ' + name)
    return hashlib.sha256(path.read_bytes()).hexdigest()


def canonical(value):
    # Compile identities contain strings, booleans, null and integral source
    # offsets; no engine-dependent floating-point formatting is involved.
    return json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=False, allow_nan=False)


def digest_value(value):
    return hashlib.sha256(canonical(value).encode()).hexdigest()


def valid_sha(value):
    return isinstance(value, str) and bool(sha_pattern.fullmatch(value))


def rows(report):
    return sorted([test['fullName'], test['status']]
                  for file in report['testResults'] for test in file['assertionResults'])


def managed_rows(values):
    states = {'pass': 'passed', 'fail': 'failed', 'skip': 'skipped', 'todo': 'todo'}
    return sorted([row['name'], states.get(row['state'], row['state'])] for row in values)


def sealed(boundary):
    completion = boundary.get('completion', {})
    return (completion.get('sealed') is True and boundary['pendingTimers'] == 0
            and all(completion.get(key) == 0 for key in (
                'pendingTimerCallbacks', 'pendingMicrotasks', 'pendingEventCallbacks'))
            and not boundary['timerErrors'] and not boundary['unhandledErrors'])


def supported(response):
    boundary = response.get('boundary', {})
    return (not response.get('error') and sealed(boundary)
            and (version < 7 or boundary.get('contract') == CONTRACT and boundary.get('intrinsicPolicy') == POLICY)
            and not boundary['unsupported'] and not boundary['unguardable'])


result = attempt('integrity', 'result.json', lambda: read('result.json')) or {}
version = result.get('boundaryChecksVersion', 1)
check_count = {1: 13, 2: 16, 3: 17, 4: 26, 5: 31, 6: 31, 7: 31}.get(version)
attempt('integrity', 'supported evidence version', lambda: require(check_count is not None, 'unknown boundary version'))
attempt('integrity', 'cache remains disabled', lambda: require(result.get('cacheEnabled') is False, 'cacheEnabled must be false'))
if version >= 4:
    attempt('integrity', 'runtime binary identity', lambda: require(valid_sha(result['workerdSha256']), 'invalid workerd SHA256'))
if version >= 7:
    attempt('integrity', 'explicit frozen-slot policy', lambda: require(
        result['boundaryContract'] == CONTRACT and result['intrinsicPolicy'] == POLICY, 'new policy declaration differs'))


def verify_build(name, build, worker_names):
    require(valid_sha(build['sha256']), 'invalid bundle SHA256')
    imports = [item for output in build['metadata']['outputs'].values() for item in output['imports']]
    require(all(item['path'] == 'node:async_hooks' for item in imports), 'bundle has unreviewed external import')
    workers = []
    for worker in worker_names:
        if (root / worker).exists():
            require(build['sha256'] == sha(worker), 'bundle hash mismatch: ' + worker)
            workers.append(worker)
    if version >= 4:
        require(all(item.get('external') is True for item in imports), 'unexpected nonexternal final import')
        identity = build['identity']
        require(identity['schema'] == 'quod-managed-compile-v1-research', 'unexpected compile identity schema')
        require(build['identitySha256'] == digest_value(identity), 'compile identity hash mismatch')
        require(identity['bundleSha256'] == build['sha256'], 'identity bundle hash mismatch')
        require(valid_sha(identity['compilerSha256']), 'missing compiler source identity')
        require(valid_sha(identity['configurationSha256']), 'missing configuration identity')
        require(identity['globs'] == build['globs'] and identity['adapters'] == build['adapters'], 'identity collection mismatch')
        require(identity['moduleNameSetsSha256'] == digest_value(build['globs']), 'module name set hash mismatch')
        for source in identity['sources']:
            require(valid_sha(source['sourceSha256']) and valid_sha(source['compiledSourceSha256']), 'invalid module source identity')
        if 'options' in build:
            require(identity['configurationSha256'] == digest_value(build['options']), 'declared options hash mismatch')
        closure = build['closure']
        require(closure['finalBundleParsed'] is True, 'final bundle was not parsed')
        require(closure['remainingDynamicImports'] == 0 and closure['remainingDynamicRequires'] == 0, 'bundle has unresolved loading')
        require(all(value == 'node:async_hooks' for value in closure['externalImports']), 'unreviewed native closure')
        if 'preparation' in build:
            preparation = build['preparation']
            require(preparation['schema'] == 'quod-managed-preparation-v1-research', 'unexpected preparation schema')
            require(valid_sha(preparation['preparerSha256']), 'missing preparer source identity')
            require(preparation['configurationSha256'] == digest_value(build['options']['vitest']), 'resolved configuration hash mismatch')
            for source in preparation['sources']:
                require(valid_sha(source['sha256']), 'invalid preparation source identity')
            require(read('project-selection.json')['identity'] == preparation, 'project preparation records differ')
    verified_builds[name] = {'sha256': build['sha256'], 'identitySha256': build.get('identitySha256'), 'workerFiles': workers}


# Check build records even when startup failed before a phase was appended.
for path in sorted(root.glob('*-build.json')):
    name = path.name
    stem = name[:-len('-build.json')]
    worker_names = (['preflight-one/worker.mjs', 'preflight-two/worker.mjs'] if stem == 'preflight' else
                    ['protector-one/worker.mjs', 'protector-two/worker.mjs'] if stem == 'protector' else
                    ['managed/worker.mjs'] if stem == 'test' else [stem + '/worker.mjs'])
    attempt('integrity', name, lambda name=name, worker_names=worker_names: verify_build(name, read(name), worker_names))


BOUNDARY_PRE_V6 = {
    'control': {'ok': True, 'sealed': True, 'pendingTimers': 0, 'pendingTimerCallbacks': 0,
                'pendingEventCallbacks': 0, 'pendingMicrotasks': 0, 'timerErrors': 0, 'unhandledErrors': 0},
    'interval': {'ok': True, 'sealed': False, 'pendingTimersMin': 1, 'reasonIncludes': 'budget',
                 'unsupportedIncludes': ['incomplete async effects:']},
    'detached-timer-pending': {'ok': True, 'sealed': False, 'pendingTimers': 0, 'pendingTimerCallbacksMin': 1,
                             'reasonIncludes': 'budget', 'unsupportedIncludes': ['incomplete async effects:']},
    'detached-timer-rejection': {'ok': False, 'sealed': False, 'pendingTimers': 0, 'pendingTimerCallbacks': 0,
                               'reasonIncludes': 'asynchronous errors', 'timerErrorsInclude': ['QUOD_DETACHED_TIMER_REJECTION']},
    'unhandled-promise': {'ok': False, 'sealed': False, 'reasonIncludes': 'asynchronous errors',
                          'unhandledErrorsInclude': ['QUOD_UNHANDLED_PROMISE']},
    'async-event-listener': {'ok': False, 'sealed': False, 'reasonIncludes': 'asynchronous errors',
                              'unhandledErrorsInclude': ['QUOD_ASYNC_EVENT_LISTENER']},
    'async-event-listener-pending': {'ok': True, 'sealed': False, 'pendingEventCallbacksMin': 1,
                                    'reasonIncludes': 'budget', 'unsupportedIncludes': ['incomplete async effects:']},
    'sync-event-listener': {'ok': False, 'sealed': False, 'reasonIncludes': 'asynchronous errors',
                             'unhandledErrorsInclude': ['QUOD_SYNC_EVENT_LISTENER']},
    'global-event-receiver': {'ok': True, 'sealed': True, 'unsupportedIncludes': [
        'global EventTarget.addEventListener', 'global EventTarget.removeEventListener', 'global EventTarget.dispatchEvent']},
    'reflect-construct-clock': {'ok': True, 'sealed': True, 'pendingTimers': 0, 'timerErrors': 0, 'unhandledErrors': 0},
    'set-add-erasure': {'ok': True, 'sealed': True, 'unsupportedIncludes': ['global fetch']},
    'json-tojson-forgery': {'ok': False, 'sealed': True, 'forgedAbsent': True,
                          'evidenceSentinel': 'QUOD_HONEST_FAILED_RECEIPT', 'checksAllPass': True},
    'als-tampering': {'ok': True, 'sealed': True, 'pendingTimers': 0, 'timerErrors': 0, 'unhandledErrors': 0},
}
BOUNDARY_V6 = {
    **BOUNDARY_PRE_V6,
    'global-onunhandledrejection': {
        'ok': False, 'sealed': False, 'pendingTimers': 0, 'timerErrors': 0, 'unhandledErrorsMin': 1,
        'reasonIncludes': 'asynchronous errors', 'unsupportedIncludes': ['global onunhandledrejection'],
        'unhandledErrorsInclude': ['QUOD_LEGACY_REJECTION'],
    },
    'global-onerror': {
        'ok': False, 'sealed': False, 'pendingTimers': 0, 'timerErrorsMin': 1, 'unhandledErrors': 0,
        'reasonIncludes': 'asynchronous errors', 'unsupportedIncludes': ['global onerror', 'global onfetch'],
        'timerErrorsInclude': ['QUOD_LEGACY_ERROR'],
    },
    'local-legacy-handler': {
        'ok': True, 'sealed': True, 'pendingTimers': 0, 'pendingTimerCallbacks': 0, 'pendingMicrotasks': 0,
        'pendingEventCallbacks': 0, 'timerErrors': 0, 'unhandledErrors': 0,
        'unsupportedIncludes': ['legacy event handler onfoo'],
    },
    'local-legacy-getter': {
        'ok': True, 'sealed': True, 'pendingTimers': 0, 'pendingTimerCallbacks': 0, 'pendingMicrotasks': 0,
        'pendingEventCallbacks': 0, 'timerErrors': 0, 'unhandledErrors': 0,
        'unsupportedIncludes': ['legacy event handler onfoo'],
    },
    'abort-handler': {
        'ok': True, 'sealed': True, 'pendingTimers': 0, 'pendingTimerCallbacks': 0, 'pendingMicrotasks': 0,
        'pendingEventCallbacks': 0, 'timerErrors': 0, 'unhandledErrors': 0,
        'unsupportedIncludes': ['AbortSignal.onabort'],
    },
}
BOUNDARY = BOUNDARY_V6 if version >= 6 else BOUNDARY_PRE_V6


def verify_response(phase):
    name = phase['name']
    require(re.fullmatch(r'[a-z0-9-]+', name) is not None, 'invalid phase path')
    require(name not in verified_phases, 'duplicate phase name')
    response = read(name + '/response.json')
    require(phase['responseSha256'] == sha(name + '/response.json'), 'response SHA256 differs')
    require(phase['data'] == response, 'phase data differs from response file')
    config = (root / name / 'config.capnp').read_text()
    require('compatibilityDate = "2026-07-01"' in config, 'wrong runtime compatibility date')
    require('compatibilityFlags = ["nodejs_als"]' in config, 'wrong runtime flags')
    require('globalOutbound = "deny"' in config, 'missing outbound deny policy')
    require('nodejs_compat' not in config and 'disk =' not in config, 'unexpected host capabilities')
    build_name = ('preflight-build.json' if name.startswith('preflight-') else
                  'protector-build.json' if name.startswith('protector-') else
                  'test-build.json' if name == 'managed' else name + '-build.json')
    require(build_name in verified_builds, 'phase build record failed verification')
    require(name + '/worker.mjs' in verified_builds[build_name]['workerFiles'], 'phase bundle not verified')
    if name != 'native-outbound':
        boundary = response['boundary']
        require(boundary['replayEligible'] is False, 'replay was enabled')
        if version >= 7:
            require(boundary['contract'] == CONTRACT and boundary['intrinsicPolicy'] == POLICY, 'response crossed intrinsic policy')
        if version >= 4:
            require(boundary['completion']['intrinsicLockdown'] is True, 'intrinsics were not locked')
            require(boundary['completion']['protectedIntrinsicObjects'] > 0, 'intrinsic policy did not cover objects')
            require(len(boundary['pendingTimerDetails']) == boundary['pendingTimers'], 'pending timer inventory differs')
            if boundary['completion']['sealed']:
                require(sealed(boundary), 'sealed response contains pending work or asynchronous errors')
        if version >= 5:
            require(boundary['symbolUnguardable'] == [], 'ambient symbol capability could not be guarded')
            inventory = boundary['symbolInventory']
            require(isinstance(inventory, list), 'symbol inventory must be an array')
            previous_owner = -1
            for symbol in inventory:
                require(set(symbol) == {'owner', 'ownerIndex', 'name', 'descriptor', 'valueType', 'primitiveValue',
                                        'enumerable', 'configurable', 'writable', 'policy'}, 'symbol inventory schema differs')
                require(isinstance(symbol['owner'], str) and isinstance(symbol['name'], str), 'live reference in symbol label')
                require(type(symbol['ownerIndex']) is int and symbol['ownerIndex'] >= previous_owner, 'symbol owner index differs')
                previous_owner = symbol['ownerIndex']
                require(isinstance(symbol['primitiveValue'], str), 'live reference in symbol value')
                require(all(type(symbol[key]) is bool for key in ('enumerable', 'configurable', 'writable')),
                        'symbol descriptor flags must be primitive booleans')
                require(symbol['policy'] in ('fixed primitive data', 'guarded'), 'unguarded ambient symbol')
                if symbol['policy'] == 'fixed primitive data':
                    require(symbol['descriptor'] == 'data' and symbol['valueType'] in (
                        'undefined', 'null', 'string', 'boolean', 'number', 'bigint', 'symbol'), 'symbol exposes native reference')
                else:
                    require(symbol['descriptor'] in ('data', 'accessor') and symbol['primitiveValue'] == '',
                            'guarded symbol descriptor differs')
    verified_phases[name] = response


def verify_executor(name):
    invocation = read(name + '/invocation.json')
    require(invocation['schema'] == 'quod-managed-invocation-v1-research' and invocation['name'] == name,
            'executor invocation schema differs')
    require(invocation['configSha256'] == sha(name + '/config.capnp'), 'invocation config hash differs')
    require(invocation['bundleSha256'] == sha(name + '/worker.mjs'), 'invocation bundle hash differs')
    require(isinstance(invocation['executable'], str) and isinstance(invocation['cwd'], str), 'invocation paths differ')
    require(invocation['args'][0] == 'serve' and len(invocation['args']) == 2 and
            invocation['args'][1].endswith('/' + name + '/config.capnp'), 'actual runtime arguments differ')
    policy = invocation['runtimePolicy']
    require(policy['schema'] == 'quod-managed-runtime-policy-v1-research', 'runtime policy schema differs')
    if version >= 7:
        require(policy['boundaryContract'] == CONTRACT and policy['intrinsicPolicy'] == POLICY, 'intrinsic policy omitted from keyed invocation')
    require(policy['workerdSha256'] == result['workerdSha256'], 'invoked runtime binary identity differs')
    require(policy['denyModuleSha256'] == sha(name + '/deny.mjs'), 'deny module identity differs')
    require(policy['compatibilityDate'] == '2026-07-01' and policy['compatibilityFlags'] == ['nodejs_als'],
            'runtime capability policy differs')
    require(policy['moduleName'] == 'worker.mjs' and policy['globalOutbound'] == 'deny' and
            policy['services'] == ['main', 'deny'] and policy['bindings'] == [], 'runtime service/binding policy differs')
    require(policy['processEnv'] == invocation['env'] and set(invocation['env']) == {'PATH', 'TZ', 'LANG'},
            'invoked environment differs')
    require(isinstance(invocation['env']['PATH'], str) and invocation['env']['TZ'] == 'UTC' and
            invocation['env']['LANG'] == 'C.UTF-8', 'runtime environment policy differs')
    require(policy['request'] == {'method': 'GET', 'path': '/run'}, 'runtime request policy differs')
    request = invocation['request']
    url = urllib.parse.urlsplit(request['url'])
    require(request['method'] == 'GET' and url.scheme == 'http' and url.hostname == '127.0.0.1' and
            url.port and url.path == '/run' and not url.query and not url.fragment, 'actual runtime request differs')
    supervisor = policy['supervisor']
    require(valid_sha(supervisor['sourceSha256']) and valid_sha(supervisor['nodeExecutableSha256']),
            'supervisor implementation identity missing')
    require(type(supervisor['watchdogMs']) is int and 0 < supervisor['watchdogMs'] <= 2147483647 and
            supervisor['responseBytesMax'] > 0, 'supervisor execution bounds missing')
    require(supervisor['completionBounds'] == {'nativeTurnFencesMax': 512, 'timerCallbackBudget': 4096,
            'timerCallbackBudgetScope': 'callbacks after completion begins; reject when exceeded', 'quietNativeTurns': 2},
            'supervisor completion bounds differ')
    host = invocation['host']
    require(set(host) == {'ImageOS', 'ImageVersion', 'platform', 'arch', 'kernel', 'libc', 'nodeVersion'},
            'host class contains unexpected machine-specific resource metadata')
    require(set(host['kernel']) == {'release', 'version'} and
            set(host['libc']) == {'family', 'runtimeVersion', 'compilerVersion'}, 'host kernel/libc class differs')
    require(host['nodeVersion'] == supervisor['nodeVersion'], 'supervisor node version differs')
    execution = read(name + '/execution.json')
    require(execution['schema'] == 'quod-managed-execution-v1-research' and execution['name'] == name,
            'executor completion schema differs')
    if name in verified_phases:
        require(execution['completed'] is True and execution['timedOut'] is False and execution['error'] is None,
                'phase retained successful data from an incomplete supervisor execution')
        require(execution['exit'] and execution['exit']['spawnError'] is None, 'worker failed to start or terminate')
        response_sha = sha(name + '/response.json')
        require(execution['responseSha256'] == response_sha, 'executor completion response hash differs')
        meta = read(name + '/response-meta.json')
        require(meta['sha256'] == response_sha and meta['bytes'] == (root / name / 'response.json').stat().st_size,
                'executor response bytes differ')
        require(200 <= meta['status'] <= 299, 'successful phase retained a failed HTTP response')
        require(meta['bytes'] <= supervisor['responseBytesMax'], 'response exceeded declared supervisor byte limit')
    else:
        require(execution['completed'] is False, 'successful executor result missing from phase record')
    if verified_invocations:
        first = next(iter(verified_invocations.values()))
        require(policy == first['runtimePolicy'] and host == first['host'], 'runtime policy or host changed within gate')
    verified_invocations[name] = {'runtimePolicy': policy, 'host': host, 'completed': execution['completed']}


def verify_preflight(name, response):
    require(response['ok'] is True, 'preflight reports failure')
    require(len(response['checks']) == check_count and all(c['pass'] for c in response['checks']), 'preflight checks changed or failed')
    boundary = response['boundary']
    require(not boundary['unguardable'] and not boundary['timerErrors'], 'preflight boundary failure')
    require(boundary['pendingTimers'] == 0 and boundary['virtualTime'] == 5, 'preflight schedule changed')
    denied = {'global fetch', 'global crypto', 'global WeakRef', 'global WebAssembly', 'AbortSignal.timeout'}
    if check_count >= 16:
        denied |= {'global FinalizationRegistry', 'Event.timeStamp'}
    if version >= 4:
        denied |= {'eval', 'Function constructor', 'AsyncFunction constructor', 'GeneratorFunction constructor',
                   'AsyncGeneratorFunction constructor', 'global EventTarget.addEventListener',
                   'global EventTarget.removeEventListener', 'global EventTarget.dispatchEvent'}
        require(sealed(boundary), 'preflight did not seal')
    require(set(boundary['unsupported']) == denied, 'preflight deny inventory differs')
    preflight_checks.append(response['checks'])
    preflight_random.append(response['random'])


def verify_boundary(mode, response):
    expected = BOUNDARY[mode]
    boundary, completion = response['boundary'], response['boundary']['completion']
    require(not response.get('error') and response['mode'] == mode, 'boundary fixture execution error')
    require(response['ok'] is expected['ok'], 'boundary fixture result differs')
    require(response['checks'] and all(check['pass'] for check in response['checks']), 'boundary fixture check failed')
    require(completion['sealed'] is expected['sealed'], 'boundary fixture seal differs')
    require(0 <= completion['nativeTurnFences'] <= 512 and 0 <= completion['timerCallbacks'] <= 4097, 'completion exceeded finite budget')
    for key in ('pendingTimers', 'pendingTimerCallbacks', 'pendingMicrotasks', 'pendingEventCallbacks', 'timerErrors', 'unhandledErrors'):
        actual = (boundary['pendingTimers'] if key == 'pendingTimers' else
                  len(boundary[key]) if key in ('timerErrors', 'unhandledErrors') else completion[key])
        if key in expected:
            require(actual == expected[key], key + ' differs')
        if key + 'Min' in expected:
            require(actual >= expected[key + 'Min'], key + ' minimum not reached')
    if 'reasonIncludes' in expected:
        require(expected['reasonIncludes'] in completion['reason'], 'completion reason differs')
    for key in ('unsupported', 'timerErrors', 'unhandledErrors'):
        for value in expected.get(key + 'Includes', expected.get(key + 'Include', [])):
            require(any(value in item for item in boundary[key]), 'missing recorded effect: ' + value)
    if expected.get('forgedAbsent'):
        require('forged' not in response, 'JSON hook forged a receipt')
        require(response['evidence']['sentinel'] == expected['evidenceSentinel'], 'honest failed receipt was lost')


for phase in result.get('phases', []):
    name = phase.get('name', '<unnamed>')
    attempt('integrity', name + ' response/build', lambda phase=phase: verify_response(phase))
    response = verified_phases.get(name)
    if response is None:
        continue
    if name.startswith('preflight-'):
        attempt('semantic', name + ' checks', lambda name=name, response=response: verify_preflight(name, response))
    elif name.startswith('protector-'):
        count = attempt('semantic', name + ' explicit native-slot semantics', lambda response=response: verify_protector(response))
        if count is not None:
            protector_checks.append(response['checks'])
    elif name == 'native-outbound':
        attempt('semantic', name + ' policy', lambda response=response: require(
            bool(response.get('nativeOutboundError')) or response.get('nativeOutboundStatus', 0) >= 500, 'native fetch escaped deny'))
        attempt('integrity', name + ' deny-service log', lambda: require(
            'QUOD_DENIED_OUTBOUND' in (root / name / 'runtime.log').read_text(), 'deny service was not observed'))
    elif name.startswith('boundary-'):
        mode = name[len('boundary-'):]
        attempt('semantic', name + ' checks', lambda mode=mode, response=response: verify_boundary(mode, response))

executor_names = {path.parent.name for path in root.glob('*/invocation.json')}
if version >= 5:
    executor_names |= set(verified_phases)
for name in sorted(executor_names):
    attempt('integrity', name + ' executor metadata', lambda name=name: verify_executor(name))


if len(preflight_checks) == 2:
    attempt('semantic', 'fresh preflight determinism', lambda: require(
        preflight_checks[0] == preflight_checks[1] and preflight_random[0] == preflight_random[1], 'fresh preflights differ'))
elif result.get('preflightPassed'):
    issues.append({'category': 'semantic', 'check': 'fresh preflight determinism', 'message': 'two verified preflights required'})

policy_receipts = None
if version >= 7:
    attempt('semantic', 'fresh protector determinism', lambda: require(
        len(protector_checks) == 2 and protector_checks[0] == protector_checks[1] and
        result.get('protectorCheckCount') == 51, 'two identical 51-check protector fixtures required'))
    policy_receipts = attempt('semantic', '107 original and 16 cross-policy receipt checks',
                              lambda: verify_policy_receipts(root / 'receipt-checks'))


def verify_fixture(fixture, module=False):
    mode = fixture['mode']
    name = ('module-' if module else 'fixture-') + mode
    response = verified_phases[name]
    report_name = name + '-ordinary.json'
    ordinary = rows(read(report_name))
    require(fixture['ordinaryReportSha256'] == sha(report_name), 'ordinary report hash differs')
    require(fixture['rows'] == ordinary and managed_rows(response['rows']) == ordinary, 'fixture names or states differ')
    if module:
        count, reason = {'literal-import': (1, None), 'computed-import': (1, 'computed module import'),
                         'evaluation': (2, 'dynamic code evaluation'), 'metadata': (3, 'ambient module metadata')}[mode]
        expected_pass, expected_exit, expected_supported = True, 0, reason is None
        unsupported = [reason] if reason else []
    else:
        count, expected_pass, expected_exit = 4, mode != 'failure', 1 if mode == 'failure' else 0
        expected_supported = mode != 'unsupported'
        unsupported = ['global fetch'] if mode == 'unsupported' else []
        require(sum(state == 'failed' for _, state in ordinary) == (1 if mode == 'failure' else 0), 'ordinary failure count changed')
    require(len(ordinary) == count, 'fixture count changed')
    require(fixture['managedPassed'] is expected_pass and response['ok'] is expected_pass, 'fixture pass result differs')
    require(fixture['ordinaryExit'] == expected_exit, 'ordinary fixture exit differs')
    require(fixture['managedSupported'] is expected_supported, 'fixture support classification differs')
    require(fixture['fallbackExecuted'] is (not expected_supported), 'fixture fallback classification differs')
    boundary = response['boundary']
    require(boundary['unsupported'] == unsupported, 'fixture unsupported reason differs')
    require(not boundary['unguardable'] and not boundary['timerErrors'], 'fixture boundary error')
    if version >= 4:
        require(sealed(boundary), 'fixture effects were not sealed')
        require(supported(response) is expected_supported, 'fixture support claim differs from boundary')


for fixture in result.get('fixtureEvidence', []):
    attempt('semantic', 'fixture-' + fixture.get('mode', '<unnamed>'), lambda fixture=fixture: verify_fixture(fixture))
for fixture in result.get('moduleFixtureEvidence', []):
    attempt('semantic', 'module-' + fixture.get('mode', '<unnamed>'), lambda fixture=fixture: verify_fixture(fixture, module=True))
if result.get('fixtureChecksPassed'):
    attempt('semantic', 'ordinary fixture mode inventory', lambda: require(
        [f['mode'] for f in result.get('fixtureEvidence', [])] == ['pass', 'failure', 'unsupported'], 'fixture modes differ'))
if result.get('moduleFixtureChecksPassed'):
    attempt('semantic', 'module fixture mode inventory', lambda: require(
        [f['mode'] for f in result.get('moduleFixtureEvidence', [])] == ['literal-import', 'computed-import', 'evaluation', 'metadata'],
        'module fixture modes differ'))


if version >= 4:
    def verify_boundary_inventory():
        evidence = result.get('boundaryFixtureEvidence', [])
        require(len({row['mode'] for row in evidence}) == len(evidence), 'duplicate boundary fixture evidence')
        for row in evidence:
            mode = row['mode']
            require(row['expected'] == BOUNDARY[mode], 'boundary expectations were changed: ' + mode)
            response = verified_phases['boundary-' + mode]
            require(row['sealed'] is response['boundary']['completion']['sealed'], 'boundary seal claim differs')
            require(row['checks'] == len(response['checks']), 'boundary check count differs')
            require(row['responseSha256'] == sha('boundary-' + mode + '/response.json'), 'boundary evidence hash differs')
        if result.get('boundaryFixtureChecksPassed'):
            modes = [row['mode'] for row in evidence]
            require((len(modes) == 18 and set(modes) == set(BOUNDARY)) if version >= 6 else modes == list(BOUNDARY),
                    'boundary mode inventory differs')
    attempt('integrity', 'boundary fixture inventory', verify_boundary_inventory)

    def verify_compiler_checks():
        directory = root / 'compiler-checks'
        for path in sorted(directory.glob('*.json')):
            if path.name == 'result.json':
                continue
            name = str(path.relative_to(root))
            built = read(name)
            verify_build(name, built, ['compiler-checks/' + path.stem + '.mjs'])
            require(verified_builds[name]['workerFiles'], 'compiler fixture bundle missing')
        if (directory / 'result.json').exists():
            compiler = read('compiler-checks/result.json')
            require(compiler['schema'] == 'quod-managed-compiler-regressions-v1' and compiler['cacheEnabled'] is False,
                    'compiler regression schema differs')
            require(compiler['checks'] and all(check['passed'] for check in compiler['checks']), 'compiler regression failed')
            if version >= 7:
                require(len(compiler['checks']) == 40, 'original 40 compiler checks changed')
            require(len({check['name'] for check in compiler['checks']}) == len(compiler['checks']), 'duplicate compiler check name')
            for bundle in compiler['bundles']:
                built = verified_builds['compiler-checks/' + bundle['name'] + '.json']
                require(bundle['sha256'] == built['sha256'] and bundle['identitySha256'] == built['identitySha256'],
                        'compiler bundle result identity differs')
            return len(compiler['checks'])
        require(not result.get('compilerChecksPassed'), 'compiler regression result missing')
        return 0
    compiler_count = attempt('integrity', 'compiler regression artifacts', verify_compiler_checks)
else:
    compiler_count = None


ordinary_count = None
if (root / 'ordinary-report.json').exists():
    def verify_ordinary():
        ordinary = rows(read('ordinary-report.json'))
        require(len(ordinary) == 47 and all(state == 'passed' for _, state in ordinary), 'ordinary workload states changed')
        require(ordinary == result['ordinary']['rows'], 'ordinary state record differs')
        require(result['ordinary']['reportSha256'] == sha('ordinary-report.json'), 'ordinary report hash differs')
        require(result['ordinary']['exit'] == 0, 'ordinary workload failed')
        require(result['fallbackExecuted'] is (not result['managedSupported']), 'fallback selection differs')
        if result['managedSupported']:
            require(managed_rows(result['managedRows']) == ordinary and result['namesAndStatesAgree'] is True, 'managed workload states differ')
            require(result['managedPassed'] is True, 'managed workload did not pass')
            require(result['testSourceSha256'] == '9798bab303233bca846a89ccb39ce28bd968329e0704f3e5d67adca934c11c75', 'unchanged source pin differs')
            if version >= 4:
                require(supported(verified_phases['managed']), 'managed support claim contradicts boundary')
        return len(ordinary)
    ordinary_count = attempt('semantic', 'ordinary unchanged workload', verify_ordinary)

required_flags = ['preflightPassed', 'nativeOutboundDenied']
if version >= 3 or 'fixtureEvidence' in result:
    required_flags.append('fixtureChecksPassed')
if version >= 4:
    required_flags += ['compilerChecksPassed', 'boundaryFixtureChecksPassed', 'moduleFixtureChecksPassed', 'expensiveTestAttempted']
if version >= 7:
    required_flags += ['protectorChecksPassed', 'receiptChecksPassed']
missing_flags = [flag for flag in required_flags if result.get(flag) is not True]
artifact_complete = not missing_flags and ordinary_count is not None
if missing_flags:
    issues.append({'category': 'gate', 'check': 'completed gate', 'message': 'unfinished gates: ' + ', '.join(missing_flags)})
if ordinary_count is None:
    issues.append({'category': 'gate', 'check': 'ordinary unchanged workload', 'message': 'no verified ordinary workload report'})
if version >= 4 and artifact_complete and result.get('managedSupported') is not True:
    issues.append({'category': 'gate', 'check': 'hardened workload completion', 'message': 'managed workload did not meet the hardened contract'})
if result.get('unsupportedOrSetupError'):
    issues.append({'category': 'gate', 'check': 'reported setup/runtime failure', 'message': result['unsupportedOrSetupError'].get('message', '')})

verified_files = {}
for path in sorted(root.rglob('*')):
    if path.is_file():
        name = str(path.relative_to(root))
        value = attempt('integrity', 'file ' + name, lambda name=name: sha(name))
        if value is not None:
            verified_files[name] = value

integrity_passed = not any(issue['category'] == 'integrity' for issue in issues)
evidence = {
    'auditPassed': not issues,
    'integrityPassed': integrity_passed,
    'artifactComplete': artifact_complete,
    'boundaryChecksVersion': version,
    'issues': issues,
    'resultSha256': verified_files.get('result.json'),
    'preflightChecksPerProcess': check_count,
    'preflightFreshProcesses': len(preflight_checks),
    'protectorChecksPerProcess': 51 if version >= 7 else None,
    'protectorFreshProcesses': len(protector_checks),
    'policyReceiptChecks': policy_receipts,
    'preflightRandomValues': preflight_random[0] if preflight_random else None,
    'compilerRegressionChecks': compiler_count,
    'boundaryFixtureModes': [name[len('boundary-'):] for name in verified_phases if name.startswith('boundary-')],
    'expectedBoundaryFixtureModes': list(BOUNDARY) if version >= 4 else [],
    'ordinaryReportSha256': verified_files.get('ordinary-report.json'),
    'ordinaryTestStates': ordinary_count,
    'managedSupported': result.get('managedSupported'),
    'fallbackExecuted': result.get('fallbackExecuted'),
    'namesAndStatesAgree': result.get('namesAndStatesAgree'),
    'nativeOutboundDenied': result.get('nativeOutboundDenied', False),
    'fixtureEvidence': result.get('fixtureEvidence', []),
    'moduleFixtureEvidence': result.get('moduleFixtureEvidence', []),
    'verifiedBuilds': verified_builds,
    'verifiedPhases': list(verified_phases),
    'verifiedInvocations': verified_invocations,
    'replays': 0,
    'verifiedFiles': verified_files,
}
print(json.dumps(evidence, indent=2))
sys.exit(0 if evidence['auditPassed'] else 1)

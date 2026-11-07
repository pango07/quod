"""Audit externally verified GitHub replay artifacts without executing their code.

Usage: replay-audit.py HARDENING_ROOT SEED_ROOT CONTROL_ROOT CONSUMER_ROOT
The seed root may contain seed/ and receipt-checks/. Archive authenticity and
job provenance are checked by the caller; this checks retained evidence links.
"""
import base64
import decimal
import hashlib
import json
import math
import pathlib
import re
import sys
import urllib.parse

EXECUTION_SCHEMA = 'quod-managed-execution-v1-research'
RECEIPT_SCHEMA = 'quod-managed-receipt-v1-research'
EXPECTED = {'count': 47, 'file': 'server/stream.property.test.ts'}
SOURCE_SHA = '9798bab303233bca846a89ccb39ce28bd968329e0704f3e5d67adca934c11c75'
BASE = {'epoch': 1700000000000, 'seed': 42, 'env': {'NODE_ENV': 'test'}}
COMPLETION_BOUNDS = {'nativeTurnFencesMax': 512, 'timerCallbackBudget': 4096,
                    'timerCallbackBudgetScope': 'callbacks after completion begins; reject when exceeded', 'quietNativeTurns': 2}
issues, hashes, builds, roles = [], {}, {}, {}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def attempt(label, action):
    try:
        return action()
    except (ValueError, TypeError, KeyError, IndexError, OSError, UnicodeError, OverflowError, RecursionError) as error:
        issues.append({'check': label, 'message': str(error)})
        return None


def pairs(values):
    result = {}
    for key, value in values:
        require(key not in result, 'duplicate JSON field: ' + key)
        result[key] = value
    return result


def decode(raw):
    def reject(value):
        raise ValueError('non-finite JSON number: ' + value)
    return json.loads(raw.decode('utf8'), object_pairs_hook=pairs, parse_constant=reject)


def read(path):
    require(not path.is_symlink(), 'symlink is not artifact evidence: ' + str(path))
    return decode(path.read_bytes())


def file_hash(path):
    if path not in hashes:
        require(not path.is_symlink(), 'symlink is not artifact evidence: ' + str(path))
        digest = hashlib.sha256()
        with path.open('rb') as handle:
            for chunk in iter(lambda: handle.read(1024 * 1024), b''):
                digest.update(chunk)
        hashes[path] = digest.hexdigest()
    return hashes[path]


def digest(raw):
    return hashlib.sha256(raw if isinstance(raw, bytes) else raw.encode('utf8')).hexdigest()


def valid_sha(value):
    return isinstance(value, str) and re.fullmatch('[0-9a-f]{64}', value) is not None


def exact(value, keys, label):
    require(isinstance(value, dict) and set(value) == set(keys), label + ' fields differ')


def integer(value):
    return type(value) is int and 0 <= value <= 9007199254740991


def number(value):
    require(math.isfinite(value), 'non-finite canonical number')
    require(value != 0 or math.copysign(1, value) > 0, 'negative zero is not canonical')
    if type(value) is int:
        require(abs(value) <= 9007199254740991, 'unsafe canonical integer')
        return str(value)
    text = repr(value).lower()
    absolute = abs(value)
    if absolute == 0:
        return '0'
    if 1e-6 <= absolute < 1e21:
        text = format(decimal.Decimal(text), 'f')
        return text.rstrip('0').rstrip('.') if '.' in text else text
    mantissa, exponent = text.split('e') if 'e' in text else (text, '0')
    mantissa = mantissa[:-2] if mantissa.endswith('.0') else mantissa
    exponent = int(exponent)
    return mantissa + 'e' + ('+' if exponent >= 0 else '') + str(exponent)


def quoted(value):
    encoded = json.dumps(value, ensure_ascii=False)
    # JSON.stringify escapes lone UTF-16 surrogates but leaves Unicode scalar
    # values intact. The JSON decoder already combines valid surrogate pairs.
    return ''.join('\\u%04x' % ord(char) if 0xd800 <= ord(char) <= 0xdfff else char for char in encoded)


def encode(value, sort=True):
    if value is None:
        return 'null'
    if type(value) is bool:
        return 'true' if value else 'false'
    if isinstance(value, str):
        return quoted(value)
    if type(value) in (int, float):
        return number(value)
    if isinstance(value, list):
        return '[' + ','.join(encode(item, sort) for item in value) + ']'
    require(isinstance(value, dict) and all(isinstance(key, str) for key in value), 'non-data canonical JSON')
    keys = sorted(value, key=lambda key: key.encode('utf-16-be', 'surrogatepass')) if sort else value
    return '{' + ','.join(quoted(key) + ':' + encode(value[key], sort) for key in keys) + '}'


def canonical_hash(value):
    return digest(encode(value))


def discover(root, predicate, label):
    matches = []
    for path in root.rglob('result.json'):
        value = read(path)
        if predicate(value):
            matches.append((path.parent, value))
    require(len(matches) == 1, label + ' needs exactly one matching result.json, found ' + str(len(matches)))
    return matches[0]


def symbol_inventory(boundary):
    require(boundary['symbolUnguardable'] == [], 'unguardable native symbols')
    require(isinstance(boundary['symbolInventory'], list), 'missing symbol inventory')
    for row in boundary['symbolInventory']:
        exact(row, ['owner', 'ownerIndex', 'name', 'descriptor', 'valueType', 'primitiveValue',
                    'enumerable', 'configurable', 'writable', 'policy'], 'symbol descriptor')
        require(isinstance(row['owner'], str) and integer(row['ownerIndex']) and isinstance(row['name'], str), 'non-primitive symbol label')
        require(isinstance(row['primitiveValue'], str) and all(type(row[key]) is bool for key in ('enumerable', 'configurable', 'writable')),
                'non-primitive symbol descriptor')
        require(row['policy'] in ('fixed primitive data', 'guarded'), 'symbol policy is unsupported')
        if row['policy'] == 'fixed primitive data':
            require(row['descriptor'] == 'data' and row['valueType'] in (
                'undefined', 'null', 'boolean', 'string', 'number', 'bigint', 'symbol'), 'symbol exposes a native reference')
        else:
            require(row['descriptor'] in ('data', 'accessor') and row['primitiveValue'] == '', 'invalid guarded symbol')


def boundary_complete(boundary, allow_denied=False):
    exact(boundary, ['inventory', 'guarded', 'unguardable', 'symbolInventory', 'symbolUnguardable', 'unsupported',
                     'timerErrors', 'unhandledErrors', 'pendingTimers', 'pendingTimerDetails', 'completion',
                     'virtualTime', 'logs', 'replayEligible', 'contract'], 'boundary')
    require(boundary['contract'] == 'quod-managed-v0-investigation' and boundary['replayEligible'] is False,
            'production admission policy changed')
    for key in ('inventory', 'guarded', 'unguardable', 'unsupported', 'timerErrors', 'unhandledErrors'):
        require(isinstance(boundary[key], list) and all(isinstance(item, str) for item in boundary[key]), 'invalid ' + key)
    for key in ('unguardable', 'timerErrors', 'unhandledErrors'):
        require(boundary[key] == [], 'boundary has ' + key)
    if not allow_denied:
        require(boundary['unsupported'] == [], 'unsupported execution cannot replay')
    symbol_inventory(boundary)
    require(type(boundary['pendingTimers']) is int and boundary['pendingTimers'] == 0 and
            boundary['pendingTimerDetails'] == [], 'pending managed timers')
    completion = boundary['completion']
    exact(completion, ['sealed', 'nativeTurnFences', 'timerCallbacks', 'pendingTimerCallbacks', 'pendingMicrotasks',
                       'pendingEventCallbacks', 'intrinsicLockdown', 'protectedIntrinsicObjects', 'reason'], 'completion')
    require(completion['sealed'] is True and completion['intrinsicLockdown'] is True and completion['reason'] == '', 'completion is not sealed')
    require(integer(completion['protectedIntrinsicObjects']) and completion['protectedIntrinsicObjects'] > 0, 'empty intrinsic policy')
    require(integer(completion['nativeTurnFences']) and 2 <= completion['nativeTurnFences'] <= 512 and
            integer(completion['timerCallbacks']), 'invalid completion counters')
    for key in ('pendingTimerCallbacks', 'pendingMicrotasks', 'pendingEventCallbacks'):
        require(completion[key] == 0 and type(completion[key]) is int, 'pending callback: ' + key)
    require(type(boundary['virtualTime']) in (int, float) and math.isfinite(boundary['virtualTime']) and boundary['virtualTime'] >= 0,
            'invalid virtual time')
    methods = {'log', 'info', 'warn', 'error', 'debug', 'trace', 'dir', 'assert', 'time', 'timeEnd', 'timeLog'}
    require(isinstance(boundary['logs'], list), 'missing console effects')
    for row in boundary['logs']:
        require(isinstance(row, list) and len(row) == 2 and row[0] in methods and isinstance(row[1], list) and
                all(isinstance(item, str) for item in row[1]), 'non-primitive console effect')


def passing_response(data):
    exact(data, ['ok', 'rows', 'files', 'boundary'], 'worker response')
    require(data['ok'] is True and isinstance(data['rows'], list) and len(data['rows']) == 47, 'incomplete passing workload')
    names = []
    for row in data['rows']:
        exact(row, ['name', 'state', 'errors'], 'test row')
        require(isinstance(row['name'], str) and row['name'] and row['state'] == 'pass' and row['errors'] == [], 'non-passing test row')
        names.append(row['name'])
    require(len(set(names)) == 47, 'duplicate test names')
    require(data['files'] == [{'name': EXPECTED['file'], 'state': 'pass', 'errors': []}], 'incomplete passing file')
    boundary_complete(data['boundary'])


def verify_build(path, available_workers):
    built = read(path)
    identity = built['identity']
    require(identity['schema'] == 'quod-managed-compile-v1-research', 'obsolete compiler identity')
    require(valid_sha(built['sha256']) and identity['bundleSha256'] == built['sha256'], 'bundle identity differs')
    require(built['identitySha256'] == canonical_hash(identity), 'compiler canonical identity SHA differs')
    require(built['sha256'] in available_workers, 'no retained compiled bundle matches build SHA')
    require(valid_sha(identity['compilerSha256']), 'missing compiler implementation identity')
    options = built.get('options', BASE)
    if path.parent.name == 'admission-integration' and path.name == 'baseline-build.json':
        options = {'epoch': 1700000000000, 'seed': 42,
                   'env': {'NODE_ENV': 'test', 'QUOD_RECEIPT_VALUE': 'declared'},
                   'vitest': {'root': '/project/.quod-managed-receipt-fixtures', 'sequence': {'seed': 42}},
                   'file': '/project/.quod-managed-receipt-fixtures/fixture.test.mjs'}
    require(identity['configurationSha256'] == canonical_hash(options), 'compiler options SHA differs')
    require(identity['globs'] == built['globs'] and identity['adapters'] == built['adapters'], 'compiler collection identity differs')
    require(identity['moduleNameSetsSha256'] == canonical_hash(built['globs']), 'module-name set SHA differs')
    require(identity['sources'] and all(isinstance(row['source'], str) and valid_sha(row['sourceSha256']) and
            valid_sha(row['compiledSourceSha256']) for row in identity['sources']), 'invalid module source identity')
    imports = [row for output in built['metadata']['outputs'].values() for row in output['imports']]
    require(all(row['path'] == 'node:async_hooks' and row['external'] is True for row in imports), 'unreviewed native import')
    closure = built['closure']
    require(closure['finalBundleParsed'] is True and closure['remainingDynamicImports'] == 0 and
            closure['remainingDynamicRequires'] == 0 and all(row == 'node:async_hooks' for row in closure['externalImports']),
            'module closure is incomplete')
    if 'preparation' in built:
        preparation = built['preparation']
        require(preparation['schema'] == 'quod-managed-preparation-v1-research' and valid_sha(preparation['preparerSha256']),
                'invalid preparation identity')
        require(preparation['configurationSha256'] == canonical_hash(built['options']['vitest']), 'prepared configuration SHA differs')
        require(read(path.parent / 'project-selection.json')['identity'] == preparation, 'preparation records differ')
        require(all(valid_sha(row['sha256']) for row in preparation['sources']), 'invalid preparation source SHA')
        require(any(row['source'] == '/project/packages/convex-helpers/server/stream.property.test.ts' and
                    row['sourceSha256'] == SOURCE_SHA for row in identity['sources']), 'unchanged test source pin missing')
    builds[path] = built


def verify_invocation(directory, phase, build):
    name = phase['name']
    require(re.fullmatch('[a-z][a-z0-9-]*', name) is not None, 'unsafe phase name')
    folder = directory / name
    raw = (folder / 'response.json').read_bytes()
    data = decode(raw)
    require(phase['data'] == data and phase['responseSha256'] == digest(raw), 'phase response record differs')
    require(file_hash(folder / 'worker.mjs') == build['sha256'], 'executed bundle SHA differs')
    invocation = read(folder / 'invocation.json')
    require(invocation['schema'] == 'quod-managed-invocation-v1-research' and invocation['name'] == name, 'invocation identity differs')
    require(invocation['bundleSha256'] == build['sha256'] and invocation['configSha256'] == file_hash(folder / 'config.capnp'),
            'actual invocation byte identities differ')
    policy = invocation['runtimePolicy']
    require(policy['schema'] == 'quod-managed-runtime-policy-v1-research' and valid_sha(policy['workerdSha256']), 'runtime binary identity missing')
    require(policy['denyModuleSha256'] == file_hash(folder / 'deny.mjs'), 'outbound deny module SHA differs')
    require(policy['compatibilityDate'] == '2026-07-01' and policy['compatibilityFlags'] == ['nodejs_als'] and
            policy['moduleName'] == 'worker.mjs' and policy['services'] == ['main', 'deny'] and
            policy['globalOutbound'] == 'deny' and policy['bindings'] == [], 'runtime capability policy differs')
    require(invocation['env'] == policy['processEnv'], 'actual process environment differs')
    exact(invocation['env'], ['PATH', 'TZ', 'LANG'], 'runtime environment')
    require(isinstance(invocation['env']['PATH'], str) and invocation['env']['PATH'] and
            invocation['env']['TZ'] == 'UTC' and invocation['env']['LANG'] == 'C.UTF-8', 'undeclared process environment')
    require(policy['request'] == {'method': 'GET', 'path': '/run'}, 'runtime request policy differs')
    url = urllib.parse.urlsplit(invocation['request']['url'])
    require(invocation['request']['method'] == 'GET' and url.scheme == 'http' and url.hostname == '127.0.0.1' and url.port and
            url.path == '/run' and not url.query and not url.fragment, 'actual HTTP request differs')
    require(invocation['args'] == ['serve', invocation['args'][1]] and invocation['args'][1].endswith('/' + name + '/config.capnp'),
            'actual runtime arguments differ')
    config = (folder / 'config.capnp').read_text()
    require('compatibilityDate = "2026-07-01"' in config and 'compatibilityFlags = ["nodejs_als"]' in config and
            'globalOutbound = "deny"' in config and 'nodejs_compat' not in config and 'disk =' not in config,
            'retained runtime capabilities differ')
    supervisor = policy['supervisor']
    require(valid_sha(supervisor['sourceSha256']) and valid_sha(supervisor['nodeExecutableSha256']), 'supervisor byte identities missing')
    require(supervisor['completionBounds'] == COMPLETION_BOUNDS, 'completion policy differs')
    require(all(integer(supervisor[key]) and supervisor[key] > 0 for key in (
        'watchdogMs', 'startupAttempts', 'healthTimeoutMs', 'startupRetryMs', 'shutdownGraceMs', 'responseBytesMax')), 'unbounded supervisor')
    execution, meta = read(folder / 'execution.json'), read(folder / 'response-meta.json')
    require(execution['name'] == name and execution['completed'] is True and execution['timedOut'] is False and
            execution['error'] is None and execution['exit'] and execution['exit']['spawnError'] is None, 'incomplete worker execution')
    require(execution['responseSha256'] == digest(raw) and meta['sha256'] == digest(raw) and meta['bytes'] == len(raw) and
            200 <= meta['status'] <= 299 and len(raw) <= supervisor['responseBytesMax'], 'response bytes or transport status differs')
    host = invocation['host']
    exact(host, ['ImageOS', 'ImageVersion', 'platform', 'arch', 'kernel', 'libc', 'nodeVersion'], 'host class')
    require(all(isinstance(host[key], str) and host[key] for key in ('ImageOS', 'ImageVersion', 'platform', 'arch', 'nodeVersion')),
            'host class is incomplete')
    exact(host['kernel'], ['release', 'version'], 'kernel class')
    exact(host['libc'], ['family', 'runtimeVersion', 'compilerVersion'], 'libc class')
    require(host['libc']['family'] != 'unknown' and host['nodeVersion'] == supervisor['nodeVersion'], 'unknown host/runtime class')
    return {'raw': raw, 'data': data, 'policy': policy, 'host': host}


def verify_execution(execution):
    exact(execution, ['identity', 'key'], 'execution descriptor')
    identity = execution['identity']
    exact(identity, ['schema', 'compile', 'preparation', 'runtimePolicy', 'supervisorPolicy', 'host'], 'execution identity')
    require(identity['schema'] == EXECUTION_SCHEMA and execution['key'] == digest(EXECUTION_SCHEMA + '\n' + encode(identity)),
            'execution domain/key differs')
    return identity


def verify_role(role, directory, result):
    require(result['schema'] == 'quod-managed-fresh-runner-v1-research' and result['role'] == role and
            result['passed'] is True and result['productionEnabled'] is False and not result.get('error'), 'research role failed or production enabled')
    phases = result['phases']
    expected_phases = ['preflight'] if role == 'replay' else ['preflight', 'managed']
    require([row['name'] for row in phases] == expected_phases, 'unexpected worker execution phases')
    actual_workers = {path.parent.name for path in directory.glob('*/worker.mjs')}
    require(actual_workers == set(expected_phases), 'unrecorded worker execution directory')
    require(type(result['actualManagedExecutions']) is int and result['actualManagedExecutions'] == (0 if role == 'replay' else 1),
            'managed execution count differs')
    require(result['replayed'] is (role == 'replay'), 'replay outcome flag differs')
    if role == 'replay':
        require(result['hit'] is True and result['namesStatesAndEffectsAgree'] is True and not result.get('missReason'), 'consumer did not genuinely hit')
        require(not (directory / 'managed').exists() and not (directory / 'managed-miss').exists(), 'consumer executed a managed fallback')
    preflight_build, test_build = builds[directory / 'preflight-build.json'], builds[directory / 'test-build.json']
    observed = [verify_invocation(directory, row, preflight_build if row['name'] == 'preflight' else test_build) for row in phases]
    preflight = observed[0]['data']
    require(preflight['ok'] is True and len(preflight['checks']) == 31 and all(row['pass'] is True for row in preflight['checks']),
            'consumer capability preflight failed')
    boundary_complete(preflight['boundary'], allow_denied=True)
    require(set(preflight['boundary']['unsupported']) == {'global fetch', 'global crypto', 'global WeakRef',
        'global FinalizationRegistry', 'Event.timeStamp', 'global WebAssembly', 'AbortSignal.timeout', 'eval',
        'Function constructor', 'AsyncFunction constructor', 'GeneratorFunction constructor', 'AsyncGeneratorFunction constructor',
        'global EventTarget.addEventListener', 'global EventTarget.removeEventListener', 'global EventTarget.dispatchEvent'},
        'preflight private denial inventory differs')
    execution = read(directory / 'execution-identity.json')
    identity = verify_execution(execution)
    require(result['executionKey'] == execution['key'] and result['host'] == identity['host'], 'role execution records differ')
    require(identity['compile'] == test_build['identity'] and identity['preparation'] == test_build['preparation'], 'key omits actual compilation/preparation')
    native_sha = canonical_hash({'preflightIdentity': preflight_build['identitySha256'],
                                'inventory': preflight['boundary']['inventory'], 'symbols': preflight['boundary']['symbolInventory']})
    policy = dict(identity['runtimePolicy'])
    require(policy.pop('nativeFingerprintSha256') == native_sha, 'preflight native fingerprint differs')
    require(all(row['policy'] == policy and row['host'] == identity['host'] for row in observed), 'actual runtime policy/host differs from key')
    require(all(valid_sha(identity['supervisorPolicy'][key]) for key in ('receiptValidatorSha256', 'experimentSha256')), 'supervisor source policy missing')
    selected = read(directory / 'selected-response.json')
    passing_response(selected)
    if role != 'replay':
        require(selected == observed[1]['data'], 'selected execution response differs')
    effects = {'ok': selected['ok'], 'rows': selected['rows'], 'files': selected['files'],
               'logs': selected['boundary']['logs'], 'virtualTime': selected['boundary']['virtualTime']}
    require(result['effects'] == effects and result['effectSha256'] == canonical_hash(effects), 'reported effects/digest differ')
    require(all(type(result[key]) in (int, float) and math.isfinite(result[key]) and result[key] >= 0 for key in (
        'commandSeconds', 'preparationSeconds', 'compilationSeconds', 'preReplaySeconds')), 'missing finite role cost')
    return {'directory': directory, 'result': result, 'execution': execution, 'selected': selected, 'effects': effects,
            'preflightBuild': preflight_build, 'observed': observed, 'nativeSha256': native_sha}


def compiler_names():
    names = set()
    for mode in ('literal-import', 'computed-import', 'evaluation', 'metadata'):
        names |= {mode + ' final AST is closed', mode + ' all native imports use ALS only'}
        if mode != 'literal-import':
            names.add(mode + ' injects private sticky rejection')
    names |= {'unchanged preparation identity repeats', 'literal dependency is recorded', 'declared configuration changes identity',
              'literal dependency bytes change identity', 'unrelated file leaves closed identity unchanged', 'glob names are recorded',
              'glob records resolved module targets', 'glob module-name mutation changes identity',
              'resolved symlink to private adapter rejects preparation', 'named ALS uses the managed facade',
              'bare native ALS alias uses the managed facade', 'configuration key order is canonical'}
    for name in ('private-namespace', 'private-generated-namespace', 'private-adapter', 'unused-private-adapter', 'native-fs',
                 'native-alias', 'native-prefix-only', 'als-namespace', 'als-default', 'als-extra', 'als-reexport',
                 'als-alias-namespace', 'als-alias-extra', 'als-dynamic', 'native-require'):
        names.add(name + ' rejects preparation')
    for name in ('type-private-namespace', 'type-private-adapter'):
        names.add(name + ' rejects original source before erasure')
    return names


def receipt_names():
    names = {'unchanged validated receipt hits', 'exact response bytes are preserved', 'complete primitive console effects are preserved',
             'missing trusted seed digest misses', 'wrong trusted seed digest misses', 'missing explicit receipt misses',
             'a receipt cannot overwrite existing seed', 'unrelated input absent from execution identity hits',
             'changed expected count misses', 'changed expected file misses', 'truncated receipt misses', 'duplicate receipt fields miss',
             'duplicate worker response fields cannot be recorded', 'own accessor rejects without invocation',
             'bundle digest mismatch rejects identity', 'configuration digest mismatch rejects identity'}
    for name in ('bundle', 'configuration', 'declared environment', 'module-name set', 'runtime binary', 'native fingerprint',
                 'runtime flags', 'supervisor', 'validator', 'host'):
        names |= {name + ' changes execution key', name + ' misses old receipt'}
    for name in ('test failure', 'passing flag with failed test', 'file failure', 'test error', 'file error', 'error response',
                 'missing test row', 'unknown test state', 'unsupported operation', 'unguardable native alias',
                 'unguardable native symbol', 'unsealed completion', 'missing intrinsic lockdown', 'pending timer',
                 'pending timer callback', 'pending microtask', 'pending event callback', 'async timer failure',
                 'unhandled rejection', 'obsolete worker response', 'non-primitive console effect', 'unexpected receipt field'):
        names |= {name + ' rejects validation', name + ' cannot be recorded'}
    for name in ('obsolete receipt schema', 'extra receipt field', 'coherent response tampering', 'wrong response checksum', 'wrong result domain'):
        names.add(name + ' misses')
    for group, key in (('runtimePolicy', 'workerdSha256'), ('runtimePolicy', 'nativeFingerprintSha256'), ('runtimePolicy', 'supervisor'),
                       ('runtimePolicy', 'globalOutbound'), ('supervisorPolicy', 'receiptValidatorSha256'), ('host', 'kernel')):
        names.add(group + ' omitted ' + key + ' rejects identity')
    return names


def matrix(path, schema, field, required):
    value = read(path)
    require(value['schema'] == schema and value['cacheEnabled'] is False, 'fixture matrix schema/policy differs')
    checks = value['checks']
    require(checks and all(row[field] is True for row in checks), 'fixture matrix contains a failure')
    names = [row['name'] for row in checks]
    require(len(set(names)) == len(names) and required <= set(names), 'fixture matrix is incomplete')
    return value


def verify_receipt(seed, replay):
    path = seed['directory'] / 'receipt.json'
    raw, metadata = path.read_bytes(), seed['result']['receipt']
    require(0 < len(raw) <= 24 * 1024 * 1024 and metadata['receiptSha256'] == digest(raw), 'trusted seed receipt SHA differs')
    receipt = decode(raw)
    require(encode(receipt).encode('utf8') == raw, 'receipt bytes are not canonical')
    exact(receipt, ['schema', 'researchOnly', 'execution', 'expected', 'response', 'resultDigest'], 'receipt')
    require(receipt['schema'] == RECEIPT_SCHEMA and receipt['researchOnly'] is True, 'obsolete receipt policy')
    require(receipt['execution'] == seed['execution'] and receipt['expected'] == EXPECTED, 'receipt execution/expectation differs')
    require(verify_execution(receipt['execution']) == seed['execution']['identity'], 'receipt key differs')
    exact(receipt['response'], ['encoding', 'bytes', 'sha256'], 'stored worker response')
    require(receipt['response']['encoding'] == 'base64', 'receipt response encoding differs')
    worker_raw = base64.b64decode(receipt['response']['bytes'], validate=True)
    require(base64.b64encode(worker_raw).decode() == receipt['response']['bytes'], 'non-canonical response base64')
    require(0 < len(worker_raw) <= 16 * 1024 * 1024 and receipt['response']['sha256'] == digest(worker_raw), 'receipt worker bytes/SHA differ')
    require(receipt['resultDigest'] == digest(RECEIPT_SCHEMA + '\n' + seed['execution']['key'] + '\n' + receipt['response']['bytes']),
            'receipt result domain digest differs')
    require(metadata['key'] == seed['execution']['key'] and metadata['responseSha256'] == digest(worker_raw) and metadata['bytes'] == len(raw),
            'seed receipt metadata differs')
    worker = decode(worker_raw)
    require(encode(worker, sort=False).encode('utf8') == worker_raw, 'worker response is not compact exact JSON')
    passing_response(worker)
    require(worker_raw == (seed['directory'] / 'managed/response.json').read_bytes(), 'receipt differs from actual seed response bytes')
    require(worker_raw == (replay['directory'] / 'replayed-response.json').read_bytes(), 'consumer did not preserve receipt bytes')
    require(worker == seed['selected'] == replay['selected'], 'receipt selection differs')
    return {'receiptSha256': digest(raw), 'responseSha256': digest(worker_raw), 'receiptBytes': len(raw)}


if len(sys.argv) != 5:
    print(json.dumps({'auditPassed': False, 'issues': [{'check': 'arguments', 'message': __doc__}]}))
    sys.exit(1)
roots = dict(zip(('hardening', 'seed', 'control', 'replay'), (pathlib.Path(value).resolve() for value in sys.argv[1:])))
located = {}
for role, root in roots.items():
    predicate = (lambda value: 'boundaryChecksVersion' in value) if role == 'hardening' else (lambda value, role=role: value.get('role') == role)
    found = attempt(role + ' artifact layout', lambda root=root, predicate=predicate, role=role: discover(root, predicate, role))
    if found:
        located[role] = found

available_workers = set()
for root in roots.values():
    for path in sorted(root.rglob('*.mjs')):
        if (path.name == 'worker.mjs' or path.parent.name == 'compiler-checks' or
                path.name == 'baseline-bundle.mjs' and path.parent.name == 'admission-integration'):
            value = attempt('bundle bytes ' + str(path), lambda path=path: file_hash(path))
            if value:
                available_workers.add(value)
for root in roots.values():
    paths = set(root.rglob('*-build.json'))
    paths |= {path for path in root.rglob('*.json') if path.parent.name == 'compiler-checks' and path.name != 'result.json'}
    for path in sorted(paths):
        attempt('compiled identity ' + str(path), lambda path=path: verify_build(path, available_workers))

for role in ('seed', 'control', 'replay'):
    if role in located:
        directory, result = located[role]
        verified = attempt(role + ' execution', lambda role=role, directory=directory, result=result: verify_role(role, directory, result))
        if verified:
            roles[role] = verified

compiler_count = receipt_count = integration_count = None
if 'hardening' in located:
    hardening_dir, hardening = located['hardening']
    def hardening_matrix():
        require(hardening['cacheEnabled'] is False and all(hardening.get(flag) is True for flag in (
            'compilerChecksPassed', 'preflightPassed', 'nativeOutboundDenied', 'boundaryFixtureChecksPassed',
            'fixtureChecksPassed', 'moduleFixtureChecksPassed', 'expensiveTestAttempted', 'managedSupported', 'namesAndStatesAgree')),
            'separate hardening gate was not complete')
        value = matrix(hardening_dir / 'compiler-checks/result.json', 'quod-managed-compiler-regressions-v1', 'passed', compiler_names())
        for bundle in value['bundles']:
            built = builds[hardening_dir / 'compiler-checks' / (bundle['name'] + '.json')]
            require(bundle['sha256'] == built['sha256'] and bundle['identitySha256'] == built['identitySha256'], 'compiler matrix bundle record differs')
        return len(value['checks'])
    compiler_count = attempt('hardening/compiler fixture matrix', hardening_matrix)
    if hardening.get('boundaryChecksVersion', 1) >= 6:
        def admission_integration():
            folder = hardening_dir / 'receipt-checks'
            required = receipt_names() | {
                'admission integration records actual dependency source',
                'admission integration records actual glob module names',
                'admission integration repeated real compilation keeps identity',
                'admission integration repeated real compilation hits receipt',
                'admission integration unrelated file leaves real compile identity unchanged',
                'admission integration unrelated file hits recorded receipt',
            }
            names = {'test source comment', 'dependency value', 'glob module-name addition',
                     'declared environment', 'configuration seed'}
            for name in names:
                required |= {'admission integration ' + name + ' changes recompiled identity',
                             'admission integration ' + name + ' misses recorded receipt'}
            value = matrix(folder / 'result.json', 'quod-managed-receipt-fixtures-v1', 'pass', required)
            require(value['researchOnly'] is True and value['cacheEnabled'] is False, 'integration enabled production')
            detail = read(folder / 'admission-integration/result.json')
            require(detail == value['admissionIntegration'] and detail['runtimeExecuted'] is False and
                    detail['responseOrigin'] == 'constructed passing response; not runtime execution' and
                    detail['preparationOrigin'] == 'synthetic own-data configuration', 'integration scope differs')
            built = builds[folder / 'admission-integration/baseline-build.json']
            require(detail['compileIdentitySha256'] == built['identitySha256'], 'actual baseline compiler identity differs')
            receipt_file = folder / 'admission-integration/constructed-response-receipt.json'
            require(detail['receipt']['receiptSha256'] == file_hash(receipt_file), 'constructed receipt byte digest differs')
            receipt = read(receipt_file)
            execution = receipt['execution']
            require(execution['identity']['compile'] == built['identity'], 'constructed receipt uses different compilation')
            key = digest('quod-managed-execution-v1-research\n' + encode(execution['identity']))
            require(key == execution['key'] == detail['baselineExecutionKey'], 'baseline ledger key differs')
            require({row['name'] for row in detail['mutations']} == names and len(detail['mutations']) == 5,
                    'actual-input mutation inventory differs')
            for row in detail['mutations']:
                require(row['hit'] is False and valid_sha(row['executionKey']) and row['executionKey'] != key and
                        valid_sha(row['compileIdentitySha256']) and row['compileIdentitySha256'] != built['identitySha256'],
                        'actual-input mutation admitted the old receipt')
            require(detail['unrelated'] == {'executionKey': key, 'hit': True}, 'unrelated real-file change did not hit')
            return len(value['checks'])
        integration_count = attempt('actual compiler-to-ledger mutation integration', admission_integration)
if 'seed' in located:
    def receipt_matrix():
        path, value = discover(roots['seed'], lambda value: value.get('schema') == 'quod-managed-receipt-fixtures-v1', 'receipt matrix')
        result = matrix(path / 'result.json', 'quod-managed-receipt-fixtures-v1', 'pass', receipt_names())
        require(result['researchOnly'] is True, 'receipt matrix enabled production admission')
        require(result['seed']['receiptSha256'] == file_hash(path / 'seed-receipt.json'), 'fixture seed byte digest differs')
        return len(result['checks'])
    receipt_count = attempt('receipt mutation/failure matrix', receipt_matrix)

receipt_summary = None
if set(roles) == {'seed', 'control', 'replay'}:
    def compare_roles():
        seed, control, replay = (roles[key] for key in ('seed', 'control', 'replay'))
        require(seed['execution'] == control['execution'] == replay['execution'], 'execution policies, inputs or host classes differ')
        require(seed['effects'] == control['effects'] == replay['effects'], 'all 47 names/states/files/console effects differ')
        require(seed['nativeSha256'] == control['nativeSha256'] == replay['nativeSha256'], 'native fingerprints differ')
        if 'hardening' in located:
            hardening_dir, _ = located['hardening']
            require(seed['preflightBuild']['identitySha256'] == builds[hardening_dir / 'preflight-build.json']['identitySha256'],
                    'hardening and replay preflight source closures differ')
            require(seed['execution']['identity']['compile'] == builds[hardening_dir / 'test-build.json']['identity'],
                    'hardening and replay workload source closures differ')
        return verify_receipt(seed, replay)
    receipt_summary = attempt('fresh-role equality and trusted receipt bytes', compare_roles)

verified_files = {}
for role, root in roots.items():
    role_files = {}
    for path in sorted(root.rglob('*')):
        if path.is_file():
            value = attempt('artifact file ' + str(path), lambda path=path: file_hash(path))
            if value:
                role_files[str(path.relative_to(root))] = value
    verified_files[role] = role_files
summary = {
    'auditPassed': not issues and set(roles) == {'seed', 'control', 'replay'} and receipt_summary is not None,
    'issues': issues,
    'executionKey': roles.get('seed', {}).get('execution', {}).get('key'),
    'trustedReceipt': receipt_summary,
    'compilerRegressionChecks': compiler_count,
    'receiptMutationChecks': receipt_count,
    'compilerAdmissionChecks': integration_count,
    'passingTestsCompared': 47 if receipt_summary else None,
    'effectsSha256': roles.get('seed', {}).get('result', {}).get('effectSha256'),
    'freshConsumerHit': roles.get('replay', {}).get('result', {}).get('hit'),
    'consumerManagedExecutions': roles.get('replay', {}).get('result', {}).get('actualManagedExecutions'),
    'productionEnabled': False,
    'compiledBuildRecords': len(builds),
    'roleCommandSeconds': {role: record['result']['commandSeconds'] for role, record in roles.items()},
    'verifiedFiles': verified_files,
}
print(json.dumps(summary, indent=2))
sys.exit(0 if summary['auditPassed'] else 1)

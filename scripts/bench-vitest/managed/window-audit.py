"""Independently audit the original cold managed Vitest window, as data only.

Usage: window-audit.py COORDINATOR_ROOT MANAGED_INDEX.json
The caller verifies GitHub archive digests before extraction. The index is
{schema:"quod-managed-window-artifacts-v1", artifacts:[{run,variant,i,commit,
root,artifactId,archiveSha256,archiveSizeBytes?,artifactName?}]}. There must be
24 managed/control artifacts. Roots may be absolute or relative to the index.
Only a fixed trusted Node expression is executed, to reproduce the harness's
localeCompare JSON fingerprint; no source or command from an artifact runs.
"""
import base64
import collections
import datetime
import decimal
import hashlib
import json
import math
import os
import pathlib
import re
import shutil
import subprocess
import sys
import urllib.parse

VARIANTS = ('floor', 'plain', 'managed-control', 'managed')
SHAS = (
    '1d7c57881d153d366becbc5d8351276b0d09b086', '290b146e0793f0d689150f3c53a1f1c7269020d1',
    '1815d4a72a9714311dfed82a101d8c817830fb0e', '03ffe84ef4431d7b02dd0e6903e36c461ed9223e',
    '6b17e1514f5222e9c019e177e36384df95a3ff25', '49a82e54416f9d7957dcbdf5953f628a54693355',
    '51f7bf84887625ae8fadbc29f1f9a028e271c8bb', 'f5c2b9e9ceb4c477e7216ab514ce0d370c3af311',
    'd244cf22ddd8d5a12e4661edfa1ba6cc09468cb8', 'c976cabdff07b9fc671473a3f2020b0ded436d2f',
    '6f830b66f7cdf80c969d21f9ee8cf49d35d64be2', '5eaa7e5160736054d53f8b71e2c522835eb427e0',
)
STATE_SHAS = ('4ecbdc19ec078761acb2d9535ee26e225508cf16431b2d600e003924c65ddc65',) * 7 + (
    '2610c747ae4bdae19f855bc06bf42eb1a537ce71e1c3302155774d9529b08494',) + (
    '09ebb9e3a722220d3a21ddfe9f63992a8a074b0fcaa95764e54c37be90d469cd',) * 4
CANDIDATE = 'packages/convex-helpers/server/stream.property.test.ts'
EXPECTED = {'count': 47, 'file': 'server/stream.property.test.ts'}
SOURCE_SHA = '9798bab303233bca846a89ccb39ce28bd968329e0704f3e5d67adca934c11c75'
# Pinned Linux binary from the independently verified c96 hardening artifact.
WORKERD_SHA = 'a4d954a9ec17a367cf7d5509a2282785e84dacca6fc13b69c532ab25d157ef48'
# Final remote build pins, supplied after build 42391658097 passed 22 checks.
# A downloaded manifest or index may not choose these expected versions.
ACTION_SHA = 'd79c8635f4dbc93c842ec1da8bd5ee6bf8e72928'
HARNESS_SHA = '82f59245d12162ce1d998f0bc4eb068db38acc3d'
MANAGED_SOURCE_SHA = '2a4d6f2f7547dd00e720a32d5b451229784ba8fb'
RUNTIME_ARCHIVE_SHA = 'c3c10a88a496c9d155178d560e1be24170e41b31ffe03848aa39a98a1addb10b'
EXECUTION_SCHEMA = 'quod-managed-execution-v1-research'
RECEIPT_SCHEMA = 'quod-managed-receipt-v1-research'
BASE = {'epoch': 1700000000000, 'seed': 42, 'env': {'NODE_ENV': 'test'}}
BOUNDS = {'nativeTurnFencesMax': 512, 'timerCallbackBudget': 4096,
          'timerCallbackBudgetScope': 'callbacks after completion begins; reject when exceeded', 'quietNativeTurns': 2}
PREPARATION_SOURCES = {
    'vitest.config.mts': '8b67460e3582eaf8d6a7209d5f7f8f89e8a44fcf6133a561c42f87b9dce52b79',
    'scripts/zodVersions.mjs': '19bfcf628785773af81477811d18e4721262dd5896411e7bfbf0d9bf285d0714',
    'packages/convex-helpers/vitest.config.mts': '6ebdf10bf5ffbf225b3cc3c6f47a4efc91ce7b5c068ba3031aa2438ea602d945',
    'example-ui/vite.config.mts': '54af8d06f01ed257ee4d5d5af7e08a2c7adc1022adcbaa370efcd42e2d82a953',
}
FINGERPRINT_JS = """import fs from 'node:fs';import crypto from 'node:crypto';
const rows=JSON.parse(fs.readFileSync(0,'utf8'));
for(const row of rows)row[2].sort((a,b)=>JSON.stringify(a).localeCompare(JSON.stringify(b)));
rows.sort((a,b)=>JSON.stringify(a).localeCompare(JSON.stringify(b)));
console.log(JSON.stringify({sha256:crypto.createHash('sha256').update(JSON.stringify(rows)).digest('hex'),rows}));
"""
issues, file_hashes, audited, fingerprint_diagnostics = [], {}, {}, []
HERE = pathlib.Path(__file__).resolve().parent


def require(condition, message):
    if not condition:
        raise ValueError(message)


def attempt(label, action):
    try:
        return action()
    except (ValueError, TypeError, KeyError, IndexError, OSError, UnicodeError,
            OverflowError, RecursionError, subprocess.SubprocessError) as error:
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
        raise ValueError('non-finite JSON value: ' + value)
    return json.loads(raw.decode('utf8'), object_pairs_hook=pairs, parse_constant=reject)


def raw(path, limit=32 * 1024 * 1024):
    require(not path.is_symlink() and path.is_file(), 'missing or symbolic artifact file: ' + str(path))
    require(0 <= path.stat().st_size <= limit, 'artifact file exceeds analysis bound: ' + str(path))
    return path.read_bytes()


def read(path):
    return decode(raw(path))


def sha(value):
    return hashlib.sha256(value if isinstance(value, bytes) else value.encode('utf8')).hexdigest()


def file_sha(path):
    if path not in file_hashes:
        require(not path.is_symlink() and path.is_file(), 'symbolic/missing artifact file: ' + str(path))
        h = hashlib.sha256()
        with path.open('rb') as handle:
            for part in iter(lambda: handle.read(1024 * 1024), b''):
                h.update(part)
        file_hashes[path] = h.hexdigest()
    return file_hashes[path]


def digest(value):
    return isinstance(value, str) and re.fullmatch('[a-f0-9]{64}', value) is not None


def integer(value):
    return type(value) is int and 0 <= value <= 9007199254740991


def exact(value, keys, label):
    require(isinstance(value, dict) and set(value) == set(keys), label + ' fields differ')


def number(value):
    require(math.isfinite(value) and (value != 0 or math.copysign(1, value) > 0), 'non-canonical number')
    if type(value) is int:
        require(abs(value) <= 9007199254740991, 'unsafe canonical integer')
        return str(value)
    text, absolute = repr(value).lower(), abs(value)
    if absolute == 0:
        return '0'
    if 1e-6 <= absolute < 1e21:
        text = format(decimal.Decimal(text), 'f')
        return text.rstrip('0').rstrip('.') if '.' in text else text
    mantissa, exponent = text.split('e') if 'e' in text else (text, '0')
    mantissa = mantissa[:-2] if mantissa.endswith('.0') else mantissa
    exponent = int(exponent)
    return mantissa + 'e' + ('+' if exponent >= 0 else '') + str(exponent)


def quote(value):
    return ''.join('\\u%04x' % ord(c) if 0xd800 <= ord(c) <= 0xdfff else c
                   for c in json.dumps(value, ensure_ascii=False))


def encode(value, sort=True):
    if value is None:
        return 'null'
    if type(value) is bool:
        return 'true' if value else 'false'
    if isinstance(value, str):
        return quote(value)
    if type(value) in (int, float):
        return number(value)
    if isinstance(value, list):
        return '[' + ','.join(encode(v, sort) for v in value) + ']'
    require(isinstance(value, dict) and all(isinstance(k, str) for k in value), 'non-data canonical JSON')
    keys = sorted(value, key=lambda k: k.encode('utf-16-be', 'surrogatepass')) if sort else value
    return '{' + ','.join(quote(k) + ':' + encode(value[k], sort) for k in keys) + '}'


def canonical_sha(value):
    return sha(encode(value))


def inventory(root):
    require(root.is_dir() and not root.is_symlink(), 'artifact root is not a real directory')
    paths, total = [], 0
    for directory, dirs, files in os.walk(root, followlinks=False):
        relative = pathlib.Path(directory).relative_to(root)
        require(len(relative.parts) <= 10, 'artifact exceeds discovery depth')
        for name in dirs + files:
            require(not pathlib.Path(directory, name).is_symlink(), 'symlink in artifact')
        for name in files:
            path = pathlib.Path(directory, name)
            require(path.is_file(), 'nonregular artifact evidence')
            require(name not in ('managed-cache-auth.json', 'config.json'), 'private Action file leaked into artifact')
            paths.append(path)
            total += path.stat().st_size
            require(len(paths) <= 5000 and total <= 512 * 1024 * 1024, 'artifact exceeds inventory bound')
    return paths


def one(paths, predicate, label):
    matches = [p for p in paths if predicate(p)]
    require(len(matches) == 1, label + ': expected one file, found ' + str(len(matches)))
    return matches[0]


def artifact_root(item):
    root = pathlib.Path(item['root'])
    if not root.is_absolute():
        root = index_path.parent / root
    require(not root.is_symlink(), 'artifact root is a symlink')
    return root.resolve()


def jsonl(path):
    return [decode(line) for line in raw(path).splitlines() if line.strip()]


def timestamp(value):
    require(isinstance(value, str) and value.endswith('Z'), 'missing UTC GitHub timestamp')
    parsed = datetime.datetime.fromisoformat(value.replace('Z', '+00:00'))
    require(parsed.microsecond == 0, 'unexpected subsecond GitHub timestamp')
    return int(parsed.timestamp())


def test_count(i):
    return 4129 if i <= 6 else 4130 if i == 7 else 4131


def identity(row):
    return row['id'], row['variant'], row['i'], row['commit']


def boundary_complete(b, allow_denied=False):
    exact(b, ['inventory', 'guarded', 'unguardable', 'symbolInventory', 'symbolUnguardable', 'unsupported',
              'timerErrors', 'unhandledErrors', 'pendingTimers', 'pendingTimerDetails', 'completion',
              'virtualTime', 'logs', 'replayEligible', 'contract'], 'boundary')
    require(b['contract'] == 'quod-managed-v0-investigation' and b['replayEligible'] is False, 'boundary policy differs')
    for key in ('inventory', 'guarded', 'unguardable', 'unsupported', 'timerErrors', 'unhandledErrors', 'symbolUnguardable'):
        require(isinstance(b[key], list) and all(isinstance(v, str) for v in b[key]), 'invalid boundary ' + key)
    for key in ('unguardable', 'timerErrors', 'unhandledErrors', 'symbolUnguardable'):
        require(b[key] == [], 'boundary has ' + key)
    require(allow_denied or b['unsupported'] == [], 'unsupported managed response')
    require(type(b['pendingTimers']) is int and b['pendingTimers'] == 0 and b['pendingTimerDetails'] == [], 'pending managed timer')
    c = b['completion']
    exact(c, ['sealed', 'nativeTurnFences', 'timerCallbacks', 'pendingTimerCallbacks', 'pendingMicrotasks',
              'pendingEventCallbacks', 'intrinsicLockdown', 'protectedIntrinsicObjects', 'reason'], 'completion')
    require(c['sealed'] is True and c['intrinsicLockdown'] is True and c['reason'] == '', 'unsealed completion')
    require(integer(c['protectedIntrinsicObjects']) and c['protectedIntrinsicObjects'] > 0 and
            integer(c['nativeTurnFences']) and 2 <= c['nativeTurnFences'] <= 512 and integer(c['timerCallbacks']), 'invalid completion evidence')
    for key in ('pendingTimerCallbacks', 'pendingMicrotasks', 'pendingEventCallbacks'):
        require(type(c[key]) is int and c[key] == 0, 'pending async callback: ' + key)
    require(type(b['virtualTime']) in (int, float) and math.isfinite(b['virtualTime']) and b['virtualTime'] >= 0, 'invalid virtual clock')
    require(isinstance(b['symbolInventory'], list), 'symbol inventory missing')
    for row in b['symbolInventory']:
        exact(row, ['owner', 'ownerIndex', 'name', 'descriptor', 'valueType', 'primitiveValue',
                    'enumerable', 'configurable', 'writable', 'policy'], 'symbol descriptor')
        require(isinstance(row['owner'], str) and integer(row['ownerIndex']) and isinstance(row['name'], str) and
                isinstance(row['primitiveValue'], str) and all(type(row[k]) is bool for k in ('enumerable', 'configurable', 'writable')), 'nonprimitive symbol evidence')
        require(row['policy'] in ('guarded', 'fixed primitive data'), 'unguarded native symbol')
        if row['policy'] == 'fixed primitive data':
            require(row['descriptor'] == 'data' and row['valueType'] in ('undefined', 'null', 'boolean', 'string', 'number', 'bigint', 'symbol'), 'native symbol reference escaped')
        else:
            require(row['descriptor'] in ('data', 'accessor') and row['primitiveValue'] == '', 'invalid guarded symbol')
    require(isinstance(b['logs'], list), 'console effects missing')
    for row in b['logs']:
        require(isinstance(row, list) and len(row) == 2 and row[0] in (
            'log', 'info', 'warn', 'error', 'debug', 'trace', 'dir', 'assert', 'time', 'timeEnd', 'timeLog') and
            isinstance(row[1], list) and all(isinstance(v, str) for v in row[1]), 'nonprimitive console effects')


def passing(data):
    exact(data, ['ok', 'rows', 'files', 'boundary'], 'candidate response')
    require(data['ok'] is True and len(data['rows']) == 47, 'candidate did not complete all 47 tests')
    for row in data['rows']:
        exact(row, ['name', 'state', 'errors'], 'candidate test')
        require(isinstance(row['name'], str) and row['name'] and row['state'] == 'pass' and row['errors'] == [], 'candidate test did not pass')
    require(len({row['name'] for row in data['rows']}) == 47, 'duplicate candidate names')
    require(data['files'] == [{'name': EXPECTED['file'], 'state': 'pass', 'errors': []}], 'candidate file differs')
    boundary_complete(data['boundary'])


def build(path, expected_entry):
    data = read(path)
    ident = data['identity']
    require(ident['schema'] == 'quod-managed-compile-v1-research' and ident['entry'] == expected_entry, 'compiler schema/entry differs')
    require(ident['compilerSha256'] == file_sha(HERE / 'compiler.mjs'), 'unreviewed compiler source')
    require(digest(data['sha256']) and ident['bundleSha256'] == data['sha256'] and
            data['identitySha256'] == canonical_sha(ident), 'compiled identity/bundle digest differs')
    require(ident['globs'] == data['globs'] and ident['adapters'] == data['adapters'] and
            ident['moduleNameSetsSha256'] == canonical_sha(data['globs']), 'module-name collection identity differs')
    require(ident['sources'] and all(isinstance(row['source'], str) and digest(row['sourceSha256']) and
            digest(row['compiledSourceSha256']) for row in ident['sources']), 'source closure identities missing')
    require(len({row['source'] for row in ident['sources']}) == len(ident['sources']), 'duplicate compiler source')
    closure = data['closure']
    require(closure['finalBundleParsed'] is True and closure['remainingDynamicImports'] == 0 and
            closure['remainingDynamicRequires'] == 0 and all(v == 'node:async_hooks' for v in closure['externalImports']), 'compiler module closure open')
    imports = [row for output in data['metadata']['outputs'].values() for row in output['imports']]
    require(all(row['path'] == 'node:async_hooks' and row['external'] is True for row in imports), 'unreviewed native import')
    return data


def invocation(out, phase, built, runtime_policy, host):
    name = phase['name']
    require(name in ('preflight', 'managed'), 'unexpected worker phase')
    directory = out / name
    data_bytes = raw(directory / 'response.json', 16 * 1024 * 1024)
    data = decode(data_bytes)
    require(encode(data, False).encode() == data_bytes, 'worker response is not exact compact own-data JSON')
    require(phase['responseSha256'] == sha(data_bytes), 'phase response SHA differs')
    i, execution, meta = read(directory / 'invocation.json'), read(directory / 'execution.json'), read(directory / 'response-meta.json')
    require(i['schema'] == 'quod-managed-invocation-v1-research' and i['name'] == name, 'invocation identity differs')
    require(i['bundleSha256'] == built['sha256'] == file_sha(directory / 'worker.mjs') and
            i['configSha256'] == file_sha(directory / 'config.capnp'), 'actual worker/config byte identity differs')
    actual_policy = dict(runtime_policy)
    actual_policy.pop('nativeFingerprintSha256')
    require(i['runtimePolicy'] == actual_policy and i['host'] == host, 'actual policy/host differs from keyed identity')
    require(i['env'] == runtime_policy['processEnv'] and set(i['env']) == {'PATH', 'TZ', 'LANG'} and
            i['env']['TZ'] == 'UTC' and i['env']['LANG'] == 'C.UTF-8' and i['env']['PATH'], 'worker ambient credentials/environment leaked')
    require(runtime_policy['workerdSha256'] == WORKERD_SHA and
            runtime_policy['denyModuleSha256'] == file_sha(directory / 'deny.mjs'), 'unreviewed binary/deny module identity')
    require(i['args'] == ['serve', i['args'][1]] and i['args'][1].endswith('/' + name + '/config.capnp'), 'runtime arguments differ')
    u = urllib.parse.urlsplit(i['request']['url'])
    require(i['request']['method'] == 'GET' and u.scheme == 'http' and u.hostname == '127.0.0.1' and
            u.port and u.path == '/run' and not u.query and not u.fragment, 'worker request policy differs')
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
'''.replace('PORT', str(u.port))
    require(config == expected_config, 'actual runtime config grants different capabilities')
    require(execution['name'] == name and execution['completed'] is True and execution['timedOut'] is False and
            execution['error'] is None and execution['exit']['spawnError'] is None and execution['responseSha256'] == sha(data_bytes), 'incomplete worker execution')
    require(meta['sha256'] == sha(data_bytes) and meta['bytes'] == len(data_bytes) and 200 <= meta['status'] <= 299, 'worker HTTP bytes/status differ')
    return data_bytes, data


def receipt(path, execution):
    encoded = raw(path, 24 * 1024 * 1024)
    data = decode(encoded)
    require(encode(data).encode() == encoded, 'receipt is not exact canonical JSON')
    exact(data, ['schema', 'researchOnly', 'execution', 'expected', 'response', 'resultDigest'], 'receipt')
    require(data['schema'] == RECEIPT_SCHEMA and data['researchOnly'] is True and data['execution'] == execution and
            data['expected'] == EXPECTED, 'receipt execution/workload identity differs')
    exact(data['response'], ['encoding', 'bytes', 'sha256'], 'receipt response')
    require(data['response']['encoding'] == 'base64', 'response encoding differs')
    response = base64.b64decode(data['response']['bytes'], validate=True)
    require(base64.b64encode(response).decode() == data['response']['bytes'] and sha(response) == data['response']['sha256'], 'receipt response byte SHA differs')
    require(data['resultDigest'] == sha(RECEIPT_SCHEMA + '\n' + execution['key'] + '\n' + data['response']['bytes']), 'receipt result domain digest differs')
    decoded = decode(response)
    require(encode(decoded, False).encode() == response, 'stored response is not exact compact JSON')
    passing(decoded)
    return {'sha256': sha(encoded), 'responseSha256': sha(response), 'bytes': len(encoded), 'rawResponse': response, 'data': decoded}


def fingerprint(report, project):
    rows = []
    require(report['success'] is True and isinstance(report['testResults'], list), 'merged full suite did not pass')
    for file in report['testResults']:
        require(isinstance(file['name'], str) and file['name'].startswith(project.rstrip('/') + '/') and
                file['status'] == 'passed' and isinstance(file['assertionResults'], list), 'full-suite file failed or outside project')
        tests = []
        for test in file['assertionResults']:
            require(isinstance(test['fullName'], str) and test['status'] in ('passed', 'pending', 'skipped', 'todo'), 'full-suite assertion did not pass')
            require(not test.get('failureMessages'), 'full-suite assertion carries errors')
            tests.append([test['fullName'], test['status']])
        rows.append([file['name'][len(project.rstrip('/')) + 1:], file['status'], tests])
    node = shutil.which('node')
    require(node is not None, 'trusted Node is needed for localeCompare fingerprint')
    env = {k: os.environ[k] for k in ('PATH', 'SYSTEMROOT') if k in os.environ}
    env.update({'TZ': 'UTC', 'LANG': 'C.UTF-8'})
    done = subprocess.run([node, '--input-type=module', '-e', FINGERPRINT_JS], input=encode(rows, False).encode(),
                          capture_output=True, timeout=30, env=env, check=True)
    computed = decode(done.stdout)
    return computed, rows


def audit_artifact(item, row, settings):
    root = artifact_root(item)
    paths = inventory(root)
    result_path = one(paths, lambda p: p.name == 'result.json' and read(p).get('schema') == 'quod-managed-dispatch-v1', 'dispatcher result')
    out, result = result_path.parent, read(result_path)
    event = row['managed']['events'][0]
    mode, hit = ('control' if row['variant'] == 'managed-control' else 'v0'), event['hit']
    require(result['mode'] == mode and result['optIn'] is True and result['hit'] is hit and
            result['fallback'] is False and result['exitCode'] == 0 and not result.get('managedError'), 'dispatcher did not preserve supported passing result')
    require(event['fallback'] is False and result['managedExecutions'] == (0 if hit else 1) == event['managedExecutions'], 'candidate execution accounting differs')
    require(result['candidate'] == CANDIDATE and len(result['files']) == 1 and result['files'][0]['file'] == CANDIDATE and
            result['files'][0]['decision'] == ('replay' if hit else 'managed') and
            result['files'][0]['executionKey'] == result['executionKey'], 'candidate decision differs')
    for key in ('effectsSha256', 'runtimeClassSha256', 'nativeFingerprintSha256', 'executionKey', 'commandSeconds'):
        require(result[key] == event[key], 'log and dispatcher disagree: ' + key)
    require(event.get('files') == [{'file': CANDIDATE, 'decision': 'replay' if hit else 'managed'}], 'compact bridge file decision differs')
    execution = read(out / 'execution-identity.json')
    exact(execution, ['identity', 'key'], 'execution')
    ident = execution['identity']
    exact(ident, ['schema', 'compile', 'preparation', 'runtimePolicy', 'supervisorPolicy', 'host'], 'execution identity')
    require(ident['schema'] == EXECUTION_SCHEMA and execution['key'] == sha(EXECUTION_SCHEMA + '\n' + encode(ident)) == result['executionKey'], 'execution domain digest differs')
    preflight, built = build(out / 'preflight-build.json', '/quod-adapter/preflight.mjs'), build(out / 'test-build.json', '/quod-adapter/runner.mjs')
    require(built['identity'] == ident['compile'] and built['preparation'] == ident['preparation'], 'keyed compiler/preparation differs')
    require(preflight['identity']['configurationSha256'] == canonical_sha(BASE) and
            built['identity']['configurationSha256'] == canonical_sha(built['options']), 'compiler options digest differs')
    require({k: built['options'][k] for k in BASE} == BASE and built['options']['file'] == '/project/' + CANDIDATE, 'managed contract/file differs')
    require(any(s['source'] == '/project/' + CANDIDATE and s['sourceSha256'] == SOURCE_SHA for s in built['identity']['sources']), 'unchanged candidate source pin absent')
    prepared, policy, host, supervisor = ident['preparation'], ident['runtimePolicy'], ident['host'], ident['supervisorPolicy']
    require(prepared['schema'] == 'quod-managed-preparation-v1-research' and prepared['preparerSha256'] == file_sha(HERE / 'preparation.mjs') and
            {s['name']: s['sha256'] for s in prepared['sources']} == PREPARATION_SOURCES and
            prepared['configurationSha256'] == canonical_sha(built['options']['vitest']), 'preparation policy/config differs')
    prep = read(out / 'preparation.json')
    require(prep['profile'] == 'convex-helpers-original-twelve' and len(prep['selected']) == 120 and
            len({(s['file'], s['project']) for s in prep['selected']}) == 120 and
            {s['name']: s['sha256'] for s in prep['reviewedSources']} == PREPARATION_SOURCES, 'actual complete project selection differs')
    require(sorted(s['file'] for s in prep['selected']).count(CANDIDATE) == 1, 'candidate not selected exactly once')
    require(policy['schema'] == 'quod-managed-runtime-policy-v1-research' and policy['moduleName'] == 'worker.mjs' and
            policy['services'] == ['main', 'deny'] and policy['globalOutbound'] == 'deny' and policy['bindings'] == [] and
            policy['compatibilityDate'] == '2026-07-01' and policy['compatibilityFlags'] == ['nodejs_als'] and
            policy['request'] == {'method': 'GET', 'path': '/run'}, 'keyed runtime capabilities differ')
    s = policy['supervisor']
    require(s['sourceSha256'] == file_sha(HERE / 'executor.mjs') and digest(s['nodeExecutableSha256']) and
            s['completionBounds'] == BOUNDS and s['watchdogMs'] == 300000 and s['responseBytesMax'] == 16 * 1024 * 1024 and
            all(integer(s[k]) and s[k] > 0 for k in ('startupAttempts', 'healthTimeoutMs', 'startupRetryMs', 'shutdownGraceMs')), 'supervisor identity/bounds differ')
    require(supervisor == {'receiptValidatorSha256': file_sha(HERE / 'receipt.mjs'),
                           'experimentSha256': file_sha(HERE / 'dispatcher.mjs'),
                           'ordinarySelectionPolicy': 'vitest-5-native-specifications-v1',
                           'ordinaryAdapterSha256': file_sha(HERE / 'ordinary-remainder.mjs'),
                           'expected': EXPECTED}, 'unreviewed receipt/dispatcher/ordinary selector source')
    exact(host, ['ImageOS', 'ImageVersion', 'platform', 'arch', 'kernel', 'libc', 'nodeVersion'], 'host class')
    require(host['platform'] == 'linux' and host['arch'] == 'x64' and host['nodeVersion'] == s['nodeVersion'] and
            host['ImageOS'] and host['ImageVersion'] and host['libc']['family'] == 'glibc', 'host class incomplete')
    require(result['runtimeClassSha256'] == canonical_sha({'runtimePolicy': policy, 'host': host}), 'software class digest differs')
    phases = result['phases']
    require([p['name'] for p in phases] == (['preflight'] if hit else ['preflight', 'managed']), 'unexpected executed phases')
    require({p.parent.name for p in paths if p.name == 'worker.mjs'} == set(p['name'] for p in phases), 'unrecorded native worker execution')
    if hit:
        require(not (out / 'managed').exists() and not (out / 'managed-miss').exists(), 'replay executed managed candidate')
    _, capability = invocation(out, phases[0], preflight, policy, host)
    require(capability['ok'] is True and len(capability['checks']) == 31 and all(c['pass'] is True for c in capability['checks']) and
            len({c['name'] for c in capability['checks']}) == 31, 'native capability preflight failed')
    boundary_complete(capability['boundary'], allow_denied=True)
    native = canonical_sha({'preflightIdentity': preflight['identitySha256'], 'inventory': capability['boundary']['inventory'],
                            'symbols': capability['boundary']['symbolInventory']})
    require(native == policy['nativeFingerprintSha256'] == result['nativeFingerprintSha256'], 'native fingerprint differs')
    selected_bytes = raw(out / 'selected-response.json', 16 * 1024 * 1024)
    data = decode(selected_bytes)
    passing(data)
    effect = {'ok': data['ok'], 'rows': data['rows'], 'files': data['files'], 'logs': data['boundary']['logs'], 'virtualTime': data['boundary']['virtualTime']}
    require(result['effects'] == effect and result['effectsSha256'] == canonical_sha(effect), 'candidate effect digest differs')
    if not hit:
        actual_bytes, actual = invocation(out, phases[1], built, policy, host)
        require(actual == data, 'selected result differs from actual worker')
        recorded = receipt(out / 'receipt.json', execution)
        require(recorded['rawResponse'] == actual_bytes and len(result['receipts']) == 1, 'new receipt lacks actual worker byte witness')
        metadata = result['receipts'][0]
        require(metadata['key'] == execution['key'] and metadata['receiptSha256'] == recorded['sha256'] and
                metadata['responseSha256'] == recorded['responseSha256'] and metadata['bytes'] == recorded['bytes'] and
                metadata['expected'] == EXPECTED and metadata['file'].endswith('/' + out.name + '/receipt.json') and
                result['files'][0]['receipt'] == metadata, 'new receipt byte/identity metadata differs')
    else:
        require(result['receipts'] == [] and not (out / 'receipt.json').exists(), 'replay emitted a new receipt')
        recorded = None
    report_path = one(paths, lambda p: p.name == pathlib.PurePosixPath(result['reportPath']).name, 'merged full report')
    report = read(report_path)
    computed, full_rows = fingerprint(report, result['project'])
    observed_fingerprint = {'run': row['run'], 'variant': row['variant'], 'i': row['i'],
                            'expectedSha256': STATE_SHAS[row['i']], 'actualSha256': computed['sha256'],
                            'files': len(full_rows), 'tests': sum(len(r[2]) for r in full_rows)}
    if computed['sha256'] != STATE_SHAS[row['i']]:
        # Preserve actual tuples for a Linux/Node-24 ICU comparison if the
        # auditor is run on a host with a different Unicode collation version.
        observed_fingerprint['actualSortedTuples'] = computed['rows']
    fingerprint_diagnostics.append(observed_fingerprint)
    require(computed['sha256'] == STATE_SHAS[row['i']] == row['semantic'] and len(full_rows) == 120 and
            sum(len(r[2]) for r in full_rows) == test_count(row['i']), 'raw full-suite fingerprint differs from original window')
    candidate_rows = [r for r in full_rows if r[0] == CANDIDATE]
    require(len(candidate_rows) == 1 and sorted(candidate_rows[0][2]) == sorted([r['name'], 'passed'] for r in data['rows']), '47 candidate states differ in merged report')
    require(report['numTotalTests'] == test_count(row['i']) and report['numTotalTestSuites'] == 120 and
            report['numFailedTests'] == 0 and report['numFailedTestSuites'] == 0, 'merged report counters differ')
    require(result['ordinaryRepeatedCandidate'] is False and len(result['ordinaryRuns']) == 1 and
            result['ordinaryRuns'][0]['exitCode'] == 0 and result['ordinary']['exitCode'] == 0, 'ordinary remainder was absent, repeated or failed')
    ordinary = read(out / 'ordinary-report.json')
    require(len(ordinary['testResults']) == 119 and all(not f['name'].endswith('/' + CANDIDATE) for f in ordinary['testResults']), 'ordinary remainder selection differs')
    request = read(out / 'ordinary-remainder-request.json')
    selection = read(out / 'ordinary-remainder-selection.json')
    instance_counts = lambda records: collections.Counter((r['absolute'], r['project']) for r in records)
    require(request['schema'] == 'quod-ordinary-remainder-request-v1' and request['project'] == result['project'] and
            request['selected'] == prep['selected'] and request['filters'] == [] and
            len(request['reporters']) == 2 and set(request['reporters']) == {'default', 'json'}, 'native ordinary request differs from full original preparation')
    selected_instances = instance_counts(prep['selected'])
    excluded = next(s for s in prep['selected'] if s['file'] == CANDIDATE)
    excluded_identity = {'absolute': excluded['absolute'], 'project': excluded['project']}
    excluded_instances = instance_counts([excluded_identity])
    remainder_instances = selected_instances - excluded_instances
    require(request['excluded'] == excluded_identity and excluded['project'] == 'convex-helpers' and
            request['outputFile'].endswith('/' + out.name + '/ordinary-report.json') and
            request['evidenceFile'].endswith('/' + out.name + '/ordinary-remainder-selection.json') and
            result['ordinarySelectionPath'] == request['evidenceFile'], 'native ordinary exclusion/output differs')
    require(selection['schema'] == 'quod-ordinary-remainder-selection-v1' and
            selection['matched'] is True and selection['runStarted'] is True and selection['completed'] is True and
            selection['exitCode'] == 0 and selection['unhandledErrors'] == 0 and selection['failedModules'] == 0 and
            'error' not in selection and 'closeError' not in selection, 'native ordinary selector/teardown failed')
    require(instance_counts(selection['selected']) == selected_instances and
            instance_counts(selection['excluded']) == excluded_instances and
            instance_counts(selection['remainder']) == remainder_instances == instance_counts(selection['executed']) and
            sum(remainder_instances.values()) == 119, 'before/after/executed native project-file multisets differ')
    require(result['ordinarySelection'] == selection and
            collections.Counter(f['name'] for f in ordinary['testResults']) ==
            collections.Counter(r['absolute'] for r in selection['executed']), 'ordinary report differs from native executed specifications')
    ordinary_invocation = read(out / 'ordinary-remainder-invocation.json')
    require(ordinary_invocation['entry'].endswith('/ordinary-remainder.mjs') and
            ordinary_invocation['args'] == [request['evidenceFile'].replace('ordinary-remainder-selection.json', 'ordinary-remainder-request.json')] and
            ordinary_invocation['cwd'] == result['project'], 'actual native selector invocation differs')
    # Vitest reports repeated absolute paths for distinct configured projects.
    # Compare complete JSON rows as multisets; a path-keyed map loses instances.
    merged_ordinary = [f for f in report['testResults'] if not f['name'].endswith('/' + CANDIDATE)]
    require(collections.Counter(encode(f) for f in merged_ordinary) ==
            collections.Counter(encode(f) for f in ordinary['testResults']), 'merged report altered ordinary project/file instances')
    expected_normalized = sorted((f['name'], t['fullName'], {'passed': 'pass', 'failed': 'fail',
        'skipped': 'skip', 'pending': 'skip', 'todo': 'todo'}[t['status']])
        for f in report['testResults'] for t in f['assertionResults'])
    require(sorted((r['file'], r['name'], r['state']) for r in result['rows']) == expected_normalized,
            'dispatcher normalized rows differ from complete merged report')
    trace = read(out / 'cache-transport.json')
    require(trace['schema'] == 'quod-managed-action-cache-v1' and isinstance(trace['events'], list), 'cache transport trace missing')
    events = trace['events']
    prefix = 'quod-managed-v1-' + row['cacheNamespace'] + '-' + execution['key'] + '-'
    authority = None
    if mode == 'control':
        require(events == [] and not hit, 'control performed cache transport or replay')
    else:
        require(len(events) == (1 if hit else 2) and events[0]['operation'] in ('restore', 'restore-error'), 'cache transport operation order differs')
        restore = events[0]
        require(all(e['executionKey'] == execution['key'] for e in events), 'transport execution key differs')
        if restore['operation'] == 'restore':
            require(restore['prefix'] == prefix, 'cache namespace/execution prefix differs')
            matched = restore['matchedServiceKey']
            suffix = matched[len(prefix):] if isinstance(matched, str) and matched.startswith(prefix) else None
            trusted = suffix if digest(suffix) else None
            require(restore['trustedReceiptSha256'] == trusted, 'cache authority was not derived from exact returned service key')
            require(restore['file'].endswith('/managed-receipts/' + execution['key'] + '/receipt.json'), 'restored receipt path differs')
            if hit:
                require(trusted is not None, 'hit lacks authenticated service-key digest')
                cached = one(paths, lambda p: p.name == 'receipt.json' and p.parent.name == execution['key'] and
                             p.parent.parent.name == 'managed-receipts', 'authenticated cached receipt')
                authority = receipt(cached, execution)
                require(authority['sha256'] == trusted and authority['rawResponse'] == selected_bytes, 'trusted receipt bytes differ from replay result')
                authority['serviceKey'] = matched
        else:
            require(not hit, 'restore error became replay')
        if not hit:
            save = events[1]
            require(save['operation'] in ('save', 'save-error'), 'miss did not attempt receipt transport')
            if save['operation'] == 'save':
                require(save['receiptSha256'] == recorded['sha256'] and save['key'] == prefix + recorded['sha256'] and
                        integer(save['cacheId']) and save['cacheId'] > 0, 'save authority/receipt/cache identity differs')
    receipt_paths = [p for p in paths if p.name == 'receipt.json']
    require(len(receipt_paths) == (1 if mode == 'control' or hit else 2), 'unexpected reusable receipt inventory')
    for path in receipt_paths:
        observed_receipt = receipt(path, execution)
        require(observed_receipt['data'] == data, 'retained receipt disagrees with selected candidate')
    for path in paths:
        file_sha(path)
    return {'run': row['run'], 'variant': row['variant'], 'i': row['i'], 'commit': row['commit'], 'root': root,
            'result': result, 'data': data, 'execution': execution, 'trace': events, 'authority': authority,
            'recorded': recorded, 'fingerprint': computed['sha256'], 'paths': paths, 'artifact': item}


def coordinator(root):
    paths = inventory(root)
    find = lambda name: one(paths, lambda p: p.name == name, name)
    settings, rows, expected, manifest, metrics = read(find('settings.json')), jsonl(find('jobs.jsonl')), jsonl(find('expected.jsonl')), read(find('managed-window.json')), read(find('job-metrics.json'))
    require(settings['managedWindow'] is True and settings['cold'] is True and settings['repeatFirst'] is False and
            settings['experimentalCloud'] is False and settings['harnessSha'] == HARNESS_SHA and
            settings['actionSha'] == ACTION_SHA and re.fullmatch('[0-9]+', settings['coordinatorRun']),
            'cold pinned coordinator provenance differs')
    require(manifest['schema'] == 'quod-managed-original-window-v1' and manifest['repository'] == 'get-convex/convex-helpers' and
            manifest['cold'] is True and manifest['repeatFirst'] is False and manifest['count'] == 12 and
            manifest['variants'] == list(VARIANTS), 'original window manifest differs')
    require(manifest['commands'] == {'install': 'npm ci', 'build': 'npm run build', 'test': 'npm run test'} and
            manifest['managedContract'] == BASE and manifest['candidate'] == {'file': CANDIDATE, 'sourceSha256': SOURCE_SHA, 'tests': 47}, 'original commands/managed contract differs')
    require(manifest['commits'] == [{'i': i, 'sha': s, 'files': 120, 'tests': test_count(i),
            'ordinarySemanticSha256': STATE_SHAS[i], 'releaseTests': i >= 2} for i, s in enumerate(SHAS)], 'original twelve commit expectations differ')
    wanted = {('convex-helpers', v, i, s) for v in VARIANTS for i, s in enumerate(SHAS)}
    require(len(rows) == len(expected) == 48 and {identity(r) for r in rows} == wanted and
            {identity(r) for r in expected} == wanted, '48-job original four-variant inventory differs')
    require(len({str(r['run']) for r in rows}) == 48 and len({str(r['job']['id']) for r in rows}) == 48, 'reused GitHub run/job identity')
    per = {}
    for row in rows:
        j, i, v = row['job'], row['i'], row['variant']
        require(type(i) is int and 0 <= i < 12 and integer(row['run']) and row['run'] > 0 and
                str(row['run_meta']['id']) == str(row['run']) and row['run_meta']['event'] == 'workflow_dispatch' and
                row['run_meta']['path'] == '.github/workflows/bench-job-one.yml' and row['run_meta']['run_attempt'] == 1,
                'run identity, measured workflow or cold first attempt differs')
        if v in ('managed', 'managed-control'):
            require(row['run_meta']['head_branch'] == 'bench/wj-' + settings['coordinatorRun'] + '-convex-helpers-' + v,
                    'managed cold-chain branch scope differs')
        require(j['conclusion'] == row['run_meta']['conclusion'] == 'success' and
                row['run_meta']['head_sha'] == settings['harnessSha'] and row['image'], 'unsuccessful/wrong-source/image measured job')
        created, started, completed = [timestamp(j[k]) for k in ('created_at', 'started_at', 'completed_at')]
        require(created <= started < completed, 'invalid GitHub job duration')
        groups = dict.fromkeys(('boot', 'setup', 'action', 'install', 'build', 'test', 'post'), 0)
        require(isinstance(j['steps'], list) and j['steps'], 'job step evidence missing')
        for step in j['steps']:
            if step['conclusion'] == 'skipped':
                continue
            a, b = timestamp(step['started_at']), timestamp(step['completed_at'])
            require(started <= a <= b <= completed and step['conclusion'] == 'success', 'failed or out-of-job step')
            name = step['name']
            group = 'boot' if name == 'Set up job' else 'post' if name == 'Complete job' or name.startswith('Post ') else 'action' if name == 'quod' else name if name in ('install', 'build', 'test') else 'setup'
            groups[group] += b - a
        work_steps = [s['name'] for s in j['steps'] if s['name'] in ('install', 'build', 'test') and s['conclusion'] != 'skipped']
        require(work_steps == ([] if v == 'floor' else ['install', 'build', 'test']), 'original install/build/test workload differs')
        runner = completed - started
        per[(v, i)] = {'id': row['id'], 'variant': v, 'i': i, 'commit': row['commit'], 'run': row['run'],
                       'runner': runner, 'wall': completed - created, 'queue': started - created, 'billed': math.ceil(runner / 60), **groups}
        p = per[(v, i)]
        p.update({'work': p['install'] + p['build'] + p['test'], 'boot_setup': p['boot'] + p['setup']})
        p['fixed'] = runner - p['work']
        if v != 'floor':
            m = row['managed']
            require(m['invalid'] is False and row['semantic'] == STATE_SHAS[i] and
                    m['states'] == {'sha256': STATE_SHAS[i], 'files': 120, 'tests': test_count(i)}, 'original full-suite log states differ')
            release = m['releaseSignature']
            require(isinstance(release, list) and release == sorted(release) and (len(release) >= 6 if i >= 2 else release == []), 'release-test tail missing/unexpected')
            for line in release:
                require(re.fullmatch('(tests|pass|fail|cancelled|skipped|todo) [0-9]+', line), 'invalid release counter')
            if i >= 2:
                require(all(any(line.startswith(k + ' ') for line in release) for k in ('tests', 'pass', 'fail', 'cancelled', 'skipped', 'todo')) and
                        all(int(line.split()[1]) == 0 for line in release if line.split()[0] in ('fail', 'cancelled')), 'release failures/missing counters')
        if v in ('managed', 'managed-control'):
            require(row['cacheNamespace'] == 'managed-' + settings['coordinatorRun'] + '-convex-helpers-' + v, 'cold chain namespace differs')
            events = row['managed']['events']
            require(len(events) == 1, 'missing/duplicate managed bridge result')
            e = events[0]
            require(e['mode'] == ('control' if v == 'managed-control' else 'v0') and type(e['hit']) is bool and
                    e['fallback'] is False and e['exitCode'] == 0 and type(e['managedExecutions']) is int and
                    e['managedExecutions'] == (0 if e['hit'] else 1), 'incomplete managed decision evidence')
            require(not e['hit'] or (v == 'managed' and i > 0), 'control or cold c0 hit')
            require(all(digest(e[k]) for k in ('effectsSha256', 'runtimeClassSha256', 'nativeFingerprintSha256', 'executionKey')), 'closed execution/effect identity missing')
        else:
            require(row.get('managed', {}).get('events', []) == [], 'baseline ran managed dispatcher')
    for row in rows:
        if row['variant'] in ('managed', 'managed-control'):
            plain = next(r for r in rows if r['variant'] == 'plain' and r['i'] == row['i'])
            require(row['managed']['releaseSignature'] == plain['managed']['releaseSignature'], 'release-test tail differs from plain')
    metric_per = {(r['variant'], r['i']): r for r in metrics['per']}
    require(len(metrics['per']) == 48 and set(metric_per) == set(per) and metrics['managedVerificationErrors'] == [], 'report metric inventory/errors differ')
    for key, p in per.items():
        r = metric_per[key]
        require(all(r[k] == value for k, value in p.items()), 'reported job times differ from independent API durations')
        if key[0] in ('managed', 'managed-control'):
            row = next(x for x in rows if x['variant'] == key[0] and x['i'] == key[1])
            require(r['hit'] == ('partial-hit' if row['managed']['events'][0]['hit'] else 'record') and r['wrong'] == [], 'candidate hit was mislabeled or unchecked')
    for v in ('managed', 'managed-control'):
        chain = sorted([r for r in rows if r['variant'] == v], key=lambda r: r['i'])
        require(all(timestamp(a['job']['completed_at']) <= timestamp(b['job']['started_at']) for a, b in zip(chain, chain[1:])), 'managed cache chain overlapped or reordered')
    for path in paths:
        file_sha(path)
    return settings, rows, metrics, per, paths


def final_checks(rows, metrics, per):
    require(len(audited) == 24, 'not all 24 managed artifacts independently verified')
    source_keys = {}
    for i in range(12):
        control, candidate = audited[('managed-control', i)], audited[('managed', i)]
        for key in ('effectsSha256', 'runtimeClassSha256', 'nativeFingerprintSha256', 'executionKey'):
            require(control['result'][key] == candidate['result'][key], 'candidate/control disagree: c%d %s' % (i, key))
        require(control['data'] == candidate['data'], 'candidate/control complete effects/boundary differ')
        for item in candidate['trace']:
            if item['operation'] == 'restore' and candidate['result']['hit']:
                require(item['matchedServiceKey'] in source_keys and source_keys[item['matchedServiceKey']]['i'] < i,
                        'replay lacks earlier successful save in this cold chain')
                origin = source_keys[item['matchedServiceKey']]
                require(origin['recorded']['rawResponse'] == candidate['authority']['rawResponse'] and
                        origin['recorded']['sha256'] == candidate['authority']['sha256'], 'replay differs from authenticated cold-chain worker witness')
            if item['operation'] == 'save':
                source_keys[item['key']] = candidate
    totals = {}
    for v in VARIANTS:
        entries = [r for key, r in per.items() if key[0] == v]
        after = [r for r in entries if r['i'] > 0]
        events = [r['managed']['events'][0] for r in rows if r['variant'] == v and v in ('managed', 'managed-control')]
        totals[v] = {'jobs': 12, 'runnerSeconds': sum(r['runner'] for r in entries), 'modeledMinutes': sum(r['billed'] for r in entries),
                     'afterFirst': {'runnerSeconds': sum(r['runner'] for r in after), 'modeledMinutes': sum(r['billed'] for r in after)},
                     'candidateReplays': sum(e['hit'] for e in events), 'managedExecutions': sum(e['managedExecutions'] for e in events),
                     'fallbacks': sum(e['fallback'] for e in events), 'fullJobHits': 0}
    recorded = metrics['managedExperiment']
    require(recorded['schema'] == 'quod-managed-original-window-report-v1' and recorded['cold'] is True and
            recorded['repeatFirst'] is False and recorded['originalPrior'] == {'runnerSeconds': 1735, 'modeledMinutes': 36} and
            recorded['variants'] == totals, 'reported full-window totals differ from independent accounting')
    p, c, m = totals['plain'], totals['managed-control'], totals['managed']
    comparison = {'runnerRatioToFreshPlain': m['runnerSeconds'] / p['runnerSeconds'], 'runnerRatioToManagedControl': m['runnerSeconds'] / c['runnerSeconds'],
                  'runnerRatioToPriorPlain': m['runnerSeconds'] / 1735, 'modeledMinutesRatioToFreshPlain': m['modeledMinutes'] / p['modeledMinutes'],
                  'modeledMinutesRatioToManagedControl': m['modeledMinutes'] / c['modeledMinutes'], 'modeledMinutesRatioToPriorPlain': m['modeledMinutes'] / 36,
                  'runnerBarMet': m['runnerSeconds'] <= p['runnerSeconds'] / 2 and m['runnerSeconds'] <= 1735 / 2,
                  'modeledMinuteBarMet': m['modeledMinutes'] <= p['modeledMinutes'] / 2 and m['modeledMinutes'] <= 18}
    require(recorded['comparison'] == comparison, 'reported timing/cost comparison differs')
    return {'variants': totals, 'comparison': comparison, 'publicRunnerChargeUSD': 0,
            'partialCandidateHits': totals['managed']['candidateReplays'], 'fullJobHits': 0}


def main():
    global index_path
    require(len(sys.argv) == 3, 'usage: window-audit.py COORDINATOR_ROOT MANAGED_INDEX.json')
    coordinator_root, index_path = pathlib.Path(sys.argv[1]).absolute(), pathlib.Path(sys.argv[2]).absolute()
    collected = attempt('coordinator original window and API accounting', lambda: coordinator(coordinator_root))
    index = attempt('verified managed artifact index', lambda: read(index_path))
    if collected and index:
        settings, rows, metrics, per, coordinator_paths = collected
        def index_check():
            require(index['schema'] == 'quod-managed-window-artifacts-v1' and len(index['artifacts']) == 24, 'managed artifact index inventory differs')
            wanted = {(v, i, str(r['run'])) for v in ('managed', 'managed-control') for i in range(12)
                      for r in rows if r['variant'] == v and r['i'] == i}
            require({(a['variant'], a['i'], str(a['run'])) for a in index['artifacts']} == wanted, 'artifact index does not match measured jobs')
            require(len({str(a['artifactId']) for a in index['artifacts']}) == 24 and
                    len({str(artifact_root(a)) for a in index['artifacts']}) == 24, 'artifact index duplicates roots/ids')
            for item in index['artifacts']:
                require(item['commit'] == SHAS[item['i']] and digest(item['archiveSha256']), 'artifact commit/archive SHA missing')
                if 'archiveSizeBytes' in item:
                    require(integer(item['archiveSizeBytes']) and item['archiveSizeBytes'] > 0, 'invalid verified archive size')
                if 'artifactName' in item:
                    require(item['artifactName'] == 'managed-evidence-' + settings['coordinatorRun'] + '-convex-helpers-' + item['variant'] + '-' + str(item['i']), 'artifact name differs from measured dispatch')
        if attempt('artifact index provenance and completeness', lambda: (index_check(), True)[1]):
            for item in index['artifacts']:
                row = next(r for r in rows if str(r['run']) == str(item['run']))
                checked = attempt('managed artifact %s c%d run%s' % (item['variant'], item['i'], item['run']), lambda item=item, row=row: audit_artifact(item, row, settings))
                if checked:
                    audited[(item['variant'], item['i'])] = checked
        summary = attempt('cold-chain authority, control equivalence and window totals', lambda: final_checks(rows, metrics, per))
    else:
        summary, settings = None, None
    file_sha(index_path)
    evidence = {'schema': 'quod-managed-window-independent-audit-v1', 'auditPassed': not issues and summary is not None,
                'issues': issues, 'verifiedManagedArtifacts': len(audited), 'settings': settings, 'summary': summary,
                'method': {'archiveAuthentication': 'caller verifies raw GitHub API archive bytes/digest before extraction; index retains that provenance',
                           'reviewedProductPins': {'action': ACTION_SHA, 'harness': HARNESS_SHA,
                               'managedSource': MANAGED_SOURCE_SHA, 'runtimeArchiveSha256': RUNTIME_ARCHIVE_SHA},
                           'timing': 'GitHub job completed_at minus started_at; rounded minutes sum ceil(each job seconds/60); queue/coordinator excluded',
                           'fingerprint': 'fixed trusted JSON-data-only Node localeCompare helper; full original fingerprints pinned independently',
                           'fingerprintHelperSha256': sha(FINGERPRINT_JS), 'binaryIdentity': 'supervised workerd SHA pinned to independently verified hardening; binary is not inside job evidence',
                           'claim': 'candidate reuse is a partial job hit; ordinary 119 files and release tests still execute'},
                'artifacts': index['artifacts'] if index else [],
                'fullSuiteFingerprints': fingerprint_diagnostics,
                'managedEvidence': [{'run': a['run'], 'variant': a['variant'], 'i': a['i'], 'commit': a['commit'],
                    'hit': a['result']['hit'], 'managedExecutions': a['result']['managedExecutions'],
                    'executionKey': a['execution']['key'], 'effectsSha256': a['result']['effectsSha256'],
                    'runtimeClassSha256': a['result']['runtimeClassSha256'],
                    'nativeFingerprintSha256': a['result']['nativeFingerprintSha256'],
                    'receiptSha256': (a['authority'] or a['recorded'])['sha256'],
                    'responseSha256': (a['authority'] or a['recorded'])['responseSha256'],
                    'cacheTransport': a['trace'], 'completion': a['data']['boundary']['completion']}
                    for _, a in sorted(audited.items())],
                'verifiedFiles': {str(p): h for p, h in sorted(file_hashes.items(), key=lambda item: str(item[0]))}}
    print(json.dumps(evidence, indent=2))
    return 0 if evidence['auditPassed'] else 1


if __name__ == '__main__':
    try:
        sys.exit(main())
    except (ValueError, TypeError, KeyError, IndexError, OSError, UnicodeError) as error:
        print(json.dumps({'schema': 'quod-managed-window-independent-audit-v1', 'auditPassed': False,
                          'issues': issues + [{'check': 'audit input', 'message': str(error)}]}, indent=2))
        sys.exit(1)

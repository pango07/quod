"""Audit retained real-dispatcher controls without executing artifact code."""
import base64
import hashlib
import json
import pathlib
import re
import sys

root = pathlib.Path(sys.argv[1])
issues, verified = [], {}
FIRST = 'managed dispatcher preserves the selected test outcome'
SECOND = 'managed dispatcher also executes the second selected file'
CASES = {
    'seed': ('pass', 'managed', 0, 1),
    'hit': ('pass', 'replay', 0, 1),
    'missing-trust': ('pass', 'managed', 0, 1),
    'wrong-trust': ('pass', 'managed', 0, 1),
    'forged-receipt': ('pass', 'managed', 0, 1),
    'unsupported-pass': ('unsupported-pass', 'ordinary', 0, 1),
    'guard-assertion': ('guard-assertion', 'ordinary', 0, 1),
    'failure': ('failure', 'ordinary', 1, 1),
    'unsupported-failure': ('unsupported-failure', 'ordinary', 1, 1),
    'multiple-files': ('pass', 'ordinary', 0, 2),
    'custom-config': ('pass', 'ordinary', 0, 1),
    'custom-reporter': ('pass', 'ordinary', 0, 1),
}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def attempt(label, action):
    try:
        return action()
    except (ValueError, TypeError, KeyError, IndexError, OSError, UnicodeError) as error:
        issues.append({'check': label, 'message': str(error)})
        return None


def pairs(values):
    result = {}
    for key, value in values:
        require(key not in result, 'duplicate JSON field: ' + key)
        result[key] = value
    return result


def read(path):
    require(not path.is_symlink(), 'symlink is not artifact evidence')
    return json.loads(path.read_text(), object_pairs_hook=pairs)


def sha(path):
    require(not path.is_symlink(), 'symlink is not artifact evidence')
    return hashlib.sha256(path.read_bytes()).hexdigest()


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def json_files(directory):
    return {path: read(path) for path in directory.rglob('*.json')}


summary = attempt('integration result', lambda: read(root / 'result.json')) or {}
records = {row['name']: row for row in summary.get('cases', [])}


def integration_inventory():
    require(summary['schema'] == 'quod-managed-dispatcher-integration-v1' and summary['passed'] is True,
            'integration driver did not complete')
    require(not summary.get('error'), 'driver reported ' + str(summary.get('error')))
    require([row['name'] for row in summary['cases']] == list(CASES), 'dispatcher case inventory differs')
    require(summary['checks'] and all(row['pass'] is True for row in summary['checks']), 'driver check failed')
    require(len({row['name'] for row in summary['checks']}) == len(summary['checks']), 'duplicate driver checks')
    require(len(summary['fixtureSourceSha256']) == 2 and all(re.fullmatch('[0-9a-f]{64}', value)
            for value in summary['fixtureSourceSha256']), 'fixture source identities missing')


attempt('complete independent case inventory', integration_inventory)


def verify_case(name, expected):
    mode, decision, exit_code, count = expected
    row = records[name]
    require(row['mode'] == mode and row['expected'] == {'decision': decision, 'exitCode': exit_code, 'tests': count},
            'artifact altered expected fixture semantics')
    directory = root / name
    result = read(directory / 'result.json')
    report = read(directory / 'report.json')
    require(row['resultSha256'] == sha(directory / 'result.json') and row['reportSha256'] == sha(directory / 'report.json'),
            'dispatcher/report byte digests differ')
    require(row['processExit'] == exit_code and result['exitCode'] == exit_code, 'dispatcher exit status differs')
    require(result['files'] and all(file['decision'] == decision for file in result['files']), 'execution path differs')
    rows = sorted([test['fullName'], test['status']] for file in report['testResults'] for test in file['assertionResults'])
    expected_rows = sorted([[FIRST, 'failed' if exit_code else 'passed']] + ([[SECOND, 'passed']] if count == 2 else []))
    require(rows == expected_rows and row['rows'] == rows, 'selected test names or states differ')
    require(len(row['selectedFiles']) == count, 'selected file count differs')
    data_files = json_files(directory)
    receipts = {path: data for path, data in data_files.items() if isinstance(data, dict) and
                data.get('schema') == 'quod-managed-receipt-v1-research'}
    require(sorted(str(path.relative_to(root)) for path in receipts) == sorted(row['receiptFiles']), 'receipt output inventory differs')
    if exit_code or decision == 'ordinary':
        require(not receipts and not any(file.get('receipt') for file in result['files']), 'failed or ordinary run emitted a reusable receipt')
    build_hashes = {data['sha256'] for data in data_files.values() if isinstance(data, dict) and
                    data.get('identity', {}).get('entry', '').endswith('/preflight.mjs')}
    workers = list(directory.rglob('worker.mjs'))
    if decision == 'replay':
        require(workers and all(sha(path) in build_hashes for path in workers), 'receipt hit executed managed test work')
    if decision == 'managed':
        require(any(sha(path) not in build_hashes for path in workers), 'managed path did not execute a real worker')
    for path, data in data_files.items():
        if path.name == 'invocation.json':
            folder = path.parent
            require(data['bundleSha256'] == sha(folder / 'worker.mjs') and data['configSha256'] == sha(folder / 'config.capnp'),
                    'actual worker/config byte identities differ')
            require(data['runtimePolicy']['denyModuleSha256'] == sha(folder / 'deny.mjs'), 'deny service byte identity differs')
            execution = read(folder / 'execution.json')
            require(execution['completed'] is True and execution['timedOut'] is False and execution['error'] is None,
                    'incomplete worker execution retained a result')
            require(execution['responseSha256'] == sha(folder / 'response.json'), 'actual worker response SHA differs')
        if isinstance(data, dict) and data.get('identity', {}).get('schema') == 'quod-managed-compile-v1-research':
            identity = data['identity']
            canonical = json.dumps(identity, sort_keys=True, separators=(',', ':'), ensure_ascii=False).encode()
            require(data['identitySha256'] == digest(canonical) and data['sha256'] == identity['bundleSha256'], 'compiler identity digest differs')
            require(data['closure']['finalBundleParsed'] is True and data['closure']['remainingDynamicImports'] == 0 and
                    data['closure']['remainingDynamicRequires'] == 0, 'compiler closure is incomplete')
    authority = read(root / 'authority' / (name + '.json'))
    require(len(authority) == len(row['trusted']), 'independent trust input count differs')
    for item, recorded in zip(authority, row['trusted']):
        require(item['key'] == recorded['key'] and item['receiptSha256'] == recorded['receiptSha256'] and
                item['file'].endswith('/' + recorded['file']), 'independent trust inputs differ')
    verified[name] = {'decision': decision, 'exitCode': exit_code, 'tests': count, 'result': result,
                      'receipts': receipts, 'workers': workers, 'authority': authority}


for name, expected in CASES.items():
    attempt(name, lambda name=name, expected=expected: verify_case(name, expected))


def verify_receipt_authority():
    seed = summary['seed']
    receipt_path = root / seed['file']
    receipt = read(receipt_path)
    require(seed['receiptSha256'] == sha(receipt_path), 'seed authority SHA differs from actual receipt bytes')
    require(receipt['execution']['key'] == seed['key'], 'seed execution key differs')
    identity_bytes = json.dumps(receipt['execution']['identity'], sort_keys=True, separators=(',', ':'), ensure_ascii=False).encode()
    require(seed['key'] == digest(b'quod-managed-execution-v1-research\n' + identity_bytes), 'execution key domain digest differs')
    response = base64.b64decode(receipt['response']['bytes'], validate=True)
    require(digest(response) == receipt['response']['sha256'], 'receipt response SHA differs')
    require(receipt['resultDigest'] == digest(('quod-managed-receipt-v1-research\n' + seed['key'] + '\n' + receipt['response']['bytes']).encode()),
            'receipt result domain digest differs')
    actual_responses = [path.parent / 'response.json' for path in verified['seed']['workers']]
    require(any(path.read_bytes() == response for path in actual_responses), 'receipt has no actual managed response witness')
    data = json.loads(response, object_pairs_hook=pairs)
    require(data['ok'] is True and len(data['rows']) == 1 and data['rows'][0]['state'] == 'pass', 'seed did not pass')
    boundary, completion = data['boundary'], data['boundary']['completion']
    require(all(boundary[key] == [] for key in ('unsupported', 'unguardable', 'symbolUnguardable', 'timerErrors', 'unhandledErrors', 'pendingTimerDetails')),
            'seed has unsupported or pending asynchronous effects')
    require(boundary['pendingTimers'] == 0 and completion['sealed'] is True and completion['reason'] == '' and
            all(completion[key] == 0 for key in ('pendingTimerCallbacks', 'pendingMicrotasks', 'pendingEventCallbacks')),
            'seed completion is not sealed')
    hit = verified['hit']['authority']
    require(len(hit) == 1 and hit[0]['key'] == seed['key'] and hit[0]['receiptSha256'] == seed['receiptSha256'] and
            hit[0]['file'].endswith('/' + seed['cachedFile']) and sha(root / seed['cachedFile']) == seed['receiptSha256'],
            'true hit lacks external seed authority')
    require(verified['missing-trust']['authority'] == [], 'missing authority case was not empty')
    require(verified['wrong-trust']['authority'][0]['receiptSha256'] == '0' * 64, 'wrong authority case did not change digest')
    forged = verified['forged-receipt']['authority'][0]
    require(forged['receiptSha256'] == seed['receiptSha256'] and
            sha(root / records['forged-receipt']['trusted'][0]['file']) != seed['receiptSha256'],
            'forged cache case did not retain original independent authority')
    require(verified['guard-assertion']['decision'] == 'ordinary' and verified['guard-assertion']['exitCode'] == 0 and
            verified['unsupported-failure']['exitCode'] == 1, 'ordinary fallback failed to control both outcomes')
    return {'key': seed['key'], 'receiptSha256': seed['receiptSha256'], 'responseSha256': digest(response)}


receipt = attempt('independent seed authority and actual result witness', verify_receipt_authority)
files = {}
for path in sorted(root.rglob('*')):
    if path.is_file():
        value = attempt('artifact file ' + str(path.relative_to(root)), lambda path=path: sha(path))
        if value:
            files[str(path.relative_to(root))] = value
evidence = {'auditPassed': not issues and len(verified) == len(CASES) and receipt is not None,
            'issues': issues, 'cases': len(verified), 'trueHits': 1 if receipt else 0,
            'ordinaryFallbackOutcomesVerified': ['pass', 'failure', 'managed guard-induced failure becomes ordinary pass'],
            'receipt': receipt, 'verifiedFiles': files}
print(json.dumps(evidence, indent=2))
sys.exit(0 if evidence['auditPassed'] else 1)

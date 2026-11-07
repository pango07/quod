"""Audit the two retained mixed managed/native executions as JSON data only."""
import base64
import hashlib
import json
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
FIRST = 'packages/convex-helpers/server/quod-managed-remainder.first.test.ts'
SECOND = 'packages/convex-helpers/server/quod-managed-remainder.second.test.ts'
FIRST_NAME = 'managed remainder candidate passes once'
SECOND_NAME = 'ordinary remainder preserves its outcome'
CANDIDATE_MARKER = 'QUOD_MANAGED_REMAINDER_CANDIDATE'
ORDINARY_MARKER = 'QUOD_MANAGED_REMAINDER_ORDINARY'
issues, verified = [], {}


def require(value, message):
    if not value:
        raise ValueError(message)


def pairs(items):
    result = {}
    for key, value in items:
        require(key not in result, 'duplicate JSON field: ' + key)
        result[key] = value
    return result


def raw(file):
    require(file.is_file() and not file.is_symlink(), 'missing/symbolic evidence: ' + str(file))
    return file.read_bytes()


def read(file):
    return json.loads(raw(file), object_pairs_hook=pairs)


def sha(file):
    return hashlib.sha256(raw(file)).hexdigest()


def attempt(name, action):
    try:
        return action()
    except (ValueError, TypeError, KeyError, IndexError, OSError, UnicodeError) as error:
        issues.append({'check': name, 'message': str(error)})


def specs(rows):
    return sorted((row['absolute'], row['project']) for row in rows)


def tests(report):
    return sorted([file['name'], test['fullName'], test['status']]
                  for file in report['testResults'] for test in file['assertionResults'])


summary = attempt('integration summary', lambda: read(root / 'result.json')) or {}


def inventory():
    require(summary['schema'] == 'quod-managed-remainder-integration-v1' and summary['passed'] is True and
            not summary.get('error'), 'mixed integration driver did not complete')
    require(summary['fixtureFiles'] == [FIRST, SECOND] and
            [r['name'] for r in summary['cases']] == ['ordinary-pass', 'ordinary-fail'], 'fixture/case inventory differs')
    require(len(summary['checks']) == 20 and all(c['pass'] is True for c in summary['checks']) and
            len({c['name'] for c in summary['checks']}) == 20, 'driver checks are incomplete')
    require(summary['fixtureSourceSha256'] == [sha(root / 'fixtures' / pathlib.Path(f).name)
            for f in (FIRST, SECOND)], 'retained fixture source identities differ')


attempt('independent fixture inventory', inventory)


def audit(mode, expected_exit):
    name, directory = 'ordinary-' + mode, root / ('ordinary-' + mode)
    row = next(r for r in summary['cases'] if r['name'] == name)
    result = read(directory / 'result.json')
    full, ordinary = read(directory / 'report.json'), read(directory / 'ordinary-report.json')
    selection = read(directory / 'ordinary-remainder-selection.json')
    request = read(directory / 'ordinary-remainder-request.json')
    preparation = read(directory / 'preparation.json')
    response = read(directory / 'selected-response.json')
    first, second = str(pathlib.Path(result['project']) / FIRST), str(pathlib.Path(result['project']) / SECOND)
    expected_specs = sorted([(first, 'convex-helpers'), (second, 'convex-helpers')])
    expected_rows = sorted([[first, FIRST_NAME, 'passed'],
                           [second, SECOND_NAME, 'passed' if expected_exit == 0 else 'failed']])
    require(row['mode'] == mode and row['expectedExit'] == row['processExit'] == expected_exit and
            result['exitCode'] == expected_exit, 'process outcome differs')
    require(row['resultSha256'] == sha(directory / 'result.json') and
            row['reportSha256'] == sha(directory / 'report.json') and
            row['selectionSha256'] == sha(directory / 'ordinary-remainder-selection.json'), 'retained byte hashes differ')
    require(result['schema'] == 'quod-managed-dispatch-v1' and result['mode'] == 'v0' and
            result['hit'] is False and result['fallback'] is False and result['managedExecutions'] == 1 and
            len(result['files']) == 1 and result['files'][0]['file'] == FIRST and
            result['files'][0]['decision'] == 'managed', 'managed path was not executed exactly once')
    require(len(result['ordinaryRuns']) == 1 and result['ordinaryRuns'][0]['exitCode'] == expected_exit and
            result['ordinaryRepeatedCandidate'] is False, 'ordinary remainder was repeated or replaced')
    require(request['schema'] == 'quod-ordinary-remainder-request-v1' and
            specs(request['selected']) == specs(preparation['selected']) == expected_specs and
            request['excluded'] == {'absolute': first, 'project': 'convex-helpers'}, 'actual original selection differs')
    require(selection['schema'] == 'quod-ordinary-remainder-selection-v1' and
            selection['matched'] is True and selection['runStarted'] is True and selection['completed'] is True and
            selection['exitCode'] == expected_exit and selection['unhandledErrors'] == 0 and
            not selection.get('error') and not selection.get('closeError'), 'native selection or completion was not sealed')
    require(specs(selection['selected']) == expected_specs and
            specs(selection['excluded']) == [(first, 'convex-helpers')] and
            specs(selection['remainder']) == specs(selection['executed']) == [(second, 'convex-helpers')],
            'native project identity or exclusion differs')
    require(len(ordinary['testResults']) == 1 and tests(ordinary) == [r for r in expected_rows if r[0] == second],
            'ordinary worker repeated candidate or omitted the second test')
    require(len(full['testResults']) == 2 and full['numTotalTests'] == 2 and
            full['success'] is (expected_exit == 0) and tests(full) == expected_rows and row['rows'] == expected_rows,
            'merged test names, states, or outcome differ')
    require(next(f for f in full['testResults'] if f['name'] == second) == ordinary['testResults'][0],
            'merged report altered native output')
    states = {'pass': 'passed', 'fail': 'failed'}
    require(sorted([r['file'], r['name'], states[r['state']]] for r in result['rows']) == expected_rows,
            'normalized rows do not describe both actual files')
    require(response['ok'] is True and len(response['rows']) == 1 and response['rows'][0]['name'] == FIRST_NAME and
            response['rows'][0]['state'] == 'pass' and response['boundary']['completion']['sealed'] is True and
            response['boundary']['unsupported'] == [], 'managed candidate did not pass supported execution')
    worker = read(directory / 'managed' / 'response.json')
    require(response == worker and read(directory / 'managed' / 'execution.json')['completed'] is True,
            'candidate result lacks actual managed execution witness')
    invocation = read(directory / 'managed' / 'invocation.json')
    require(invocation['bundleSha256'] == sha(directory / 'managed' / 'worker.mjs') and
            invocation['configSha256'] == sha(directory / 'managed' / 'config.capnp'), 'worker/config bytes differ')
    ordinary_log = raw(directory / 'ordinary-remainder.log').decode('utf8')
    combined = raw(root / (name + '.log')).decode('utf8')
    require(ordinary_log.count(CANDIDATE_MARKER) == 0 and ordinary_log.count(ORDINARY_MARKER) == 1 and
            combined.count(CANDIDATE_MARKER) == 1 and
            sum(CANDIDATE_MARKER in ' '.join(values) for _, values in response['boundary']['logs']) == 1,
            'candidate or remainder console effects repeated')
    receipts = [p for p in directory.rglob('*.json') if read(p).get('schema') == 'quod-managed-receipt-v1-research']
    callback = read(directory / 'record-callback.json')
    require(len(result['receipts']) == len(receipts) == callback['recordCalls'] == (1 if expected_exit == 0 else 0),
            'failed whole command published a passing candidate receipt')
    if expected_exit == 0:
        receipt = read(receipts[0])
        require(base64.b64decode(receipt['response']['bytes'], validate=True) == raw(directory / 'managed' / 'response.json') and
                result['receipts'][0]['receiptSha256'] == sha(receipts[0]), 'passing receipt differs from actual managed response')
    verified[name] = {'exitCode': expected_exit, 'selected': 2, 'managed': 1, 'ordinary': 1,
                      'completeTests': 2, 'receipts': len(receipts)}


for mode, expected_exit in [('pass', 0), ('fail', 1)]:
    attempt(mode, lambda mode=mode, expected_exit=expected_exit: audit(mode, expected_exit))

files = {}
for file in sorted(root.rglob('*')):
    if file.is_file():
        value = attempt('file ' + str(file.relative_to(root)), lambda file=file: sha(file))
        if value:
            files[str(file.relative_to(root))] = value
print(json.dumps({'schema': 'quod-managed-remainder-independent-audit-v1',
                  'auditPassed': not issues and len(verified) == 2,
                  'issues': issues, 'cases': verified, 'verifiedFiles': files}, indent=2))
sys.exit(0 if not issues and len(verified) == 2 else 1)

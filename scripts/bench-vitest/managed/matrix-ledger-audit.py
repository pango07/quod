#!/usr/bin/env python3
"""Independent data-only audit of the fixed passing matrix ledger experiment.

Requires the separately SHA-verified original matrix evidence. Imports or
executes no downloaded source. This does not authenticate a real cache SDK.
"""
import argparse
import base64
import collections
import hashlib
import json
import pathlib
import subprocess
import sys
import zipfile

MATRIX_PIN = 'ff279ea5b59b359d26985f08f22e0749f9eccbde'
LEDGER_PIN = '7ed4694e500f8417ab761c21320da3d91eeff9af'
ZIP_SHA = 'd33f5b267adb6b9f2233fbc3df11d67e28c0fc6c0da017d7786de4d1dabcb282'
ZIP_SIZE = 2073421
ROOT = pathlib.Path(__file__).resolve().parents[3]
def sha(value):
    return hashlib.sha256(value).hexdigest()
def canonical(value):
    return json.dumps(value, sort_keys=True, ensure_ascii=False, separators=(',', ':'), allow_nan=False).encode()
def unique(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError('duplicate JSON field: ' + key)
        result[key] = value
    return result
def decode(value):
    return json.loads(value, object_pairs_hook=unique, parse_constant=lambda x: (_ for _ in ()).throw(ValueError(x)))
def read(file):
    assert file.is_file() and not file.is_symlink() and file.stat().st_size <= 32 * 1024 * 1024
    return decode(file.read_bytes())
def source_hash(name):
    return sha(subprocess.check_output(['git', 'show', LEDGER_PIN + ':scripts/bench-vitest/managed/' + name], cwd=ROOT))
def audit(args):
    root = pathlib.Path(args.ledger).resolve()
    matrix = pathlib.Path(args.matrix).resolve()
    archive = pathlib.Path(args.archive)
    assert archive.stat().st_size == ZIP_SIZE and sha(archive.read_bytes()) == ZIP_SHA
    with zipfile.ZipFile(archive) as zipped:
        files = [row for row in zipped.infolist() if not row.is_dir()]
        assert len(files) == 112 and len({row.filename for row in files}) == len(files)
        for row in files:
            name = pathlib.PurePosixPath(row.filename)
            assert not name.is_absolute() and '..' not in name.parts and '\\' not in row.filename
            assert sha(root.joinpath(*name.parts).read_bytes()) == sha(zipped.read(row))
        assert {row.filename for row in files} == {str(file.relative_to(root)) for file in root.rglob('*') if file.is_file()}
    assert not any(file.is_symlink() for file in root.rglob('*'))
    result = read(root / 'matrix-ledger/result.json')
    previous_audit = read(root / 'matrix-independent-audit.json')
    assert previous_audit['passed'] and previous_audit['issues'] == [] and previous_audit['sourcePin'] == MATRIX_PIN
    assert result['passed'] and not any(result[name] for name in ['productionEnabled', 'cachePublished', 'actualSdkAuthenticationClaimed'])
    assert result['checks'] and all(row['passed'] is True for row in result['checks'])
    names = [row['name'] for row in result['checks']]
    assert len(set(names)) == len(names)
    actual = read(matrix / 'result.json')
    assert actual['passed'] and actual['coverage']['managedPassing'] == 108 and actual['coverage']['disagreements'] == 0
    supported = {row['instanceId']: row for row in actual['instances'] if row['decision'] == 'managed'}
    native_bytes = (matrix / 'native-selection.json').read_bytes()
    report_bytes = (matrix / 'native-report.json').read_bytes()
    native = decode(native_bytes)
    modules = {row['instanceId']: row for row in native['executed']}
    records = result['receipts']
    assert len(records) == len({row['instanceId'] for row in records}) == 108
    assert {row['instanceId'] for row in records} == set(supported)
    validator_sha, fixture_sha = source_hash('matrix-receipt.mjs'), source_hash('matrix-receipt-fixtures.mjs')
    states = collections.Counter()
    total_bytes = 0
    duplicate_files = 0
    skipped_files = 0
    for item in records:
        instance_id = item['instanceId']
        data = (root / 'matrix-ledger/receipts' / (instance_id + '.json')).read_bytes()
        total_bytes += len(data)
        assert len(data) == item['bytes'] <= 2 * 1024 * 1024 and sha(data) == item['receiptSha256']
        receipt = decode(data)
        assert data == canonical(receipt)
        execution, conformance = receipt['execution'], receipt['conformance']
        identity = execution['identity']
        assert identity['schema'] == 'quod-managed-matrix-execution-v2-research'
        assert execution['key'] == sha(identity['schema'].encode() + b'\n' + canonical(identity)) == item['key']
        instance = identity['instance']
        assert instance == {key: supported[instance_id][key] for key in ['instanceId', 'project', 'file', 'pool']}
        evidence = matrix / supported[instance_id]['evidenceDirectory']
        built = read(evidence / 'test-build.json')
        assert identity['compile'] == built['identity'] and identity['preparation'] == read(evidence / 'preparation.json')
        assert identity['compile']['policy']['keepNames'] is True
        assert identity['supervisorPolicy']['receiptValidatorSha256'] == validator_sha
        assert identity['supervisorPolicy']['experimentSha256'] == fixture_sha
        assert identity['host'] == actual['host'] == identity['nativePolicy']['host']
        assert conformance['instance'] == instance and conformance['executionKey'] == execution['key']
        assert conformance['nativeReportSha256'] == sha(report_bytes) and conformance['nativeSelectionSha256'] == sha(native_bytes)
        assert conformance['nativePolicySha256'] == sha(canonical(identity['nativePolicy']))
        conformance_sha = sha(conformance['schema'].encode() + b'\n' + canonical(conformance))
        assert conformance_sha == receipt['conformanceSha256']
        encoded = receipt['response']['bytes']
        response_bytes = base64.b64decode(encoded, validate=True)
        assert base64.b64encode(response_bytes).decode() == encoded
        assert response_bytes == (evidence / 'managed/response.json').read_bytes()
        assert sha(response_bytes) == receipt['response']['sha256'] == item['responseSha256']
        response = decode(response_bytes)
        assert receipt['resultDigest'] == sha((receipt['schema'] + '\n' + execution['key'] + '\n' + conformance_sha + '\n' + encoded).encode())
        state_rows = lambda rows: collections.Counter((row['name'], row['state']) for row in rows)
        assert state_rows(response['rows']) == state_rows(conformance['rows']) == state_rows(modules[instance_id]['rows'])
        assert response['files'][0]['name'] == conformance['file']['name'] == modules[instance_id]['relativeModuleId']
        assert response['files'][0]['state'] == conformance['file']['state'] == modules[instance_id]['state']
        boundary = response['boundary']; completion = boundary['completion']
        assert response['ok'] and completion['sealed'] and completion['intrinsicLockdown'] and completion['reason'] == ''
        assert all(boundary[name] == [] for name in ['unsupported', 'unguardable', 'symbolUnguardable', 'timerErrors', 'unhandledErrors'])
        assert boundary['pendingTimers'] == 0 and all(completion[name] == 0 for name in ['pendingTimerCallbacks', 'pendingMicrotasks', 'pendingEventCallbacks'])
        states.update(row['state'] for row in response['rows'])
        duplicate_files += len({row['name'] for row in response['rows']}) < len(response['rows'])
        skipped_files += response['files'][0]['state'] == 'skip'
    assert states == {'pass': 3946, 'skip': 1} and duplicate_files > 0 and skipped_files == 1
    assert total_bytes == result['totalReceiptBytes'] == 12822280
    upper = 512 + sum(((row['bytes'] + 2) // 3) * 4 + 512 for row in records)
    assert upper == result['transportBytesUpperBound'] == 17152308 < 32 * 1024 * 1024
    return {'schema': 'quod-managed-matrix-ledger-independent-audit-v1', 'passed': True, 'issues': [],
            'matrixSourcePin': MATRIX_PIN, 'ledgerSourcePin': LEDGER_PIN, 'artifactSha256': ZIP_SHA,
            'receipts': len(records), 'states': dict(states), 'duplicateNamedFiles': duplicate_files,
            'allSkippedFiles': skipped_files, 'totalReceiptBytes': total_bytes, 'transportBytesUpperBound': upper,
            'fixtureChecks': len(names), 'actualSdkAuthenticationClaimed': False,
            'limit': 'Data-only receipt fidelity and provenance audit; no SDK cache service or fresh-runner replay claim.'}
if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('ledger'); parser.add_argument('matrix'); parser.add_argument('--archive', required=True)
    args = parser.parse_args()
    try:
        result = audit(args)
    except Exception as error:
        result = {'schema': 'quod-managed-matrix-ledger-independent-audit-v1', 'passed': False,
                  'issues': [type(error).__name__ + ': ' + str(error)]}
    print(json.dumps(result, indent=2))
    sys.exit(0 if result['passed'] else 1)

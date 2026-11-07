"""Independent data checks for the isolated frozen-slot experiment only."""
import base64
import hashlib
import json

CONTRACT = 'quod-managed-v0-protectors-frozen-slots-investigation'
POLICY = 'quod-managed-frozen-array-promise-slots-v1-probe'
SLOTS = ('Array.prototype.constructor', 'Array.prototype[Symbol.iterator]', 'Array[Symbol.species]',
         'Promise.prototype.constructor', 'Promise.prototype.then', 'Promise.resolve', 'Promise[Symbol.species]')
PROTECTOR_NAMES = {'explicit frozen native slot policy', 'four native targets are frozen'}
for slot in SLOTS:
    PROTECTOR_NAMES |= {slot + suffix for suffix in (' retains original native descriptor identity',
        ' strict write cannot mutate', ' strict delete cannot mutate', ' Reflect.set rejects without mutation')}
PROTECTOR_NAMES |= {
    'Array native species getter returns its receiver', 'Promise native species getter returns its receiver',
    'native array species operations retain results', 'native Promise.resolve preserves identity and value',
    'native Promise chains settle normally', 'native subclass species retains its constructor',
    'inherited array constructor assignment follows frozen data semantics',
    'inherited array iterator assignment follows frozen data semantics',
    'inherited promise then assignment follows frozen data semantics',
    'inherited subclass static resolve assignment follows frozen data semantics',
    'explicit own data definitions remain supported', 'ordinary Array.push instance shadowing remains supported',
    'ordinary Error.name instance shadowing remains supported', 'managed Date.now can be temporarily replaced and restored',
    'private ALS facade survives native Promise policy', 'Object descriptor writes remain sticky unsupported',
    'Reflect descriptor writes remain sticky unsupported', 'host fetch remains sticky unsupported',
    'new contract is explicit in returned boundary', 'native failed writes add no undeclared unsupported marker',
    'all native and managed effects complete under the new policy',
}
POLICY_NAMES = {'frozen policy is explicit in execution identity', 'legacy policy has a different execution domain key',
    'authenticated legacy receipt misses frozen policy', 'frozen receipt rejects legacy expected execution',
    'coherent frozen key with legacy response still misses', 'new receipt retains exact declared frozen response policy'}
for name in ('missing runtime boundary contract', 'missing runtime intrinsic policy',
             'legacy runtime boundary contract', 'changed runtime intrinsic policy'):
    POLICY_NAMES.add(name + ' rejects identity')
for name in ('legacy response policy', 'missing response intrinsic policy', 'changed response intrinsic policy'):
    POLICY_NAMES |= {name + ' rejects validation', name + ' cannot be recorded'}


def require(condition, message):
    if not condition:
        raise ValueError(message)


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
    names |= {'admission integration records actual dependency source', 'admission integration records actual glob module names',
        'admission integration repeated real compilation keeps identity', 'admission integration repeated real compilation hits receipt',
        'admission integration unrelated file leaves real compile identity unchanged', 'admission integration unrelated file hits recorded receipt'}
    for name in ('test source comment', 'dependency value', 'glob module-name addition', 'declared environment', 'configuration seed'):
        names |= {'admission integration ' + name + ' changes recompiled identity', 'admission integration ' + name + ' misses recorded receipt'}
    return names


def names_pass(checks, expected, label):
    require(len(checks) == len(expected) and {c['name'] for c in checks} == expected and
            all(c['pass'] is True for c in checks), label + ' inventory changed or failed')


def verify_protector(data):
    require(data['ok'] is True and data['policy'] == POLICY, 'protector fixture policy/result differs')
    names_pass(data['checks'], PROTECTOR_NAMES, '51 protector checks')
    b, c = data['boundary'], data['boundary']['completion']
    require(b['contract'] == CONTRACT and b['intrinsicPolicy'] == POLICY and b['replayEligible'] is False, 'protector boundary policy differs')
    require(set(b['unsupported']) == {'intrinsic mutation constructor', 'intrinsic mutation then', 'global fetch'} and
            len(b['unsupported']) == 3, 'native failed writes lost or added sticky effects')
    require(c['sealed'] is True and c['intrinsicLockdown'] is True and c['reason'] == '' and b['pendingTimers'] == 0 and
            all(c[k] == 0 for k in ('pendingTimerCallbacks', 'pendingMicrotasks', 'pendingEventCallbacks')) and
            b['timerErrors'] == b['unhandledErrors'] == b['unguardable'] == b['symbolUnguardable'] == [], 'protector completion is not sealed')
    return len(data['checks'])


def verify_policy_receipts(root):
    def pairs(rows):
        result = {}
        for key, value in rows:
            require(key not in result, 'duplicate receipt JSON key')
            result[key] = value
        return result
    def read(name):
        p = root / name
        require(not p.is_symlink(), 'symbolic policy receipt evidence')
        raw = p.read_bytes()
        return raw, json.loads(raw, object_pairs_hook=pairs)
    def canonical(value):
        return json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=False, allow_nan=False).encode()
    h = lambda value: hashlib.sha256(value).hexdigest()
    def checked(name):
        raw, value = read(name)
        require(raw == canonical(value), 'cross-policy receipt is not canonical')
        require(set(value) == {'schema', 'researchOnly', 'execution', 'expected', 'response', 'resultDigest'} and
                value['schema'] == 'quod-managed-receipt-v1-research' and value['researchOnly'] is True and
                value['expected'] == {'count': 1, 'file': 'fixture.test.mjs'}, 'cross-policy receipt schema/scope differs')
        key = h(b'quod-managed-execution-v1-research\n' + canonical(value['execution']['identity']))
        require(key == value['execution']['key'], 'cross-policy execution domain digest differs')
        encoded = value['response']['bytes']
        response = base64.b64decode(encoded, validate=True)
        require(base64.b64encode(response).decode() == encoded and h(response) == value['response']['sha256'] and
                h(('quod-managed-receipt-v1-research\n' + key + '\n' + encoded).encode()) == value['resultDigest'], 'cross-policy response domain digest differs')
        return h(raw), value, json.loads(response, object_pairs_hook=pairs)
    _, result = read('result.json')
    require(result['schema'] == 'quod-managed-receipt-fixtures-v1' and result['cacheEnabled'] is False and result['researchOnly'] is True,
            'receipt matrix scope differs')
    names_pass(result['checks'], receipt_names(), 'original 107 receipt checks')
    names_pass(result['policyChecks'], POLICY_NAMES, '16 cross-policy checks')
    policy = result['policyAdmission']
    require(policy['intrinsicPolicy'] == POLICY and policy['boundaryContract'] == CONTRACT and
            policy['responseOrigin'] == 'constructed cross-policy responses; not runtime execution', 'cross-policy submatrix scope differs')
    seed_sha, seed, seed_data = checked('seed-receipt.json')
    require(seed_sha == result['seed']['receiptSha256'] and seed['execution']['key'] == policy['currentExecutionKey'], 'current policy seed identity differs')
    require(seed['execution']['identity']['runtimePolicy']['intrinsicPolicy'] == POLICY and
            seed['execution']['identity']['runtimePolicy']['boundaryContract'] == CONTRACT and
            seed_data['boundary']['intrinsicPolicy'] == POLICY and seed_data['boundary']['contract'] == CONTRACT, 'new policy omitted from key/response')
    require(seed_data['ok'] is True and seed_data['rows'] == [{'name': 'closed managed fixture', 'state': 'pass', 'errors': []}] and
            seed_data['files'] == [{'name': 'fixture.test.mjs', 'state': 'pass', 'errors': []}], 'constructed seed workload differs')
    old_sha, old, old_data = checked('legacy-policy-receipt.json')
    require(old_sha == policy['legacyReceipt']['sha256'] and old['execution']['key'] == policy['legacyExecutionKey'] != policy['currentExecutionKey'], 'legacy policy key/byte evidence differs')
    expected_old = json.loads(json.dumps(seed['execution']['identity']))
    del expected_old['runtimePolicy']['boundaryContract']
    del expected_old['runtimePolicy']['intrinsicPolicy']
    expected_old_data = json.loads(json.dumps(seed_data))
    expected_old_data['boundary']['contract'] = 'quod-managed-v0-investigation'
    del expected_old_data['boundary']['intrinsicPolicy']
    require(old['execution']['identity'] == expected_old and old_data == expected_old_data and
            policy['legacyReceipt']['file'] == 'legacy-policy-receipt.json', 'legacy cross-policy evidence was relabeled')
    relabeled_sha, relabeled, relabeled_data = checked('frozen-key-legacy-response.json')
    require(policy['relabeledLegacyResponse']['file'] == 'frozen-key-legacy-response.json' and
            relabeled_sha == policy['relabeledLegacyResponse']['sha256'] and relabeled['execution'] == seed['execution'] and
            relabeled_data == old_data, 'coherent new key/legacy response rejection evidence differs')
    return {'originalChecks': len(result['checks']), 'policyChecks': len(result['policyChecks']), 'legacyKey': old['execution']['key'],
            'currentKey': seed['execution']['key']}

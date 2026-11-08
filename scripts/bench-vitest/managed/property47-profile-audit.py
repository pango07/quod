#!/usr/bin/env python3
"""Independent data-only reader for finite property47 gate42430045283.

Usage: --repository TRUSTED_SOURCE --project-repository TRUSTED_C11_GIT_OBJECTS
       --trusted-reader EXACT_AA904B92_DATA_PRIMITIVES EVIDENCE_INDEX.json
Exact reviewed run/job/source/archive authority is pinned before index reads.
Only fixed-SHA trusted local reader bytes may be loaded; no artifact/project
code, compiler, workload, Python cache or payload import is executed.
This completed synthetic-authority research gate makes no SDK or window claim.
"""
import argparse
import collections
import copy
import datetime
import hashlib
import json
import math
import os
import pathlib
import re
import stat
import subprocess
import sys
import types

RUN = 42430045283
SOURCE = 'f67fe14033de76f0b26b436cef4e5d50b0af1ee3'
ARTIFACTS = {'controls': (13597347940, 7995168, '430921dc229ce17c7b79011d285b7e2c074cf46baaf4f6ec8eb32499f740a460', 'managed-property47-controls'), 'default-integration': (13597188553, 1479806, 'e98548e5ff23fcda58f095b7e0d7ab2d9e65833fb1143121392e06ef5ba5f777', 'managed-property47-default-integration'), 'profile': (13597128639, 2838055, 'd7aef74d32f13de2909c192fffa0e1a7b1cadb3cf4152ce249c620c4a74483a9', 'managed-property47-profile')}
JOB = (128576718582, 'benchmark-managed-property47-profile', '2026-11-08T00:30:25Z', '2026-11-08T00:39:17Z')
ARCHIVE_PROFILES = {'controls': {'compressedBytes': 100000000, 'expandedBytes': 500000000, 'fileBytes': 33554432, 'paths': 20000}, 'default-integration': {'compressedBytes': 100000000, 'expandedBytes': 500000000, 'fileBytes': 33554432, 'paths': 20000}, 'profile': {'compressedBytes': 100000000, 'expandedBytes': 500000000, 'fileBytes': 33554432, 'paths': 20000}, 'baseline': {'compressedBytes': 100000000, 'expandedBytes': 500000000, 'fileBytes': 33554432, 'paths': 20000}}
REVIEW_APPROVED = True
EXPECTED_SOURCE = 'f67fe14033de76f0b26b436cef4e5d50b0af1ee3'
PRIMITIVE_READER_SHA256 = 'aa904b92abf882b78f876884e4539257a847fca977fcd75aa119a14b496ad01b'
BASELINE = (13587034973, 36186785, 'f8e20f3a68cc3fbec7ce01a5ea90e8b50a1d558cbf08146a7fb049e242adc282')
BASELINE_RUN = 42406036816
BASELINE_SOURCE = 'ff279ea5b59b359d26985f08f22e0749f9eccbde'
PROJECT_COMMIT = '5eaa7e5160736054d53f8b71e2c522835eb427e0'
COMPILER = 'a87f1f8fafea2e0f1b4a1e299199f5581172a2fe90f59882d3c64eb50f68acaf'
CORE = 'f01cd9a55f6eceb913c5b402481fd6b6dd7266191d8182f97ca60f158765c703'
BATCH_POLICY = 'closed-adjacent-full-fences-before-any-candidate-worker-v1-diagnostic'
PROFILE_LABEL = 'reviewed-original-property47-single-instance-v1-research'
TARGET = {'instanceId': 'ccda7135f8d1ede102763427b3efabcef8130aaed24822ce92a5ec7ef94a05b3',
          'project': 'convex-helpers', 'file': 'packages/convex-helpers/server/stream.property.test.ts', 'pool': 'forks'}
ORDINARY = 'packages/convex-helpers/functionReferences.test.ts'
SOURCE_SHA = ['9798bab303233bca846a89ccb39ce28bd968329e0704f3e5d67adca934c11c75',
              '09544a8dd35b40754de45d058ccb5627f86c7248c12ba3ce1a55c6ad3dd65fa6']
SOURCE_PATHS = [TARGET['file'], ORDINARY]
SENTINEL = 'QUOD property profile native failure sentinel'
MODES = ['seed', 'hit', 'refresh', 'mixed-routing', 'rollback', 'rollback-native-failure']
ROLES = ['controls', 'default-integration', 'profile']
ARTIFACT_NAMES = {'controls': 'managed-property47-controls', 'default-integration': 'managed-property47-default-integration',
                  'profile': 'managed-property47-profile'}
FROZEN_FILES = {
 '.github/workflows/gate.yml': '1dd151fc3fb9392d65736dc7df23ddb54360229246e946dab412052e2f4746c4',
 'matrix-input-dispatcher.mjs': '36aa6899058b2efdbc55c5e072dc7a7129974189c768533a55a953104b59231a',
 'matrix-candidate-profiles.mjs': '840ef60f36368ace3edb40a23d6984d0366c6a9a89a3f4d8d5d34c7a46bc3fce',
 'property47-profile-probe.mjs': 'e5181c1964397a10ac8bca5d14aa00a345df2fb39edf2de7e83c8224cf0a5042'}
SOURCE_NAMES = ['compiler.mjs','resolved-compile-certificate.mjs','preparation.mjs','executor.mjs','prelude.mjs','runner.mjs',
 'matrix-native.mjs','matrix-input-dispatcher.mjs','matrix-candidates.mjs','matrix-receipt.mjs','matrix-receipt-fixtures.mjs',
 'matrix-probe.mjs','matrix-candidate-envelope.mjs','matrix-candidate-profiles.mjs','property47-profile-probe.mjs',
 'matrix-input-dispatcher-fixtures.mjs','matrix-input-proof-fixtures.mjs','matrix-batch-dispatcher-fixtures.mjs',
 'compiler-fixtures/check.mjs','package.json','.github/workflows/gate.yml']
MAX_API = 16 * 1024 * 1024
MAX_FILE = 32 * 1024 * 1024
COMMON = [
 'complete original120 selection retains one exact property candidate and119 ordinary instances',
 'active profile roster and selector bytes are bound into every current execution',
 'current logical report preserves all original4131 project/name/states and API file states',
 'native selection is exact and complete for the nonhit IDs',
 'all lookup callbacks precede any record or publication',
 'only sealed current data can omit the exact property counterpart',
 'whole-command exit and publication retain sticky failure semantics',
 'this diagnostic claims synthetic authority only and never a full-job hit']
EXTRA = {
 'seed': ['one guarded cold batch retains four full fences and one fresh47-state worker',
          'native120 provides actual cold conformance before one receipt record'],
 'hit': ['one authenticated current-input proof skips the only candidate compile and worker'],
 'refresh': ['coherent stale-source certificate triggers one guarded compile then exact byte-ledger refresh with no worker'],
 'mixed-routing': ['one unrelated historical route cannot create a second candidate or lookup'],
 'rollback': ['real node:fs source import declines the reserved batch before any candidate worker',
              'full native120 fallback runs once without omitting a candidate','reserved failure poisons only its disposable process'],
 'rollback-native-failure': ['real node:fs source import declines the reserved batch before any candidate worker',
              'full native120 fallback retains the actual appended native assertion failure without retry',
              'reserved failure poisons only its disposable process']}
CHILD_CHECKS = {mode: COMMON + EXTRA[mode] for mode in MODES}
PARENT_CHECKS = ['reviewed original source bytes exist before any child','default108 profile label and exact roster remain unchanged',
 'property profile has only the independently reviewed exact tuple']
PARENT_CHECKS += ['nonprimitive or unknown profile rejected: ' + value for value in ['"null"','"[object Object]"','""','47','true','"' + PROFILE_LABEL + '"','"unknown-property-profile"']]
PARENT_CHECKS += ['missing target leaves every other actual instance ordinary']
PARENT_CHECKS += ['same ID cannot substitute property identity: ' + key for key in ['project','file','pool']]
PARENT_CHECKS += ['duplicate property instance rejected']
for mode in MODES:
    PARENT_CHECKS += [mode + ': all exact fresh child controls pass']
    if mode == 'seed': PARENT_CHECKS += ['seed retains exactly one synthetic conformed candidate']
    PARENT_CHECKS += [mode + ': original source bytes restored before next process']
assert len(PARENT_CHECKS) == 28 and [len(CHILD_CHECKS[mode]) for mode in MODES] == [10,9,9,9,11,11]


def sha(body):
    return hashlib.sha256(body).hexdigest()


def canonical(value):
    return json.dumps(value,sort_keys=True,separators=(',',':'),ensure_ascii=False,allow_nan=False)


def unique(pairs):
    result = {}
    for key,value in pairs:
        assert key not in result, 'duplicate JSON key'
        result[key] = value
    return result


def raw(file, maximum=MAX_FILE):
    file = pathlib.Path(file)
    before = file.lstat()
    assert stat.S_ISREG(before.st_mode) and 0 <= before.st_size <= maximum, 'bounded regular file required'
    observation = lambda row: (row.st_dev,row.st_ino,row.st_size,row.st_mtime_ns,row.st_ctime_ns)
    with file.open('rb') as stream:
        assert observation(os.fstat(stream.fileno())) == observation(before)
        body = stream.read(maximum + 1)
        assert observation(os.fstat(stream.fileno())) == observation(before)
    after = file.lstat()
    assert stat.S_ISREG(after.st_mode) and observation(before) == observation(after)
    assert len(body) == before.st_size <= maximum, 'sampled raw data changed'
    return body


def read(file, maximum=MAX_FILE):
    return json.loads(raw(file,maximum),object_pairs_hook=unique,
        parse_constant=lambda value: (_ for _ in ()).throw(AssertionError(value)))


def exact(value, names):
    assert isinstance(value,dict) and set(value) == set(names), 'unexpected schema fields'


def checked_pins():
    assert all(value is not None for value in [RUN,SOURCE,ARTIFACTS,JOB,ARCHIVE_PROFILES]), 'PENDING future authority pins; no index may choose them'
    assert REVIEW_APPROVED is True, 'disabled until complete source and exact actual profiles are approved'
    assert type(RUN) is int and RUN > 0 and SOURCE == EXPECTED_SOURCE
    assert set(ARTIFACTS) == set(ROLES) and set(ARCHIVE_PROFILES) == set(ROLES + ['baseline'])
    assert len(JOB) == 4 and type(JOB[0]) is int and JOB[1] == 'benchmark-managed-property47-profile'
    for role in ROLES:
        value = ARTIFACTS[role]
        assert isinstance(value,tuple) and len(value) == 4 and type(value[0]) is int
        assert 0 < value[1] <= 100000000 and re.fullmatch('[0-9a-f]{64}',value[2]) and value[3] == ARTIFACT_NAMES[role]
    assert len({value[0] for value in ARTIFACTS.values()}) == 3
    for profile in ARCHIVE_PROFILES.values():
        assert profile == {'compressedBytes':100000000,'expandedBytes':500000000,'fileBytes':33554432,'paths':20000}


def load_primitives(file, roots, payloads):
    supplied = pathlib.Path(file).absolute(); path = supplied.resolve(strict=True)
    assert not any(path.is_relative_to(pathlib.Path(root).resolve(strict=True)) for root in roots)
    assert path not in {pathlib.Path(value).resolve(strict=True) for value in payloads}
    body = raw(supplied,256*1024)
    assert sha(body) == PRIMITIVE_READER_SHA256, 'trusted data-reader source differs'
    module = types.ModuleType('trusted_property47_data_primitives')
    module.__file__ = str(path); module.__package__ = None
    # This exact already-reviewed body is trusted local reader source. Neither
    # importlib nor pyc can replace the verified bytes; payload code never enters.
    exec(compile(body,str(path),'exec',dont_inherit=True),module.__dict__)
    module.raw = raw
    return module


def git_bytes(repository,commit,file):
    assert re.fullmatch('[0-9a-f]{40}',commit)
    assert isinstance(file,str) and not file.startswith('/') and '..' not in pathlib.PurePosixPath(file).parts
    env = {**os.environ,'GIT_NO_LAZY_FETCH':'1','GIT_TERMINAL_PROMPT':'0'}
    body = subprocess.check_output(['git','show',commit+':'+file],cwd=repository,env=env)
    assert 0 < len(body) <= 2*1024*1024
    return body


def inventory(value,names,flag='pass',counts=True):
    assert value['passed'] is True and value['checks'] == [{'name':name,flag:True} for name in names]
    assert len(set(names)) == len(names)
    if counts: assert value['checkCount'] == value['expectedCheckCount'] == len(names)


def profile_value():
    roster = [TARGET]
    return {'schema':'quod-managed-matrix-candidate-profile-v1-research','label':PROFILE_LABEL,'roster':roster,
        'rosterSha256':sha(json.dumps([[TARGET[key] for key in ['instanceId','project','file','pool']]],separators=(',',':')).encode())}


def check_execution(P,value,row,sources,property_profile=True):
    identity = value['identity']
    assert value['key'] == P.domain(P.EXECUTION,identity) == row['executionKey']
    copied = copy.deepcopy(identity)
    if property_profile:
        assert copied['supervisorPolicy'].pop('candidateProfile') == profile_value()
        assert copied['supervisorPolicy'].pop('candidateProfileSelectorSha256') == sources['matrix-candidate-profiles.mjs']
    P.check_execution(copied,row,sources)
    assert identity['runtimePolicy']['supervisor']['watchdogMs'] == (180000 if row['file'] == TARGET['file'] else 30000)
    return identity


def receipt(P,body,execution,original,seed_native_root):
    value = P.decode(body)
    assert P.canonical(value).encode() == body
    P.exact(value,['schema','researchOnly','execution','conformance','conformanceSha256','response','resultDigest'])
    assert value['schema'] == P.RECEIPT and value['researchOnly'] is True and value['execution'] == execution
    key = execution['key']; c = value['conformance']
    P.exact(c,['schema','executionKey','instance','nativeReportSha256','nativeSelectionSha256','nativePolicySha256','file','rows'])
    assert c['schema'] == P.CONFORMANCE and c['executionKey'] == key and c['instance'] == execution['identity']['instance']
    assert c['nativeReportSha256'] == sha(raw(seed_native_root/'native-report.json'))
    assert c['nativeSelectionSha256'] == sha(raw(seed_native_root/'native-selection.json'))
    assert c['nativePolicySha256'] == sha(canonical(execution['identity']['nativePolicy']).encode())
    assert value['conformanceSha256'] == P.domain(P.CONFORMANCE,c)
    response = P.b64(value['response']['bytes'],16*1024*1024)
    assert value['response']['encoding'] == 'base64' and value['response']['sha256'] == sha(response)
    assert value['resultDigest'] == sha((P.RECEIPT+'\n'+key+'\n'+value['conformanceSha256']+'\n'+value['response']['bytes']).encode())
    data = P.decode(response); P.boundary(data)
    expected = collections.Counter((row['name'],row['state']) for row in original['rows'])
    assert collections.Counter((row['name'],row['state']) for row in data['rows']) == expected
    assert collections.Counter((row['name'],row['state']) for row in c['rows']) == expected
    assert c['file'] == {'name':original['relativeModuleId'],'state':original['state']}
    assert data['files'][0]['name'] == original['relativeModuleId'] and data['files'][0]['state'] == original['state']
    return value,response

LEGACY_CHECKS = ['the actual full native control is validated once before minting', 'duplicate native selection JSON keys cannot mint conformance', 'duplicate native reporter JSON keys cannot mint conformance', 'actual conformed per-instance round trip: 77ef865cec7a852de947263518160401146ba54b0ba51b0e5efd7a82eba2f714', 'actual conformed per-instance round trip: cc43286e7db83b64f075d29944ecd5c26b825622d067bfc01ccc3a358867137d', 'actual conformed per-instance round trip: d9ccfd26194ce498fc14f7472f616507547e784d52fe48f126cc5051c5861c93', 'actual conformed per-instance round trip: 0f7a96505f8f3f06bb762cb7a8ebce3ca85543fc0a72ec2bd7c3372fdd09897c', 'actual conformed per-instance round trip: 7cdf73f08f3a1972e2e86499133d5630c0e62865545938eac5c4412a8764b398', 'actual conformed per-instance round trip: 12af88cff50043e299109e6c8cebf013288e820a54e3edc87b9aa29bd2b4c784', 'actual conformed per-instance round trip: 01916f6ab4f674068bb91ca840a06db8837b2d6f13591d487d83d9a3ae37e867', 'actual conformed per-instance round trip: 9c057bb6ed92df37834b4c94ee9f094b16bf3db288ff6f1c60d047850cada39b', 'actual conformed per-instance round trip: d96892ac9d5964b685e681902d0379c83730a9301b7375e71652b37741b67e1b', 'actual conformed per-instance round trip: f79884922311e1057c3d6fda535a9de821271728407bbd034416f4407f7bbd20', 'actual conformed per-instance round trip: 2511595948aae3360ca77100e8737c8215563ffdeb363d310a19eb13f65e4979', 'actual conformed per-instance round trip: 521245ea4b63c707d0aa96d37663b0953d13b9a47a5116ab3e74f364dc7c0755', 'actual conformed per-instance round trip: ba3f39cd145acc1d55d619b50992cec04a59291e4b376680fbf75eb3bde9bf31', 'actual conformed per-instance round trip: c12ac7dcd30b50a6bd58fccbc2b53a18086114f01353e97c117eb915cc459934', 'actual conformed per-instance round trip: 74797924bd359f2c15c25ea9faf92a4b65d54b21a40d346e56c41d1df0be4136', 'actual conformed per-instance round trip: e76ef1fc5d477e3049a3f3cd291ef6477625687e3ad4b98c03bc73b840a8e9a2', 'actual conformed per-instance round trip: 326e358c55648ff3f8bcde78cb0ba3064a38d90784ee08cbe1f3e9c2757a89c2', 'actual conformed per-instance round trip: 0f9d74426547617052dc4141eced71c132642e28bc5d822b7b395294c35c9962', 'actual conformed per-instance round trip: ba6f0d3cc3f06771eb3b815a34cbc5ff7dffbaba88b1c0d0d493ccc079b7519e', 'actual conformed per-instance round trip: 7bac8dabd563b9c1147becb8be77b9ebf3b30fe58665031aef2307630b075a48', 'actual conformed per-instance round trip: a8eb2ea57b9db5808780181cefad06db279686f903b2c085c1b1c8a7372ad918', 'actual conformed per-instance round trip: 654e14236a6b95e6b11e0498cd3efb2a99ee46b2077718e6f563f9b29aacfe9d', 'actual conformed per-instance round trip: c82440bd9af4d0596e568ab82925067bdb678ae5de158cb7020d07ea3f949fe3', 'actual conformed per-instance round trip: 53c5240a95b8548f437ebdd08c46b9fd7f1c132544eb533a62993a845808fe14', 'actual conformed per-instance round trip: b83c3917511da22f923d2913b946d3bf96191fc19f8441aa475db1bf4fbab5b5', 'actual conformed per-instance round trip: d3858dce08341199295c8afd13254c116693977c3f12d3195189c8819b7640b9', 'actual conformed per-instance round trip: 26e061a6149dbaddc195a35a54777fe34f2c52755d79e1255c999fc395962cea', 'actual conformed per-instance round trip: 2dc84025ccdf4c77183591d31722d1a9bbd675296a3f63b7ed9637f3471fa2df', 'actual conformed per-instance round trip: db69e7e5c375d3d47ef28887f986461217e3c053a9ba6705c33b417f82efbac1', 'actual conformed per-instance round trip: 8c472c967de0eb922e4260db812a302942762bf8dbd15e6fea23be0e90ae6902', 'actual conformed per-instance round trip: 86549d0a93f037d74568d8a7e5e616316c5499845a92f210b4bbad2c7d2fbc97', 'actual conformed per-instance round trip: c078178997212846222b7e38d3da669ad83595d099bab46f786ef5fca34631ee', 'actual conformed per-instance round trip: a18ab8d21eecebb2a3307a0c456a2348791515a7d0cedeac031f25aadc55c4af', 'actual conformed per-instance round trip: 4fd58b58aed4ac160a095ef39be6460d501f5edcee5df929592e4f718b58bd42', 'actual conformed per-instance round trip: 673914346b1be6aa90924436137e954102af96e319883839735da18e72b75c8a', 'actual conformed per-instance round trip: 1a8ae2e08c561dff57c64c367e7a45f1c028e3e5de9a75e68d098baf380b4433', 'actual conformed per-instance round trip: 624fbb7416754520c9a1dd92722f63466c3c9326ada6c5ecde1825e709f187e3', 'actual conformed per-instance round trip: 7de6c5259c8e71caf5ae5e0c3b40cf6a8e676a95335ec39b0982eb81af5496da', 'actual conformed per-instance round trip: 692930d94685c1651ae81e7a2b5a2c68fb18ccdb313f92086a79eab99fcd675a', 'actual conformed per-instance round trip: e895bed87dd5f7dd566827d21f61e0ea2c445ed235d7cf5ce0beeda6dcde6469', 'actual conformed per-instance round trip: ed063c4d73d659f1b6b8bb8b8ab9f3070b4f727c1475107ee9c4bc7a00b1741e', 'actual conformed per-instance round trip: 8e2ae05a09c74c21778df11abfdf95e403116ea44809cb8ee04cdb256e36c2e9', 'actual conformed per-instance round trip: 2a1285c6cca885bdcbc71a1db45ae79dec705878d9bb7fc715b5c910af0ac6ff', 'actual conformed per-instance round trip: 351db3ae1d6e7eeb576e23939525c531212afd3623559bb49e6f3575d5e66583', 'actual conformed per-instance round trip: 7a29784ee750071ece659df8e3a777205c28d7ef171d2d6651059170a8b56813', 'actual conformed per-instance round trip: a5fef28bc2c020f5ebfda7ade361f9c53d127518c9c996274079914169a2f2b2', 'actual conformed per-instance round trip: e39d31824d8c4423d241a534c4ccfee78a36bdb4c7280b451d567f8f1aca2fa6', 'actual conformed per-instance round trip: 874630b4c288f45f1fe624b35139df0a75fbd32b016a93d3849f1935705fe6db', 'actual conformed per-instance round trip: 6130003303d93c61545ba4180a9e45ac659a3367c8a7cb629b5e61220e2d9ab4', 'actual conformed per-instance round trip: 3955c64fa4682c6ca6b8217d5a191af8f44330838803036bc5722c8d3ae16763', 'actual conformed per-instance round trip: 5b5cbc0696960e7cb9e8759d1240042053fc0aedbc67da77df0f544a574e1b4e', 'actual conformed per-instance round trip: 1c2e9147401f4fff375e87c9925f3305002e8a8abf8db1419b75d1b369c59762', 'actual conformed per-instance round trip: f4093ecf83d31dbfa7e8af723c1a1dff67a85c55720c2f5353ac6180a70c831e', 'actual conformed per-instance round trip: 19067d5c34f5257bb071f48ea5537b5276a2dabafcb52c40f5cd59deadf07d61', 'actual conformed per-instance round trip: 9ac736ae9cc0d84b198225a694135c39bc7f9a049a1501ef02c46ffa59da0cfe', 'actual conformed per-instance round trip: 9ae2ec230ad0e1027780cdf3df7192ef8b5620fe866491c8a552a4185c54d7e8', 'actual conformed per-instance round trip: 2c3125c7e8563f906ce802f9c659e3eebb4e91265dd3791bbebb5e3f7a9ba803', 'actual conformed per-instance round trip: 1d10f5efaffe8c56556886e36f02c3568b54fe396209fa203be65749d032b4a3', 'actual conformed per-instance round trip: 4566e0b89cbf2548493f5d60cf18ac16ecbc3dcd3453372496848d188f879265', 'actual conformed per-instance round trip: 5ff407c34fe3380294eb10246d7cb7d26695d1a186c8d257a55a5a512d9c5682', 'actual conformed per-instance round trip: 068617b6d97a228a35323ca4a9b782a18adb5295eb8d878caf2cf82647cdc5c3', 'actual conformed per-instance round trip: cccb11cd0285ab6a327b386ab13a3659ba7019904983ff0eaeb9541b8160032e', 'actual conformed per-instance round trip: 583abb3cba0f3ccc0f67ae88c06f70e822040d64db7e93ad21e5f0daa1bb8621', 'actual conformed per-instance round trip: 0db00838622ad2a40db0ca8b9ed514b735d780f75b92176ab17e6a8508a29d9e', 'actual conformed per-instance round trip: a70c603b5fd46c30e26a886d7a6d2979416eda77787c4c7b85532ebcbac23591', 'actual conformed per-instance round trip: 9f4e9f827f1219ab283cda836c3f90cc2c1a0d837731a0f157f474223ac72d8e', 'actual conformed per-instance round trip: 2c900ae396429563cf36c81a745955b5c87e627a9c59b8fd4e5d1271fed38024', 'actual conformed per-instance round trip: 28a673923ffa6803e59fbcdcca6abbb550ed700346d04487cd56f00f2bccf034', 'actual conformed per-instance round trip: 567cb2654928c714846b9895704b27b500e36038174138e6dfd051de45381aef', 'actual conformed per-instance round trip: 4173907b66bbce893abebd24116d6be12997932f8452a7dab6fa0ab99f3282eb', 'actual conformed per-instance round trip: 867dfcc25a41e4428e19420f78e1aa0a029917a8a9cd582185a86b999a83c35d', 'actual conformed per-instance round trip: f78536b3e1ce21aef87f13ab494546d74ffa5538e482d2ba974084ee827a3cf0', 'actual conformed per-instance round trip: f6979b530ae8f446615da032661670bb6af1d5d0f1aeffc0a588be85474912b8', 'actual conformed per-instance round trip: c7b79f7db04d758ad97eb566a3d3e47efd3ed8f99e735363e9062c241616b803', 'actual conformed per-instance round trip: 81c564ffd782d052a2c79d53da938aff2eaf5c958860cfbff841557f135e5581', 'actual conformed per-instance round trip: 97abc53f4620240869775318bb895c2391d2abf531b375e25d009d9e2a6a3b54', 'actual conformed per-instance round trip: 072fc36c262c555fe837cdad322536e4189dec845b81cc3ac77f2841be7fcda6', 'actual conformed per-instance round trip: ab3777d611f43666879b98f11a676d114e7e9a82338382d630b7b9f220b932b6', 'actual conformed per-instance round trip: 2edf2e75109b646d629c2012a290dfe045b5c24213d1675ecf1fdd89633f91d6', 'actual conformed per-instance round trip: 799d0380052aaf38183a60863c84a3ef427c7cf44cd3b5cf96c94fb5f61c1cba', 'actual conformed per-instance round trip: 19a56d6f79c31dea46f7d54c51b8d9c0fb5557fcbcbfa3bc191ac126f28e9579', 'actual conformed per-instance round trip: 1856333295b1ed24c6717596802ccbf0b0cd680ac0b30c36d31e2bcea825c88b', 'actual conformed per-instance round trip: 4f2ca3f952a45a2ce88e9914ccd8fe1fb7b5992298237eef1ab5d20454647f46', 'actual conformed per-instance round trip: 3c1fd3912edefca2b8b4082d558586feeb0c2419856d752489ec5cc2c3940d65', 'actual conformed per-instance round trip: c1b80c3442b8bf6b81532b907e78c29945d2b61b6d719ed801a37113bd6b0338', 'actual conformed per-instance round trip: 9276c9c6d95abd996d7f4973f34c1d348b5f79fa510fb8697352a058a1fbdaa5', 'actual conformed per-instance round trip: 84bf01761c725c0c8477267a540c5d2433a8d44754cdc3ad0266bbe41e72b549', 'actual conformed per-instance round trip: c90423a4006c8a3742da92d197db65b77ec90e9da8c144c8c1d4480d26e66553', 'actual conformed per-instance round trip: f02e46aee519c821243429ce35e026b08fe437d4eb625ee01a7c0a13a1b3733d', 'actual conformed per-instance round trip: 9b3232a9ad20e596c2cf393ba07ec0d2a07f55bbb9a4ffabf8c5897f768350fa', 'actual conformed per-instance round trip: f17aa662beff90a98d19188e4767d1f69ba6d824d4bd76267f25490c659f9977', 'actual conformed per-instance round trip: 9c98091d49e2d4cd31e18e176ce89140caca028fac36c5f28962210906a11b32', 'actual conformed per-instance round trip: 1c76f1fc71b951a0a9ff847d1dc8a8241952a8507a983179ff7d6dc9fec56e1e', 'actual conformed per-instance round trip: ccda7135f8d1ede102763427b3efabcef8130aaed24822ce92a5ec7ef94a05b3', 'actual conformed per-instance round trip: 56c3bdbe07c5dd00d90739e97deb628e1dee6f2992c8774e86e1377adbf0ffee', 'actual conformed per-instance round trip: 91e1148ef3bea68f12f82b47f8c19f0d781e093d994cbb5fd464f08f3e4e2fe3', 'actual conformed per-instance round trip: b914a0563bff596e7c27a4a252de2d847569f26aabf54979920d9f4ceffe8a6b', 'actual conformed per-instance round trip: 22150b1aa059d0add1f6926d0dd966f1d81cd250623f337cf669f9b67604a910', 'actual conformed per-instance round trip: 2a2ecf86c83b44c1a8c66addf4180b1664c8b9c277c0f7da99cd2f140643d3b9', 'actual conformed per-instance round trip: 5ca8e8d1f3cdf8e9d108bbfa02596000d1153eb4e6e768d5ceb72dfb7da67d17', 'actual conformed per-instance round trip: fda7801b0a2c8935e4aa2cc11e5629673273b14c33d0fd57356c14c3edb1cd92', 'actual conformed per-instance round trip: 53f3e7de8aecdd2312ade69a52badbd031713e769726d9e0c15913bd317b7bc3', 'actual conformed per-instance round trip: 649f1af5b0592c2ac2d93b87ccc7160ed48dba57f55a8754de3625d2b251578e', 'actual conformed per-instance round trip: ac919d83737b1d2afe950578e5134f2cede397a215e2c205848a692c6dea1acb', 'actual conformed per-instance round trip: 6af5e4aba1578902b8e6eb6d169c915ddddb3e449623473b755123c581923b71', 'actual conformed per-instance round trip: 7a7d86a16f78533fb82ab6ba945428a6c42f71c34d7c92fe363e467619d6a713', 'actual conformed per-instance round trip: fb5f025cb6d00fefb19070c4bfb66dfc45a8aa682b060957d1abdac73cb174a5', 'actual conformed per-instance round trip: c4dfc40ed5172e41d7a9e1cc12d09112c719e43f4a0f7e20b2902cc2d9ff8ac9', 'all actual managed instances were retained exactly once', 'every actual receipt fits the2MiB authenticated entry bound', 'base64 receipts and metadata fit the one-pack32MiB transport budget', 'the actual all-skipped original file is reusable without changing its state', 'the actual duplicate-named original tests retain their full multiplicity', 'missing external authority is a miss', 'wrong external byte digest is a miss', 'a digest embedded inside the candidate is not an authority argument', 'a valid receipt cannot cross actual project identity', 'coherently rehashed forged candidate misses the unchanged external authority', 'even an authenticated malformed result must match native conformance', 'actual failed state cannot reuse', 'incomplete test rows cannot reuse', 'unsupported caught capability cannot reuse', 'unsealed completion cannot reuse', 'nonempty completion reason cannot reuse', 'pending timers cannot reuse', 'pending timer callbacks cannot reuse', 'pending microtasks cannot reuse', 'pending event callbacks cannot reuse', 'unhandled asynchronous failure cannot reuse', 'file-level errors cannot reuse', 'unknown diagnostic fields cannot reuse', 'old runtime policy cannot reuse', 'old intrinsic policy cannot reuse', 'coherently changed native capability policy cannot reuse', 'passed retry diagnostics are preserved as primitive data', 'skipped failure diagnostics cannot be normalized into a pass', 'plain cached conformance cannot mint another receipt', 'a plain object cannot stand in for validated fresh native control', 'a changed native conformance token cannot mint', 'response accessors are rejected before being invoked', 'the forged response getter was never invoked', 'response toJSON hooks cannot forge validation', 'the forged serializer was never invoked', 'an incomplete native producer cannot mint conformance', 'a failed native producer cannot mint conformance', 'a native API/full JSON row mismatch cannot mint conformance', 'a forged selected native identity cannot mint conformance', 'a failed original JSON report cannot mint conformance', 'fresh native context must match the exact current matrix project']

DISPATCH_CHECKS = ['seed: the two-file scope retains one reviewed candidate and one ordinary instance', 'seed: all thirteen original and appended states remain present', 'seed: publication observes the final whole-command exit exactly once', 'seed: full logical artifact binds its exact bytes and final publication exit', 'seed: logical file identities and states retain their actual native or replay provenance', 'seed: actual native public selection matches the declared scope', 'seed: ordinary control executes exactly once in the real native batch', 'seed: the complete nonhit native batch retains the candidate', 'seed: the managed candidate control executes exactly once in its fresh realm', 'a conformed cold pass records one receipt after the complete native batch', 'empty and absent test-line selections share the actual unfiltered identity', 'nonempty test-line selection remains distinct and outside reviewed candidates', 'real line-filter preparation remains unsupported under reviewed configuration', 'hit: the two-file scope retains one reviewed candidate and one ordinary instance', 'hit: all thirteen original and appended states remain present', 'hit: publication observes the final whole-command exit exactly once', 'hit: full logical artifact binds its exact bytes and final publication exit', 'hit: logical file identities and states retain their actual native or replay provenance', 'hit: actual native public selection matches the declared scope', 'hit: ordinary control executes exactly once in the real native batch', 'hit: the real native batch omits the hit candidate', 'hit: a hit starts no managed worker', 'external synthetic authority admits the current exact execution and preserves all logical states', 'source-byte-miss: the two-file scope retains one reviewed candidate and one ordinary instance', 'source-byte-miss: all thirteen original and appended states remain present', 'source-byte-miss: publication observes the final whole-command exit exactly once', 'source-byte-miss: full logical artifact binds its exact bytes and final publication exit', 'source-byte-miss: logical file identities and states retain their actual native or replay provenance', 'source-byte-miss: actual native public selection matches the declared scope', 'source-byte-miss: ordinary control executes exactly once in the real native batch', 'source-byte-miss: the complete nonhit native batch retains the candidate', 'source-byte-miss: the managed candidate control executes exactly once in its fresh realm', 'changing source bytes while retaining all assertions declines the old input proof', 'fresh-resolution-miss: the two-file scope retains one reviewed candidate and one ordinary instance', 'fresh-resolution-miss: all thirteen original and appended states remain present', 'fresh-resolution-miss: publication observes the final whole-command exit exactly once', 'fresh-resolution-miss: full logical artifact binds its exact bytes and final publication exit', 'fresh-resolution-miss: logical file identities and states retain their actual native or replay provenance', 'fresh-resolution-miss: actual native public selection matches the declared scope', 'fresh-resolution-miss: ordinary control executes exactly once in the real native batch', 'fresh-resolution-miss: the complete nonhit native batch retains the candidate', 'fresh-resolution-miss: the managed candidate control executes exactly once in its fresh realm', 'a new higher-priority JS resolution target declines the old proof despite unchanged prior source bytes', 'missing-authority: the two-file scope retains one reviewed candidate and one ordinary instance', 'missing-authority: all thirteen original and appended states remain present', 'missing-authority: publication observes the final whole-command exit exactly once', 'missing-authority: full logical artifact binds its exact bytes and final publication exit', 'missing-authority: logical file identities and states retain their actual native or replay provenance', 'missing-authority: actual native public selection matches the declared scope', 'missing-authority: ordinary control executes exactly once in the real native batch', 'missing-authority: the complete nonhit native batch retains the candidate', 'missing-authority: the managed candidate control executes exactly once in its fresh realm', 'missing-authority: invalid receipt authority/schema misses and executes managed plus native controls', 'wrong-authority: the two-file scope retains one reviewed candidate and one ordinary instance', 'wrong-authority: all thirteen original and appended states remain present', 'wrong-authority: publication observes the final whole-command exit exactly once', 'wrong-authority: full logical artifact binds its exact bytes and final publication exit', 'wrong-authority: logical file identities and states retain their actual native or replay provenance', 'wrong-authority: actual native public selection matches the declared scope', 'wrong-authority: ordinary control executes exactly once in the real native batch', 'wrong-authority: the complete nonhit native batch retains the candidate', 'wrong-authority: the managed candidate control executes exactly once in its fresh realm', 'wrong-authority: invalid receipt authority/schema misses and executes managed plus native controls', 'authenticated-malformed: the two-file scope retains one reviewed candidate and one ordinary instance', 'authenticated-malformed: all thirteen original and appended states remain present', 'authenticated-malformed: publication observes the final whole-command exit exactly once', 'authenticated-malformed: full logical artifact binds its exact bytes and final publication exit', 'authenticated-malformed: logical file identities and states retain their actual native or replay provenance', 'authenticated-malformed: actual native public selection matches the declared scope', 'authenticated-malformed: ordinary control executes exactly once in the real native batch', 'authenticated-malformed: the complete nonhit native batch retains the candidate', 'authenticated-malformed: the managed candidate control executes exactly once in its fresh realm', 'authenticated-malformed: invalid receipt authority/schema misses and executes managed plus native controls', 'unsupported-pass: the two-file scope retains one reviewed candidate and one ordinary instance', 'unsupported-pass: all thirteen original and appended states remain present', 'unsupported-pass: publication observes the final whole-command exit exactly once', 'unsupported-pass: full logical artifact binds its exact bytes and final publication exit', 'unsupported-pass: logical file identities and states retain their actual native or replay provenance', 'unsupported-pass: actual native public selection matches the declared scope', 'unsupported-pass: ordinary control executes exactly once in the real native batch', 'unsupported-pass: the complete nonhit native batch retains the candidate', 'unsupported-pass: the managed candidate control executes exactly once in its fresh realm', 'caught unsupported fetch makes managed failure non-authoritative and real native pass wins', 'unsupported-failure: the two-file scope retains one reviewed candidate and one ordinary instance', 'unsupported-failure: all thirteen original and appended states remain present', 'unsupported-failure: publication observes the final whole-command exit exactly once', 'unsupported-failure: full logical artifact binds its exact bytes and final publication exit', 'unsupported-failure: logical file identities and states retain their actual native or replay provenance', 'unsupported-failure: actual native public selection matches the declared scope', 'unsupported-failure: ordinary control executes exactly once in the real native batch', 'unsupported-failure: the complete nonhit native batch retains the candidate', 'unsupported-failure: the managed candidate control executes exactly once in its fresh realm', 'unsupported-failure: no receipt is recorded or materialized after any whole-command failure', 'unsupported managed execution cannot hide an actual native assertion failure', 'assertion-failure: the two-file scope retains one reviewed candidate and one ordinary instance', 'assertion-failure: all thirteen original and appended states remain present', 'assertion-failure: publication observes the final whole-command exit exactly once', 'assertion-failure: full logical artifact binds its exact bytes and final publication exit', 'assertion-failure: logical file identities and states retain their actual native or replay provenance', 'assertion-failure: actual native public selection matches the declared scope', 'assertion-failure: ordinary control executes exactly once in the real native batch', 'assertion-failure: the complete nonhit native batch retains the candidate', 'assertion-failure: the managed candidate control executes exactly once in its fresh realm', 'assertion-failure: no receipt is recorded or materialized after any whole-command failure', 'assertion-failure: supported assertion or file hook failure stays sticky', 'assertion-failure: the actual failure reaches its intended row or file-only channel', 'hook-failure: the two-file scope retains one reviewed candidate and one ordinary instance', 'hook-failure: all thirteen original and appended states remain present', 'hook-failure: publication observes the final whole-command exit exactly once', 'hook-failure: full logical artifact binds its exact bytes and final publication exit', 'hook-failure: logical file identities and states retain their actual native or replay provenance', 'hook-failure: actual native public selection matches the declared scope', 'hook-failure: ordinary control executes exactly once in the real native batch', 'hook-failure: the complete nonhit native batch retains the candidate', 'hook-failure: the managed candidate control executes exactly once in its fresh realm', 'hook-failure: no receipt is recorded or materialized after any whole-command failure', 'hook-failure: supported assertion or file hook failure stays sticky', 'hook-failure: the actual failure reaches its intended row or file-only channel', 'outer-cleanup-error: the two-file scope retains one reviewed candidate and one ordinary instance', 'outer-cleanup-error: all thirteen original and appended states remain present', 'outer-cleanup-error: publication observes the final whole-command exit exactly once', 'outer-cleanup-error: full logical artifact binds its exact bytes and final publication exit', 'outer-cleanup-error: logical file identities and states retain their actual native or replay provenance', 'outer-cleanup-error: actual native public selection matches the declared scope', 'outer-cleanup-error: ordinary control executes exactly once in the real native batch', 'outer-cleanup-error: the complete nonhit native batch retains the candidate', 'outer-cleanup-error: the managed candidate control executes exactly once in its fresh realm', 'outer-cleanup-error: no receipt is recorded or materialized after any whole-command failure', 'outer-cleanup-error: sealed managed exception remains sticky without reusable row/file shape', 'outer-cleanup-error: the actual runner catch retains the exception and closed empty-denial boundary', 'outer-cleanup-error: the exact ordinary counterpart passes all thirteen states', 'ordinary-failure: the two-file scope retains one reviewed candidate and one ordinary instance', 'ordinary-failure: all thirteen original and appended states remain present', 'ordinary-failure: publication observes the final whole-command exit exactly once', 'ordinary-failure: full logical artifact binds its exact bytes and final publication exit', 'ordinary-failure: logical file identities and states retain their actual native or replay provenance', 'ordinary-failure: actual native public selection matches the declared scope', 'ordinary-failure: ordinary control executes exactly once in the real native batch', 'ordinary-failure: the complete nonhit native batch retains the candidate', 'ordinary-failure: the managed candidate control executes exactly once in its fresh realm', 'ordinary-failure: no receipt is recorded or materialized after any whole-command failure', 'ordinary remainder failure prevents publication of an otherwise passing managed candidate']

PROOF_CHECKS = ['compressed envelope reconstructs exact original receipt bytes', 'deduplicated envelope reconstructs exact original certificate bytes', 'an embedded digest cannot replace external envelope authority', 'wrong external envelope authority rejects intact compressed bytes', 'another current routing key rejects authenticated candidate bytes', 'authenticated truncated gzip rejects before any receipt lookup', 'authenticated false expanded length rejects', 'authenticated decompression cannot exceed the independent output cap', 'authenticated reconstructed certificate SHA mismatch rejects', 'authenticated ambiguous envelope JSON rejects', 'a real fresh source validation issues an opaque frozen proof', 'validated proof and actual full bundle produce the exact same complete execution identity and key', 'mutating a diagnostic identity copy cannot mutate the private proof', 'plain object cannot replace a real module-issued compile proof', 'shallow copy cannot replace a real module-issued compile proof', 'serialized token cannot replace a real module-issued compile proof', 'real compile proof cannot authorize changed current options', 'real compile proof cannot authorize changed current preparation']

COMPILER_CHECKS = ['literal-import final AST is closed', 'literal-import all native imports use ALS only', 'computed-import final AST is closed', 'computed-import all native imports use ALS only', 'computed-import injects private sticky rejection', 'evaluation final AST is closed', 'evaluation all native imports use ALS only', 'evaluation injects private sticky rejection', 'metadata final AST is closed', 'metadata all native imports use ALS only', 'metadata injects private sticky rejection', 'unchanged preparation identity repeats', 'literal dependency is recorded', 'declared configuration changes identity', 'literal dependency bytes change identity', 'unrelated file leaves closed identity unchanged', 'glob names are recorded', 'glob records resolved module targets', 'glob module-name mutation changes identity', 'private-namespace rejects preparation', 'private-generated-namespace rejects preparation', 'private-adapter rejects preparation', 'unused-private-adapter rejects preparation', 'native-fs rejects preparation', 'native-alias rejects preparation', 'native-prefix-only rejects preparation', 'als-namespace rejects preparation', 'als-default rejects preparation', 'als-extra rejects preparation', 'als-reexport rejects preparation', 'als-alias-namespace rejects preparation', 'als-alias-extra rejects preparation', 'als-dynamic rejects preparation', 'native-require rejects preparation', 'type-private-namespace rejects original source before erasure', 'type-private-adapter rejects original source before erasure', 'resolved symlink to private adapter rejects preparation', 'named ALS uses the managed facade', 'bare native ALS alias uses the managed facade', 'configuration key order is canonical', 'adding a denied foreign glob name changes closed identity', 'removing that glob name restores the exact prior closed identity', 'original class and function names are an explicit compile policy', 'self-glob final module graph is closed', 'self-glob admits only its exact actual entry', 'self-glob actual original assertions pass', 'self-glob private completion remains sealed', 'self-glob denial inventory is exact after caught calls', 'foreign-glob final module graph is closed', 'foreign-glob admits only its exact actual entry', 'foreign-glob actual original assertions pass', 'foreign-glob private completion remains sealed', 'foreign-glob denial inventory is exact after caught calls', 'glob symlink cannot expose a private adapter even as a denied foreign key', 'glob target native imports remain denied when selected as the actual entry']

DEFAULT_MODES = ['seed','hit','refresh','mixed','rollback','rollback-native-failure']
DEFAULT_FILES = ['packages/convex-helpers/index.test.ts','packages/convex-helpers/browser.test.ts',ORDINARY]
DEFAULT_SOURCE_SHA = ['06e78a6fd8e397384c1c2d8b1e38939755882d8ba0d6017d5946cdb468564b2f',
 '7ddc21e36493d08c848bea3fd195f8766df0ab5bed397083049d1832bede73e9',SOURCE_SHA[1]]
DEFAULT_COMMON = ['original three-file scope retains two candidates and one ordinary file',
 'native selection completes once and all actual file identities remain visible','full logical artifact binds exact bytes and final exit',
 'ordinary test executes exactly once','every lookup finishes before records/publication']
DEFAULT_PASS = ['cold members use exactly one closed compiler batch','workers remain fresh serial operations in original instance order',
 'only complete admitted replays omit their ordinary counterpart','all successful logical states remain passing',
 'publication callback retains the original final-exit contract']
DEFAULT_ROLLBACK = ['real unsupported native import declines the actual batch before any candidate worker',
 'one previously valid proof/receipt stays provisional and is discarded',
 'one complete real native fallback executes all original specs with no hit omission',
 'batch-admission decline cannot record or publish even after native pass','native exit remains authoritative without failure erasure',
 'failed reservation poisons only this disposable process']
DEFAULT_CHILD = {mode:DEFAULT_COMMON + (DEFAULT_ROLLBACK if mode.startswith('rollback') else DEFAULT_PASS) +
 (['conservative certificate miss retains full byte-ledger hit and refresh rule'] if mode=='refresh' else []) +
 (['one full-source miss cannot suppress another current authenticated hit'] if mode=='mixed' else []) for mode in DEFAULT_MODES}
DEFAULT_PARENT = ['all three reviewed original source files are present before any child']
for mode in DEFAULT_MODES:
    DEFAULT_PARENT += [mode+': fresh process preserves every own integration check',
      'seed records two exact complete receipts after actual managed/native passes' if mode=='seed' else
      'native failure is retained in the exact changed ordinary control row' if mode=='rollback-native-failure' else
      mode+': all logical project/name/states and API file states equal uncached control',
      mode+': source edits are restored before any other disposable process']
assert len(DEFAULT_PARENT)==19 and sum(len(row) for row in DEFAULT_CHILD.values())==64


def audit_controls(P,root,baseline_root,baseline,sources):
    assert raw(root/'source-pin.txt').decode().strip()==SOURCE
    legacy=read(root/'legacy-ledger/result.json');inventory(legacy,LEGACY_CHECKS,'passed',False)
    assert len(LEGACY_CHECKS)==152 and legacy['actualSdkAuthenticationClaimed'] is False
    roster_text=git_bytes(REPOSITORY,SOURCE,'scripts/bench-vitest/managed/matrix-candidates.mjs').decode()
    roster=P.decode(roster_text.split('Object.freeze(',1)[1].split('.map(row => Object.freeze(row))',1)[0])
    ids=set(P.instance_map(roster));assert len(ids)==108 and TARGET['instanceId'] in ids
    P.audit_legacy(root/'legacy-ledger',legacy,baseline_root,baseline,sources,ids)
    fixture=read(root/'dispatcher/result.json');inventory(fixture,DISPATCH_CHECKS,'passed',False)
    assert len(DISPATCH_CHECKS)==142 and fixture['checkCount']==142 and fixture['caseCount']==13 and fixture['originalSourcesRestored'] is True
    proof=read(root/'dispatcher/input-proof-controls.json');inventory(proof,PROOF_CHECKS,'pass',False)
    assert len(PROOF_CHECKS)==18 and fixture['inputEnvelopeControls']==proof
    P.audit_fixtures(root/'dispatcher',fixture)
    compiler=read(root/'compiler/result.json')
    assert compiler['checks']==[{'name':name,'passed':True} for name in COMPILER_CHECKS]
    assert len(set(COMPILER_CHECKS))==55
    assert compiler['schema']=='quod-managed-compiler-regressions-v1'
    assert compiler['originalCheckCount']==40 and compiler['selfGlobCheckCount']==15 and len(COMPILER_CHECKS)==55
    for name in ['self-glob','foreign-glob']:
        built=read(root/'compiler'/(name+'.json')); bundle=raw(root/'compiler'/(name+'.mjs'))
        assert built['sha256']==sha(bundle)==built['identity']['bundleSha256']
        assert built['identitySha256']==sha(canonical(built['identity']).encode()) and built['identity']['compilerSha256']==COMPILER
        worker=read(root/'compiler'/('runtime-'+name)/'response.json')
        assert worker['ok'] is True and len(worker['rows'])==(3 if name=='self-glob' else 1)
        assert all(row['state']=='pass' for row in worker['rows'])
        assert sorted(worker['boundary']['unsupported'])==([] if name=='self-glob' else ['lazy module ./foreign-modules/native.mjs','lazy module ./foreign-modules/value.mjs'])
        # Validate every remaining private completion field without concealing
        # the intentionally caught denial inventory in this negative fixture.
        copied=copy.deepcopy(worker);copied['boundary']['unsupported']=[];P.boundary(copied)
    return {'ledgerChecks':152,'dispatcherCases':13,'dispatcherChecks':142,'proofChecks':18,'compilerChecks':55,
            'namedInventoriesExact':True,'rawLegacy108ReceiptsAnd13CasesChecked':True}


def audit_default(P,root,sources):
    parent=read(root/'result.json');inventory(parent,DEFAULT_PARENT)
    assert parent['caseCount']==6 and parent['originalSourcesRestored'] is True
    assert [row['mode'] for row in parent['cases']]==DEFAULT_MODES
    assert all(row['passed'] is True and row['status']==0 and row['signal'] is None and row['error'] is None for row in parent['cases'])
    seed_rows=seed_files=None;responses={};certs={};summaries={}
    for mode in DEFAULT_MODES:
        directory=root/mode;child=read(directory/'result.json');inventory(child,DEFAULT_CHILD[mode])
        assert child['mode']==mode and child['actualSdkAuthenticationClaimed'] is False and child['originalSourcesRestored'] is True
        assert [row['file'] for row in child['sources']]==DEFAULT_FILES
        assert [row['originalSha256'] for row in child['sources']]==DEFAULT_SOURCE_SHA
        for i,row in enumerate(child['sources']):assert row['currentSha256']==sha(raw(directory/'sources'/(str(i)+'.test.ts')))
        out=directory/'dispatch';d=read(out/'result.json');assert d==child['dispatch']
        assert d['compilationPolicy']==BATCH_POLICY and d['selectedInstances']==3 and d['candidateInstances']==2 and d['fullJobHit'] is False
        failed=mode=='rollback-native-failure';n=P.native(out/'native',passing=not failed)
        assert len(n['originalSelected'])==3 and {row['file'] for row in n['originalSelected']}==set(DEFAULT_FILES)
        value,rows,files=P.logical(out,d);assert sum(rows.values())==19 and sum(files.values())==3
        workers,replays,native_count,skipped,recorded,refreshed={
         'seed':(2,0,3,0,2,0),'hit':(0,2,1,2,0,0),'refresh':(0,2,1,1,1,1),'mixed':(1,1,2,1,1,0),
         'rollback':(0,0,3,0,0,0),'rollback-native-failure':(0,0,3,0,0,0)}[mode]
        for key,expected in [('managedExecutions',workers),('replayedInstances',replays),('nativeSpecsExecuted',native_count),
          ('skippedCompilations',skipped),('recordedReceipts',recorded),('refreshedCandidates',refreshed)]:assert d[key]==expected
        actuals=P.instance_map(n['executed']);originals=P.instance_map(n['originalSelected']);assert set(P.instance_map(d['instances']))==set(originals)
        expected_rows=collections.Counter();expected_files=collections.Counter()
        for row in d['instances']:
            iid=row['instanceId'];assert row['project']=='convex-helpers' and row['pool']=='forks'
            assert {key:row[key] for key in ['instanceId','project','file','pool']}=={key:originals[iid][key] for key in ['instanceId','project','file','pool']}
            assert row['candidate']==(row['file'] in DEFAULT_FILES[:2])
            if row['candidate']:assert row['evidenceDirectory']=='instances/'+iid
            if row.get('workerEvidenceDirectory'):assert row['workerEvidenceDirectory']=='managed-'+iid
            if row['decision']=='replay':
                evidence=out/row['evidenceDirectory'];body=raw(evidence/'replayed-response.json')
                assert body==responses[row['file']];data=P.decode(body);P.boundary(data)
                tests=data['rows'];name,state=data['files'][0]['name'],data['files'][0]['state']
                assert iid not in actuals
            else:
                actual=actuals[iid];assert row['nativeExecuted'] is True
                tests=actual['rows'];name,state=actual['relativeModuleId'],actual['state']
            expected_rows.update((row['project'],row['file'],row['pool'],test['name'],test['state']) for test in tests)
            expected_files[(iid,row['project'],row['file'],row['pool'],name,state)]+=1
            if row.get('workerEvidenceDirectory'):
                evidence=out/row['evidenceDirectory'];phase=out/row['workerEvidenceDirectory']
                built=read(evidence/'test-build.json');body=raw(phase/'response.json');P.boundary(P.decode(body))
                assert sha(raw(phase/'worker.mjs'))==built['sha256']==built['identity']['bundleSha256']
                execution=read(evidence/'execution-identity.json');check_execution(P,execution,row,sources,False)
                P.certificate(read(evidence/'compile-certificate.json'),execution['identity'],sources)
                if mode=='seed':responses[row['file']]=body;certs[row['file']]=raw(evidence/'compile-certificate.json')
        assert rows==expected_rows and files==expected_files
        if mode=='seed':seed_rows,seed_files=rows,files
        elif not failed:assert rows==seed_rows and files==seed_files
        else:
            assert [row for row in rows.elements() if row[4]=='fail']==[('convex-helpers',ORDINARY,'forks','QUOD batch control 2','fail')]
            assert sum(count for row,count in files.items() if row[-1]=='fail')==1
        batch=d['compilationBatch']
        if mode=='hit':assert batch=={'attempted':False,'requests':0,'completed':False,'seconds':0} and not (out/'batch-compilation.json').exists()
        else:
            assert read(out/'batch-compilation.json')==batch and batch['attempted'] is True
            count=2 if mode=='seed' else 1;trace=P.batch_trace(count);assert batch['requests']==count
            if mode.startswith('rollback'):
                assert batch['completed'] is False and batch['fenceTrace']==trace[:2]
                assert 'only a named AsyncLocalStorage import is supported: node:fs' in batch['error']['message']
            else:assert batch['completed'] is True and batch['diagnostics']['fullFences']==len(trace) and batch['diagnostics']['trace']==trace
        assert len([phase for phase in d['phases'] if phase['name'].startswith('managed-')])==workers
        assert len(list(out.glob('managed-*')))==workers
        assert [phase['name'] for phase in d['phases'] if phase['name'].startswith('managed-')]==[row['workerEvidenceDirectory'] for row in d['instances'] if row.get('workerEvidenceDirectory')]
        assert [row['operation'] for row in child['callbacks']]==['lookup','lookup']+['record']*recorded+([] if mode.startswith('rollback') else ['publish'])
        assert d['exitCode']==n['exitCode']==(1 if failed else 0) and d['stickyManagedFailure'] is False and d['observedNativeFailure'] is failed
        if mode.startswith('rollback'):
            assert d['admissionDeclined'] and n['omittedAsHits']==[] and not (out/'receipts').exists()
            assert not list(out.rglob('replayed-response.json')) and d['publication']['saved'] is False
            assert all(row['decision']=='ordinary' and not row.get('compilationSkipped') and not row.get('receiptSha256') for row in d['instances'])
        elif mode=='refresh':
            row=next(row for row in d['instances'] if row['file']==DEFAULT_FILES[1])
            assert raw(out/row['evidenceDirectory']/'compile-certificate.json')==certs[row['file']]
            assert row['inputMissReason']=='current full source bytes differ'
        summaries[mode]={'checks':len(DEFAULT_CHILD[mode]),'workers':workers,'replays':replays,'native':native_count,'exitCode':d['exitCode']}
    return {'parentChecks':19,'childChecks':[len(DEFAULT_CHILD[mode]) for mode in DEFAULT_MODES],'cases':summaries}


def current_proof(P,directory,decision,cert,identity):
    assert decision['hit'] is True and 'proof' not in decision and 'identity' not in decision
    assert decision['identitySha256']==sha(canonical(identity['compile']).encode())
    assert decision['bundleSha256']==identity['compile']['bundleSha256']
    assert decision['authority']=='input equivalence only; authenticated passing receipt admission still required'
    requests=P.resolution_requests([cert]);tools=cert['tools'];geometry=cert['rootGeometry']
    body={'project':geometry['project'],'adapterRoot':geometry['adapter'],'requests':requests,'expectedToolsSha256':sha(canonical(tools).encode())}
    assert decision['uniqueResolverRequests']==len(requests)
    assert decision['resolver']=={'requestSha256':sha((canonical(body)+'\n').encode()),
        'toolIdentitySha256':sha(canonical(tools).encode()),'node':'v24.18.0','execArgv':[],
        'profile':'one new plain Node process and one new resolver-only esbuild build'}


def audit_property(P,root,baseline,sources,project_repository):
    parent=read(root/'result.json');inventory(parent,PARENT_CHECKS)
    assert parent['schema']=='quod-managed-property47-profile-probe-v1-research'
    assert parent['caseCount']==6 and parent['originalSourcesRestored'] is True
    assert parent['actualSdkAuthenticationClaimed'] is False and parent['originalDispatcherChecksModified'] is False
    assert parent['parentInventory']==PARENT_CHECKS and parent['caseInventories']==CHILD_CHECKS
    assert [row['mode'] for row in parent['cases']]==MODES
    assert all(row['passed'] is True and row['status']==0 and row['signal'] is None and row['error'] is None for row in parent['cases'])
    originals=[git_bytes(project_repository,PROJECT_COMMIT,file) for file in SOURCE_PATHS]
    assert [sha(body) for body in originals]==SOURCE_SHA
    original_map=P.instance_map(baseline['executed']);original=original_map[TARGET['instanceId']]
    assert {key:original[key] for key in TARGET}==TARGET and len(original['rows'])==47
    baseline_rows=P.state_rows(baseline['executed']);baseline_files=P.file_rows(baseline['executed'])
    target_rows=P.state_rows([original]);target_files=P.file_rows([original])
    seed_execution=seed_certificate=seed_response=seed_receipt=seed_envelope=seed_options=seed_native_root=None
    summary={}
    for mode in MODES:
        directory=root/mode;out=directory/'dispatch';child=read(directory/'result.json');inventory(child,CHILD_CHECKS[mode])
        assert child['schema']=='quod-managed-property47-profile-case-v1-research' and child['mode']==mode
        assert child['actualSdkAuthenticationClaimed'] is False and child['originalSourcesRestored'] is True
        rollback=mode.startswith('rollback');failed=mode=='rollback-native-failure';hit=mode in ['hit','refresh','mixed-routing']
        assert [row['file'] for row in child['sources']]==SOURCE_PATHS
        for index,row in enumerate(child['sources']):
            body=raw(directory/'sources'/(str(index)+'.test.ts'))
            expected=originals[index]
            if index==0 and rollback:
                expected+=b'\nimport {readFileSync as quodProperty47Read} from "node:fs";\nif (typeof quodProperty47Read !== "function") throw new Error("QUOD property native import unavailable");\n'
            if index==1 and failed:
                expected+=('\nimport {test as quodProperty47Test,expect as quodProperty47Expect} from "vitest";\nquodProperty47Test('+json.dumps(SENTINEL,separators=(',',':'))+',()=>quodProperty47Expect(false).toBe(true));\n').encode()
            assert body==expected and row['currentSha256']==sha(body) and row['originalSha256']==SOURCE_SHA[index]
        d=read(out/'result.json');assert d==child['dispatch']
        assert d['schema']=='quod-managed-matrix-input-dispatch-v1-research' and d['productionEnabled'] is False
        assert d['fullJobHit'] is False and d['selectedInstances']==120 and d['candidateInstances']==1 and d['initialOrdinaryInstances']==119
        assert d['selectedScope']=={'kind':'complete-original-default-selection','files':None}
        assert d['candidatePolicy']==PROFILE_LABEL and d['candidateProfile']==profile_value()
        assert d['supervisorPolicy']['candidateProfile']==profile_value()
        assert d['supervisorPolicy']['candidateProfileSelectorSha256']==sources['matrix-candidate-profiles.mjs']
        assert d['compilationPolicy']==BATCH_POLICY and d['supervisorPolicy']['compilationPolicy']==BATCH_POLICY
        all_instances=P.instance_map(d['instances']);assert set(all_instances)==set(original_map)
        candidates=[row for row in d['instances'] if row['candidate']];assert len(candidates)==1;candidate=candidates[0]
        assert {key:candidate[key] for key in TARGET}==TARGET
        assert candidate['evidenceDirectory']=='instances/'+TARGET['instanceId']
        evidence=out/candidate['evidenceDirectory']
        assert [file.name for file in (out/'instances').iterdir()]==[TARGET['instanceId']]
        n=P.native(out/'native',passing=not failed)
        assert n['unhandledErrors']==[] and n['failedModules']==int(failed)
        assert set(P.instance_map(n['originalSelected']))==set(original_map)
        assert len(n['executed'])==d['nativeSpecsExecuted']==(119 if hit else 120)
        omitted=P.instance_map(n['omittedAsHits']);assert set(omitted)==({TARGET['instanceId']} if hit else set())
        expected_rows=baseline_rows.copy();expected_files=baseline_files.copy()
        if failed:
            expected_rows[(TARGET['project'],ORDINARY,TARGET['pool'],SENTINEL,'fail')]+=1
            changed=[row for row in expected_files if row[2]==ORDINARY and row[1]==TARGET['project']];assert len(changed)==1
            before=changed[0];assert expected_files[before]==1;del expected_files[before]
            expected_files[(*before[:-1],'fail')]+=1
        assert P.state_rows(n['executed'])==(expected_rows-target_rows if hit else expected_rows)
        assert P.file_rows(n['executed'])==(expected_files-target_files if hit else expected_files)
        value,states,files=P.logical(out,d,expected_rows,expected_files)
        assert len(value['files'])==120 and len(value['rows'])==(4132 if failed else 4131)
        assert d['exitCode']==n['exitCode']==(1 if failed else 0)
        assert d['observedNativeFailure'] is failed and d['stickyManagedFailure'] is False
        preflight=read(out/'preflight/response.json');P.boundary(preflight,preflight=True)
        assert len(preflight['checks'])==31 and len({row['name'] for row in preflight['checks']})==31 and all(row['pass'] is True for row in preflight['checks'])
        prebuilt=read(out/'preflight-build.json');assert prebuilt['identitySha256']==sha(canonical(prebuilt['identity']).encode())
        assert prebuilt['options']=={'epoch':1700000000000,'seed':42,'env':{'NODE_ENV':'test'}}
        fingerprint=sha(canonical({'preflightIdentity':prebuilt['identitySha256'],'inventory':preflight['boundary']['inventory'],
            'symbols':preflight['boundary']['symbolInventory']}).encode())
        assert fingerprint==d['nativeFingerprintSha256'] and prebuilt['identity']['compilerSha256']==COMPILER
        phases={row['name']:row for row in d['phases']};assert len(phases)==len(d['phases'])
        invocation=read(out/'preflight/invocation.json')
        P.worker_evidence(out/'preflight',prebuilt['identity'],invocation['runtimePolicy'],d['host'],phases['preflight'])
        batch=d['compilationBatch'];batchfile=out/'batch-compilation.json'
        if mode in ['hit','mixed-routing']:
            assert batch=={'attempted':False,'requests':0,'completed':False,'seconds':0} and not batchfile.exists()
        else:
            assert read(batchfile)==batch and batch['attempted'] is True and batch['requests']==1
            if rollback:
                assert batch['completed'] is False and 0<len(batch['fenceTrace'])<4 and batch['fenceTrace']==P.batch_trace(1)[:len(batch['fenceTrace'])]
                assert 'only a named AsyncLocalStorage import is supported: node:fs' in batch['error']['message']
            else:
                diagnostics=batch['diagnostics'];assert batch['completed'] is True and diagnostics['requests']==1
                assert diagnostics['fullFences']==4 and diagnostics['trace']==P.batch_trace(1) and diagnostics['publicOperationsAtReservation']==0
                assert diagnostics['authority']=='actual compilation and capture only; no response or receipt admission'
        workers=1 if mode=='seed' else 0;records=1 if mode in ['seed','refresh'] else 0
        expected_counts={'managedExecutions':workers,'replayedInstances':int(hit),'skippedCompilations':int(mode in ['hit','mixed-routing']),
            'recordedReceipts':records,'refreshedCandidates':int(mode=='refresh'),'inputCandidates':int(mode!='seed'),
            'nativeConformanceInstances':workers}
        for key,count in expected_counts.items():assert d[key]==count
        assert len(list(out.glob('managed-*')))==workers and len([row for row in phases if row.startswith('managed-')])==workers
        expected_artifacts={'builds':int(mode in ['seed','refresh']),'certificates':int(mode in ['seed','refresh']),'workerDirectories':workers}
        assert child['artifactCounts']==expected_artifacts
        assert int((evidence/'test-build.json').is_file())==expected_artifacts['builds']
        assert int((evidence/'compile-certificate.json').is_file())==expected_artifacts['certificates']
        for name in ['test-build.json','compile-certificate.json']:
            expected_paths={candidate['evidenceDirectory']+'/'+name} if mode in ['seed','refresh'] else set()
            assert {file.relative_to(out).as_posix() for file in out.rglob(name)}==expected_paths
        assert [row['operation'] for row in child['callbacks']]==['lookup']+['record']*records+([] if rollback else ['publish'])
        assert all(row.get('candidateKey')==candidate['candidateKey'] for row in child['callbacks'] if row['operation']!='publish')
        assert len(child['recorded'])==records
        if rollback:
            assert d['admissionDeclined'] and candidate['decision']=='ordinary'
            assert not candidate.get('compilationSkipped') and not candidate.get('receiptSha256') and not candidate.get('workerEvidenceDirectory')
            assert not list(out.rglob('replayed-response.json')) and not (out/'receipts').exists()
            assert not (evidence/'execution-identity.json').exists() and not (evidence/'recorded-candidate.json').exists()
            assert read(evidence/'input-decision.json')['hit'] is False
            assert raw(evidence/'candidate-envelope.json',P.MAX_ITEM)==seed_envelope
            assert d['publication']=={'saved':False,'reason':'research admission declined before complete candidate execution'}
            assert all(row['nativeExecuted'] is True for row in d['instances'])
        else:
            assert not d.get('admissionDeclined') and d['publication']=={'saved':False,'reason':'synthetic-profile-diagnostic-no-SDK'}
            assert child['callbacks'][-1]=={'operation':'publish','exitCode':0}
            execution=read(evidence/'execution-identity.json');identity=check_execution(P,execution,candidate,sources)
            assert identity['host']==d['host'] and identity['supervisorPolicy']==d['supervisorPolicy'] and identity['nativePolicy']==d['nativePolicy']
            assert identity['runtimePolicy']['nativeFingerprintSha256']==fingerprint
            assert read(evidence/'preparation.json')==identity['preparation']
            if mode=='seed':
                seed_options=read(evidence/'test-build.json')['options']
                assert seed_options['epoch']==1700000000000 and seed_options['seed']==42 and seed_options['env']=={'NODE_ENV':'test'}
            route={'preparation':identity['preparation'],'options':seed_options,'runtimePolicy':identity['runtimePolicy'],
                'supervisorPolicy':identity['supervisorPolicy'],'host':identity['host'],'nativePolicy':identity['nativePolicy']}
            assert candidate['candidateKey']==P.domain(P.ENVELOPE,route)
            assert sha(canonical(seed_options).encode())==identity['compile']['configurationSha256']
            assert seed_options['file']=='/project/'+TARGET['file'] and seed_options['providedContext']==identity['preparation']['providedContext']
            if mode in ['seed','refresh']:
                certbytes=raw(evidence/'compile-certificate.json',P.MAX_ITEM);cert=P.decode(certbytes)
                assert certbytes==(canonical(cert)+'\n').encode();P.certificate(cert,identity,sources)
                built=read(evidence/'test-build.json');assert built['identity']==identity['compile'] and built['preparation']==identity['preparation'] and built['options']==seed_options
                assert built['identitySha256']==sha(canonical(built['identity']).encode()) and built['sha256']==identity['compile']['bundleSha256']
                assert batch['diagnostics']['totalCertificates']==len(certbytes) and batch['diagnostics']['totalIdentities']==len(canonical(identity['compile']).encode())
                if mode=='seed':
                    assert candidate['workerEvidenceDirectory']=='managed-'+TARGET['instanceId']
                    phase=out/candidate['workerEvidenceDirectory'];response=P.worker_evidence(phase,identity['compile'],identity['runtimePolicy'],identity['host'],phases[phase.name])
                    assert batch['diagnostics']['totalBundles']==len(raw(phase/'worker.mjs')) and sha(response)==candidate['responseSha256']
                    P.boundary(P.decode(response));assert candidate['comparison']['equal'] is True and candidate['comparison']['managedCount']==47 and candidate['fileMatches'] is True
                    seed_execution=execution;seed_certificate=certbytes;seed_response=response;seed_native_root=out/'native'
                    seed_receipt=raw(out/'receipts'/(TARGET['instanceId']+'.json'),P.MAX_ITEM)
                    receipt(P,seed_receipt,execution,original,seed_native_root)
                else:
                    assert execution==seed_execution and certbytes==seed_certificate
                    assert candidate['inputMissReason']=='current full source bytes differ'
                    decision=read(evidence/'input-decision.json');assert decision['hit'] is False and decision['reason']==candidate['inputMissReason']
            if hit:
                assert execution==seed_execution and candidate['decision']=='replay'
                assert raw(evidence/'replayed-response.json')==seed_response
                assert not candidate.get('workerEvidenceDirectory')
                assert candidate['receiptSha256']==sha(seed_receipt)
                envelopebytes=raw(evidence/'candidate-envelope.json',P.MAX_ITEM)
                expected_envelope=raw(directory/'synthetic-refresh-envelope.json',P.MAX_ITEM) if mode=='refresh' else seed_envelope
                assert envelopebytes==expected_envelope
                envelope,cert,certbytes,oldreceipt,oldreceiptbytes=P.unpack_envelope(envelopebytes,candidate['candidateKey'],sha(envelopebytes))
                assert oldreceiptbytes==seed_receipt and oldreceipt['execution']==execution
                assert read(evidence/'candidate-authentication.json')=={'candidateKey':candidate['candidateKey'],
                    'trustedEnvelopeSha256':sha(envelopebytes),'trustedCertificateSha256':sha(certbytes),'trustedReceiptSha256':sha(oldreceiptbytes)}
                if mode=='refresh':
                    expected=P.decode(seed_certificate);expected['sourceFiles'][0]['rawSha256']='0'*64
                    assert cert==expected
                else:
                    assert certbytes==seed_certificate;P.certificate(cert,identity,sources)
                    current_proof(P,evidence,read(evidence/'input-decision.json'),cert,identity)
            if records:
                recorded=child['recorded'][0];body=raw(evidence/'recorded-candidate.json',P.MAX_ITEM)
                assert recorded['candidateKey']==candidate['candidateKey'] and recorded['envelopeSha256']==sha(body)==candidate['envelopeSha256']
                assert recorded['executionKey']==execution['key'] and recorded['testFile']==TARGET['file'] and pathlib.PurePosixPath(recorded['file']).name=='recorded-candidate.json'
                _,cert,certbytes,value,receiptbytes=P.unpack_envelope(body,candidate['candidateKey'],sha(body))
                assert certbytes==seed_certificate and receiptbytes==seed_receipt and value['execution']==execution
                if mode=='seed':seed_envelope=body
                else:assert body==seed_envelope
                assert candidate['envelopeBytes']==len(body)
        if mode=='mixed-routing':
            request=read(root/(mode+'-request.json'));assert len(request['authority'])==2
            assert request['authority'][1]['candidateKey']=='0'*64 and request['authority'][1]['historicalRouteOnly'] is True
            assert len([row for row in child['callbacks'] if row['operation']=='lookup'])==1
        elif mode in ['hit','refresh']:
            request=read(root/(mode+'-request.json'));assert len(request['authority'])==1 and request['authority'][0]['candidateKey']==candidate['candidateKey']
        summary[mode]={'checks':len(CHILD_CHECKS[mode]),'nativeInstances':len(n['executed']),'states':sum(states.values()),
            'managedWorkers':workers,'replays':int(hit),'recorded':records,'exitCode':d['exitCode'],
            'sourcesRestored':True,'batchFences':0 if mode in ['hit','mixed-routing'] else len(batch.get('fenceTrace',[])) if rollback else 4}
    return {'parentChecks':28,'childChecks':[len(CHILD_CHECKS[mode]) for mode in MODES],'cases':summary,
        'original47RowsAndFileStateExact':True,'currentCertificateProofAndWholeReceiptLinked':True,
        'actualSdkAuthenticationClaimed':False,'fullJobHit':False}


def embedded_inventory(P,root,provenance):
    value=read(root/'inventory.json',1024*1024)
    assert value['maximumPaths']==20000 and value['maximumExpandedBytes']==500000000 and value['maximumFileBytes']==33554432
    assert value['inventoryReservedPaths']==1 and value['inventoryReservedBytes']==1024*1024
    assert value['pathsBeforeInventory']+1==provenance['filesByteVerified']+provenance['directoriesVerified']+1
    rows={row['name']:row for row in value['files']};assert len(rows)==len(value['files'])
    assert set(rows)==set(provenance['files'])-{'inventory.json'}
    for name,row in rows.items():assert row['bytes']==provenance['files'][name]['bytes'] and row['sha256']==provenance['files'][name]['sha256']


def audit(index,primitives,repository,project_repository):
    P=primitives
    exact(index,['schema','runJson','jobsJson','artifactsJson','baselineRunJson','baselineArtifactsJson','evidence'])
    assert index['schema']=='quod-property47-data-audit-inputs-v1' and set(index['evidence'])==set(ROLES+['baseline'])
    run,jobs,artifacts=read(index['runJson'],MAX_API),read(index['jobsJson'],MAX_API),read(index['artifactsJson'],MAX_API)
    assert run['id']==RUN and run['head_sha']==SOURCE and run['status']=='completed' and run['conclusion']=='success'
    assert run['run_attempt']==1 and run['event']=='workflow_dispatch' and run['path']=='.github/workflows/gate.yml'
    assert jobs['total_count']==len(jobs['jobs']) and artifacts['total_count']==len(artifacts['artifacts'])
    mapped_jobs={row['id']:row for row in jobs['jobs']};mapped_artifacts={row['id']:row for row in artifacts['artifacts']}
    assert len(mapped_jobs)==len(jobs['jobs']) and len(mapped_artifacts)==len(artifacts['artifacts'])==3
    job=mapped_jobs[JOB[0]];assert job['run_id']==RUN and job['name']==JOB[1]
    assert job['status']=='completed' and job['conclusion']=='success' and (job['started_at'],job['completed_at'])==JOB[2:]
    assert all(row['conclusion']=='skipped' or row['conclusion']=='success' and row['name'] in [JOB[1],'changes'] for row in jobs['jobs'])
    start,end=(datetime.datetime.fromisoformat(value.replace('Z','+00:00')) for value in JOB[2:]);seconds=(end-start).total_seconds();assert seconds>0
    roots,provenance={},{}
    for role in ROLES+['baseline']:
        pin=BASELINE if role=='baseline' else ARTIFACTS[role][:3]
        profile=ARCHIVE_PROFILES[role]
        P.MAX_ARCHIVE=profile['compressedBytes'];P.MAX_EXPANDED=profile['expandedBytes'];P.MAX_FILE=profile['fileBytes'];P.MAX_PATHS=profile['paths']
        roots[role],provenance[role]=P.archive(role,index['evidence'][role],pin)
        if role=='baseline':continue
        meta=mapped_artifacts[pin[0]]
        assert meta['name']==ARTIFACT_NAMES[role] and meta['size_in_bytes']==pin[1] and meta['digest']=='sha256:'+pin[2] and meta['expired'] is False
        assert meta['workflow_run']['id']==RUN and meta['workflow_run']['head_sha']==SOURCE
        embedded_inventory(P,roots[role],provenance[role])
    oldrun=read(index['baselineRunJson'],MAX_API);oldmeta=read(index['baselineArtifactsJson'],MAX_API)
    assert oldrun['id']==BASELINE_RUN and oldrun['head_sha']==BASELINE_SOURCE and oldrun['status']=='completed' and oldrun['conclusion']=='success'
    old=[row for row in oldmeta['artifacts'] if row['id']==BASELINE[0]];assert len(old)==1;old=old[0]
    assert old['size_in_bytes']==BASELINE[1] and old['digest']=='sha256:'+BASELINE[2] and old['name']=='managed-matrix'
    assert old['workflow_run']['id']==BASELINE_RUN and old['workflow_run']['head_sha']==BASELINE_SOURCE
    sources={}
    for name in SOURCE_NAMES:
        file=name if name=='.github/workflows/gate.yml' else 'scripts/bench-vitest/managed/'+name
        sources[name]=sha(git_bytes(repository,SOURCE,file))
    assert sources['compiler.mjs']==COMPILER and sources['resolved-compile-certificate.mjs']==CORE
    assert all(sources[name]==expected for name,expected in FROZEN_FILES.items())
    for name in SOURCE_NAMES:
        if name in FROZEN_FILES:continue
        file='scripts/bench-vitest/managed/'+name
        assert git_bytes(repository,SOURCE,file)==git_bytes(repository,'85c96d6c62c9252f3aa464a851f514348353d010',file), 'unreviewed pre-existing runtime/fixture change'
    for file in ['source-sha256.txt','profile-source-sha256.txt']:
        for line in raw(roots['controls']/file).decode().splitlines():
            digest,name=line.split();name=name.removeprefix('scripts/bench-vitest/managed/')
            assert name in sources and digest==sources[name]
    baseline_root=P.locate(roots['baseline'],['managed-matrix','native-selection.json']).parent
    baseline=P.native(baseline_root,baseline=True)
    assert len(baseline['executed'])==120 and sum(P.state_rows(baseline['executed']).values())==4131
    assert collections.Counter(module['state'] for module in baseline['executed'])=={'pass':119,'skip':1}
    controls=audit_controls(P,roots['controls'],baseline_root,baseline,sources)
    default=audit_default(P,roots['default-integration'],sources)
    property_result=audit_property(P,roots['profile'],baseline,sources,project_repository)
    # Rebind all bytes and exact directory sets after every read. These are
    # sampled nonadversarial stability checks, not an atomic filesystem claim.
    for role in ROLES+['baseline']:
        pin=BASELINE if role=='baseline' else ARTIFACTS[role][:3]
        _,after=P.archive(role,index['evidence'][role],pin)
        assert after==provenance[role], 'evidence changed during audit'
    return {'schema':'quod-property47-independent-data-audit-v1','passed':True,'issues':[],
      'run':RUN,'sourcePin':SOURCE,'trustedReaderSha256':PRIMITIVE_READER_SHA256,'readerSha256':sha(raw(pathlib.Path(__file__),256*1024)),
      'job':{'id':JOB[0],'startedAt':JOB[2],'completedAt':JOB[3],'completeJobSeconds':seconds,'modeledRoundedMinutes':math.ceil(seconds/60)},
      'sources':sources,'provenance':provenance,'originalControls':controls,'defaultIntegration':default,'propertyProfile':property_result,
      'claims':{'actualSDKAuthentication':False,'fullJobHit':False,'windowOrSpeedupClaim':False},
      'limitations':['synthetic envelope authority in a finite profile; no SDK cache claim',
        'full original selection, assertion states and API file states only; no native reporter/timing equivalence claim',
        'current opaque proof issuance is reviewed pinned TCB; retained data does not independently rerun resolution',
        'sampled nonadversarial data stability; no atomic adversarial filesystem snapshot claim']}


REPOSITORY=None

def main():
    global REPOSITORY
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repository',required=True,type=pathlib.Path)
    parser.add_argument('--project-repository',required=True,type=pathlib.Path)
    parser.add_argument('--trusted-reader',required=True,type=pathlib.Path)
    parser.add_argument('index',type=pathlib.Path)
    args=parser.parse_args()
    try:
        checked_pins()  # Must precede every index/helper/repository/payload read.
        index=read(args.index,1024*1024);parent=args.index.absolute().parent
        for key in ['runJson','jobsJson','artifactsJson','baselineRunJson','baselineArtifactsJson']:
            index[key]=str(parent/index[key])
        for descriptor in index['evidence'].values():
            exact(descriptor,['root','archive'])
            for key in ['root','archive']:descriptor[key]=str(parent/descriptor[key])
        roots=[value['root'] for value in index['evidence'].values()]
        payloads=[args.index,*[index[key] for key in ['runJson','jobsJson','artifactsJson','baselineRunJson','baselineArtifactsJson']],
            *[value['archive'] for value in index['evidence'].values()]]
        P=load_primitives(args.trusted_reader,roots,payloads)
        REPOSITORY=args.repository.resolve(strict=True)
        result=audit(index,P,REPOSITORY,args.project_repository.resolve(strict=True))
    except Exception as error:
        result={'schema':'quod-property47-independent-data-audit-v1','passed':False,'issues':[type(error).__name__+': '+str(error)]}
    print(json.dumps(result,indent=2,ensure_ascii=False,allow_nan=False))
    return 0 if result['passed'] else 1


if __name__=='__main__':
    sys.exit(main())

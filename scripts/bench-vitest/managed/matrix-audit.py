#!/usr/bin/env python3
"""Data-only audit of a SHA-verified original-c11 matrix experiment.

No artifact source is imported or executed. SOURCE_PIN is an explicit caller
authority and is resolved only in this trusted checkout. Archive provenance is
optional for inspecting a partial run, mandatory for a publishable passed audit.
"""
import argparse
import collections
import hashlib
import json
import pathlib
import re
import subprocess
import sys
import zipfile

CONTRACT = 'quod-managed-v0-protectors-frozen-slots-investigation'
POLICY = 'quod-managed-frozen-array-promise-slots-v1-probe'
# Independently retained full c11 native report; includes repeated projects.
ORIGINAL_STATES_SHA = '171d492ae86006d671f06795a634ed255c87fe5795a8f50f5d542b20054188cb'
ORIGINAL_ROSTER_SHA = 'ffabb977ddbd211cc047bcd9d09d7138b965d3fc3cb60e820a192c780b87220b'
ORIGINAL_PROJECT_STATES_SHA = 'eb7962277f74c63e25c070d2c002182c37841acc031a82a8cff505adae9d8d3a'
ORIGINAL_47_SOURCE_SHA = '9798bab303233bca846a89ccb39ce28bd968329e0704f3e5d67adca934c11c75'
ZOD_PROBES = {
    '4614d999e6a5b5a8bbdc0d3033647dbb7ea73f2594394224a145d9c43721c7ba': '85cbd8672064ea19f41dda8eb4565596afb3be3ce10a739fd64ee9b59a41ef22',
    'c9dab88c4e7977e4ef0c536c07ef74515333050088bef04dbe871bb92f4a9a2d': '85cbd8672064ea19f41dda8eb4565596afb3be3ce10a739fd64ee9b59a41ef22',
    '94d18b95a0fe1194758a8426f331f4e37a2d3a733ec64effe513265ecc963378': '85cbd8672064ea19f41dda8eb4565596afb3be3ce10a739fd64ee9b59a41ef22',
    'e41f9e774b96a9fe2b04c08f5aad32e6db2f6aa090671d2a375e7062b4f178aa': '85cbd8672064ea19f41dda8eb4565596afb3be3ce10a739fd64ee9b59a41ef22',
    '968e3433e810cf7ba5efb609a1cb6d9a6cfb51215fd1f699938228f7ac25deb3': '8cd1a4f347c3d874337e22f0c9f9781d9b9d2de264f8345a96679f026f061c7f',
    'e50c63d70e8b1abf2968936acf22c9e4e96e735f0af6d367785e0948e46553fa': '09f707d71b847ffc1c2af9ae9347df9b06a9302ef5cf3a3c5d292cdacb10e9cd',
    'bb08133b11b478c8e715038fff26ebea204d7fd5443fba31fe138b142d202efb': '09f707d71b847ffc1c2af9ae9347df9b06a9302ef5cf3a3c5d292cdacb10e9cd',
}
STATE = {'passed': 'pass', 'failed': 'fail', 'skipped': 'skip', 'pending': 'skip', 'todo': 'todo'}
ROOT = pathlib.Path(__file__).resolve().parents[3]


def sha(data):
    return hashlib.sha256(data).hexdigest()


def encoded(data, canonical=False):
    return json.dumps(data, sort_keys=canonical, separators=(',', ':'), ensure_ascii=False, allow_nan=False).encode()


def unique_pairs(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError('duplicate JSON key: ' + key)
        result[key] = value
    return result


def load(file):
    if file.is_symlink() or not file.is_file() or file.stat().st_size > 32 * 1024 * 1024:
        raise ValueError('unsafe or oversized JSON file: ' + str(file))
    return json.loads(file.read_text(), object_pairs_hook=unique_pairs,
                      parse_constant=lambda value: (_ for _ in ()).throw(ValueError(value)))


def relative(file):
    positions = [file.index(prefix) for prefix in ('packages/', 'example-ui/') if prefix in file]
    if not positions:
        raise ValueError('unreviewed test path: ' + file)
    return file[min(positions):]


def key(row):
    return row['project'], row['file'], row['pool']


def rows_hash(rows):
    return sha(encoded(sorted(rows)))


def state_keys(rows):
    # Match JS Array.sort() over serialized keys, including duplicate names.
    return sorted([encoded([row['name'], row['state']]).decode() for row in rows],
                  key=lambda value: value.encode('utf-16-be'))


def audit(args):
    root = pathlib.Path(args.root).resolve()
    if (root / '_temp/managed-matrix/result.json').is_file():
        matrix = root / '_temp/managed-matrix'
    else:
        matrix = root
    if any(path.is_symlink() for path in root.rglob('*')):
        raise ValueError('artifact contains a symlink')
    issues = []
    require = lambda condition, message: issues.append(message) if not condition else None
    require(bool(re.fullmatch('[0-9a-f]{40}', args.source_pin)), 'explicit full source pin required')
    source_sha = {}
    probe_source = b''
    compiler_source = b''
    for name in ('compiler.mjs', 'prelude.mjs', 'runner.mjs', 'preflight.mjs', 'preparation.mjs', 'matrix-planner.mjs', 'matrix-probe.mjs', 'executor.mjs'):
        data = subprocess.check_output(['git', 'show', args.source_pin + ':scripts/bench-vitest/managed/' + name], cwd=ROOT)
        source_sha[name] = sha(data)
        if name == 'matrix-probe.mjs':
            probe_source = data
        if name == 'compiler.mjs':
            compiler_source = data
    self_glob_policy = b'actual-real-test-entry-only; all-other-matches-sticky-denied' in compiler_source
    provenance = False
    if args.archive and args.run_json and args.artifacts_json:
        archive = pathlib.Path(args.archive)
        run = load(pathlib.Path(args.run_json))
        artifacts = load(pathlib.Path(args.artifacts_json))['artifacts']
        candidates = [row for row in artifacts if row['name'] == 'managed-matrix']
        require(len(candidates) == 1, 'one retained matrix artifact required')
        if len(candidates) == 1:
            artifact = candidates[0]
            require(archive.stat().st_size == artifact['size_in_bytes'], 'archive size differs from GitHub metadata')
            require('sha256:' + sha(archive.read_bytes()) == artifact['digest'], 'archive SHA differs from GitHub metadata')
            require(artifact['workflow_run']['id'] == run['id'], 'artifact run identity differs')
            require(artifact['workflow_run']['head_sha'] == args.source_pin == run['head_sha'], 'source pin differs from actual GitHub run')
            require(run['status'] == 'completed' and run['run_attempt'] == 1, 'one completed immutable run required')
            with zipfile.ZipFile(archive) as zipped:
                entries = [entry for entry in zipped.infolist() if not entry.is_dir()]
                require(len(entries) <= 10000 and sum(entry.file_size for entry in entries) <= 500 * 1024 * 1024, 'archive exceeds bounded data inventory')
                names = set()
                for entry in entries:
                    name = pathlib.PurePosixPath(entry.filename)
                    if name.is_absolute() or '..' in name.parts or entry.filename in names or ((entry.external_attr >> 16) & 0o170000) == 0o120000:
                        raise ValueError('unsafe or duplicate archive entry')
                    names.add(entry.filename)
                    extracted = root.joinpath(*name.parts)
                    require(extracted.is_file() and sha(extracted.read_bytes()) == sha(zipped.read(entry)), 'extracted bytes differ from verified archive: ' + entry.filename)
                require(names == {str(file.relative_to(root)) for file in root.rglob('*') if file.is_file()}, 'extracted data inventory differs from archive')
            provenance = True
    result = load(matrix / 'result.json')
    prep = load(matrix / 'preparation.json')
    plan = load(matrix / 'matrix-plan.json')
    native = load(matrix / 'native-selection.json')
    report = load(matrix / 'native-report.json')
    require(result['schema'] == 'quod-managed-matrix-probe-v1-research', 'matrix result schema differs')
    require(all(result.get(name) is False for name in ('productionEnabled', 'cacheEnabled', 'receiptAdmission')), 'investigation must not admit receipts or production caching')
    selected = prep['selected']
    selected_by_id = {row['instanceId']: row for row in selected}
    require(len(selected_by_id) == len(selected) == 120, 'all120 unique actual selected specifications required')
    require(rows_hash([list(key(row)) for row in selected]) == ORIGINAL_ROSTER_SHA, 'original project/file/pool roster differs')
    for row in selected:
        identity = {name: row[name] for name in ('file', 'project', 'pool', 'testLines', 'testIds', 'testTagsFilter', 'testNamePattern')}
        require(sha(encoded(identity, True)) == row['instanceId'], 'actual specification instance ID differs: ' + row['instanceId'])
        require(all(row[name] is None for name in ('testLines', 'testIds', 'testTagsFilter', 'testNamePattern')), 'original test filters changed')
    prepared = {row['instanceId']: row for row in prep['prepared']}
    declined = {row['instanceId']: row for row in prep['declined']}
    require(len(prepared) == len(prep['prepared']) and len(declined) == len(prep['declined']), 'duplicate preparation decisions')
    require(not prepared.keys() & declined.keys() and prepared.keys() | declined.keys() == selected_by_id.keys(), 'prepared/declined disjoint union differs from actual selection')
    require(plan['identity']['plannerSha256'] == source_sha['matrix-planner.mjs'], 'planner source differs')
    require(plan['key'] == sha(b'quod-managed-matrix-plan-v1-research\n' + encoded(plan['identity'], True)) == result['planKey'], 'plan identity key differs')
    planned = {row['instanceId']: row for row in plan['identity']['instances']}
    require(planned.keys() == selected_by_id.keys() and len(planned) == len(plan['identity']['instances']), 'plan omitted/repeated/added a selected instance')
    require(plan['prepared'] == len(prepared) and plan['declined'] == len(declined), 'plan decision counts differ')
    for instance_id, decision in planned.items():
        require(decision['specification'] == {**selected_by_id[instance_id], 'absolute': '/project/' + selected_by_id[instance_id]['file']}, 'planned actual specification differs')
        require(decision['decision'] == ('attempt-managed' if instance_id in prepared else 'ordinary'), 'planned preparation decision differs')
        if instance_id in prepared:
            require(decision['preparation'] == prepared[instance_id]['identity'], 'keyed preparation identity differs')

    require(native['schema'] == 'quod-managed-matrix-native-v1' and native['completed'] is True and native['matched'] is True and native['runStarted'] is True, 'complete exact native control required')
    require(native['exitCode'] == 0 and native['unhandledErrors'] == [] and native['failedModules'] == 0, 'native test/control failure')
    require({row['instanceId'] for row in native['selected']} == selected_by_id.keys(), 'native selected IDs differ')
    actual = {key(row): row for row in native['executed']}
    require(len(actual) == len(native['executed']) == 120 and actual.keys() == {key(row) for row in selected}, 'native omitted/repeated an actual project/file instance')
    require(native['reportSha256'] == sha((matrix / 'native-report.json').read_bytes()), 'original native JSON SHA differs')
    require(report['success'] is True and len(report['testResults']) == 120, 'original native JSON files/success differ')
    json_rows = [[relative(file['name']), row['fullName'], STATE[row['status']]] for file in report['testResults'] for row in file['assertionResults']]
    require(len(json_rows) == 4131 and rows_hash(json_rows) == ORIGINAL_STATES_SHA, 'original4131 complete name/state fingerprint differs')
    api_rows = [[file['file'], row['name'], row['state']] for file in native['executed'] for row in file['rows']]
    require(collections.Counter(map(tuple, json_rows)) == collections.Counter(map(tuple, api_rows)), 'public native APIs differ from unmodified JSON report')
    project_rows = [[file['project'], file['file'], row['name'], row['state']] for file in native['executed'] for row in file['rows']]
    require(rows_hash(project_rows) == ORIGINAL_PROJECT_STATES_SHA, 'original project-labelled4131 names/states differ')
    for row in native['executed']:
        context = {'expectedZodAlias': row['project']} if re.fullmatch(r'zod-4\.[0-6]', row['project']) else {}
        require(row['providedContext'] == context, 'actual native provided context differs: ' + row['project'])

    runtime_policy = result['runtimePolicy']
    require(runtime_policy['boundaryContract'] == CONTRACT and runtime_policy['intrinsicPolicy'] == POLICY, 'runtime policy label differs')
    require(runtime_policy['services'] == ['main', 'deny'] and runtime_policy['globalOutbound'] == 'deny' and runtime_policy['bindings'] == [], 'native service capabilities expanded')
    require(runtime_policy['compatibilityFlags'] == ['nodejs_als'] and runtime_policy['moduleName'] == 'worker.mjs', 'native flags/module expanded')
    require(runtime_policy['supervisor']['sourceSha256'] == source_sha['executor.mjs'], 'supervisor source differs')
    seen_pids = set()

    def completed(data):
        boundary = data.get('boundary', {})
        completion = boundary.get('completion', {})
        return (boundary.get('contract') == CONTRACT and boundary.get('intrinsicPolicy') == POLICY and
                completion.get('sealed') is True and completion.get('intrinsicLockdown') is True and completion.get('reason') == '' and
                boundary.get('pendingTimers') == 0 and all(completion.get(name) == 0 for name in ('pendingTimerCallbacks', 'pendingMicrotasks', 'pendingEventCallbacks')) and
                all(boundary.get(name) == [] for name in ('unguardable', 'symbolUnguardable', 'timerErrors', 'unhandledErrors')))

    def phase(directory, built):
        invocation = load(directory / 'invocation.json')
        require(invocation['bundleSha256'] == built['sha256'] == sha((directory / 'worker.mjs').read_bytes()), 'actual worker bytes differ: ' + str(directory))
        require(invocation['configSha256'] == sha((directory / 'config.capnp').read_bytes()), 'actual service config differs')
        policy = invocation['runtimePolicy']
        require({k: v for k, v in policy.items() if k != 'supervisor'} == {k: v for k, v in runtime_policy.items() if k != 'supervisor'}, 'instance native runtime capability policy differs')
        require(invocation['host'] == result['host'], 'instance native host differs')
        require(policy['supervisor']['sourceSha256'] == source_sha['executor.mjs'], 'instance actual supervisor differs')
        require(invocation['pid'] not in seen_pids, 'fresh process PID reused in this short gate')
        seen_pids.add(invocation['pid'])
        execution = load(directory / 'execution.json')
        if not (directory / 'response.json').is_file():
            require(execution.get('error') is not None or execution.get('timedOut') is True, 'missing response needs visible execution failure')
            return None
        data = load(directory / 'response.json')
        digest = sha((directory / 'response.json').read_bytes())
        require(execution.get('responseSha256') == digest == load(directory / 'response-meta.json')['sha256'], 'actual response bytes differ')
        return data

    def build(built):
        identity = built['identity']
        require(identity['compilerSha256'] == source_sha['compiler.mjs'], 'actual compiler source differs')
        require(built['identitySha256'] == sha(encoded(identity, True)), 'compile identity hash differs')
        require(identity['configurationSha256'] == sha(encoded(built['options'], True)), 'actual declared compile options differ')
        closure = built['closure']
        require(closure['finalBundleParsed'] is True and closure['remainingDynamicImports'] == 0 and closure['remainingDynamicRequires'] == 0 and closure['externalImports'] == ['node:async_hooks'], 'final module graph is not closed')
        require(identity['policy']['dynamicCode'] == 'private-sticky-denial-plus-runtime-constructor-guards', 'actual eval guard policy changed')
        if self_glob_policy:
            require(identity['policy'].get('lazyGlob') == 'actual-real-test-entry-only; all-other-matches-sticky-denied' and
                    identity['policy']['importMeta'] == 'literal-relative-glob-self-entry-only; all-other-access-sticky-denied', 'new self-entry-only lazy import policy differs')
            require(built['globs'] == identity['globs'], 'compiled/keyed glob inventory differs')
            for glob in identity['globs']:
                require(glob['names'] == [match['name'] for match in glob['matches']], 'glob removed or changed a matched foreign key')
                admitted = [match for match in glob['matches'] if match['target'] == identity['testEntry']]
                require(glob['admittedTargets'] == admitted, 'glob can import a foreign target or omitted the exact self target')
                require(all(any(source['source'] == match['target'] for source in identity['sources']) for match in admitted), 'admitted glob target is outside the closed original source graph')
        for source in identity['sources']:
            if source['source'].startswith('/quod-adapter/'):
                name = source['source'].removeprefix('/quod-adapter/')
                expected = source_sha.get(name)
                if expected is not None:
                    require(source['sourceSha256'] == expected, 'actual adapter source differs: ' + name)
        for adapter in identity['adapters']:
            if '/v4/core/util.js' in adapter['source']:
                require(ZOD_PROBES.get(adapter['sourceSha256']) == adapter.get('probeSha256'), 'Zod exact published probe source differs')
                require(adapter['policy'] == 'Exact Zod optional evaluator probe disabled; existing interpreter fallback only; actual eval remains denied', 'Zod adapter capability policy changed')

    preflight_build = load(matrix / 'preflight-build.json')
    build(preflight_build)
    preflights = [phase(matrix / name, preflight_build) for name in ('preflight-one', 'preflight-two')]
    for data in preflights:
        require(data is not None and data.get('ok') is True and completed(data) and len(data['checks']) == 31 and len({row['name'] for row in data['checks']}) == 31 and all(row['pass'] is True for row in data['checks']), 'all31 private boundary controls must pass in both fresh realms')
    if all(preflights):
        require(preflights[0]['random'] == preflights[1]['random'], 'fresh deterministic random differs')
        for name in ('inventory', 'symbolInventory'):
            require(preflights[0]['boundary'][name] == preflights[1]['boundary'][name], 'fresh ambient inventory differs')
    validation = load(matrix / 'compiler-validation.json')
    require(len(validation) == 5 and all(row['pass'] is True and row['rejection']['name'] == 'AssertionError' for row in validation), 'all5 alias/context compiler negative controls required')
    require(result['compilerValidationChecks'] == validation, 'compiler negative control result/file differ')
    regression_file = matrix.parent / 'matrix-compiler-checks/result.json'
    if regression_file.is_file():
        regressions = load(regression_file)
        require(len(regressions['checks']) == (54 if self_glob_policy else 40) and all(row['passed'] is True for row in regressions['checks']), 'original40 or added14 compiler regressions failed/omitted')
        if self_glob_policy:
            require(regressions['originalCheckCount'] == 40 and regressions['selfGlobCheckCount'] == 14, 'original/new compiler submatrix counts differ')
            new_names = [
                'adding a denied foreign glob name changes closed identity',
                'removing that glob name restores the exact prior closed identity',
            ]
            for name in ('self-glob', 'foreign-glob'):
                new_names.extend(name + suffix for suffix in (
                    ' final module graph is closed', ' admits only its exact actual entry',
                    ' actual original assertions pass', ' private completion remains sealed',
                    ' denial inventory is exact after caught calls',
                ))
                built = load(regression_file.parent / (name + '.json'))
                build(built)
                data = phase(regression_file.parent / ('runtime-' + name), built)
                expected_denials = [] if name == 'self-glob' else ['lazy module ./foreign-modules/native.mjs', 'lazy module ./foreign-modules/value.mjs']
                require(data is not None and data.get('ok') is True and completed(data) and
                        len(data['rows']) == (2 if name == 'self-glob' else 1) and all(row['state'] == 'pass' for row in data['rows']) and
                        sorted(data['boundary']['unsupported']) == expected_denials, 'actual self/foreign glob runtime semantics or sticky denials failed')
                admitted = [match for glob in built['globs'] for match in glob['admittedTargets']]
                require(len(admitted) == (1 if name == 'self-glob' else 0), 'runtime fixture did not prove the bounded glob admission')
            new_names.extend([
                'glob symlink cannot expose a private adapter even as a denied foreign key',
                'glob target native imports remain denied when selected as the actual entry',
            ])
            require(regressions['checks'][40:] == [{'name': name, 'passed': True} for name in new_names], 'added14 exact closed-glob control roster differs')
    else:
        require(False, 'original40 compiler regression evidence missing')

    instances = result['instances']
    require(len(instances) == 120 and {row['instanceId'] for row in instances} == selected_by_id.keys(), 'result omitted/repeated selected instances')
    supported_roster = []
    declines = collections.Counter()
    disagreements = 0
    managed_passing = 0
    failures = 0
    for row in instances:
        instance_id = row['instanceId']
        native_instance = actual[key(row)]
        require(row['nativeExecuted'] is True and row['nativeCount'] == len(native_instance['rows']) and row['nativeState'] == native_instance['state'], 'native fallback evidence differs for ' + instance_id)
        if instance_id not in prepared:
            require(row['stage'] == 'preparation' and row['supported'] is False and row['decision'] == 'ordinary' and row['reason'] == declined[instance_id]['reason'], 'preparation decline became invisible or authoritative')
            declines['preparation: ' + row['reason']] += 1
            continue
        directory = matrix / 'instances' / instance_id
        preparation = load(directory / 'preparation.json')
        require(preparation == prepared[instance_id]['identity'] and preparation['preparerSha256'] == source_sha['preparation.mjs'], 'actual preparation bytes/source differ')
        alias = row['project'] if re.fullmatch(r'zod-4\.[0-6]', row['project']) else None
        context = {'expectedZodAlias': alias} if alias else {}
        require(preparation['zodAlias'] == alias and preparation['providedContext'] == context, 'keyed actual project context/alias differs')
        zod_rules = [item for item in preparation['aliases'] if item['find']['source'] == r'^zod(?=\/|$)']
        require(len(zod_rules) == int(alias is not None) and all(item['replacement'] == alias for item in zod_rules), 'actual resolved Zod alias multiplicity/target differs')
        vite_rules = [item for item in preparation['aliases'] if item not in zod_rules]
        require(sorted(item['find']['source'] for item in vite_rules) == sorted([r'^\/?@vite\/env', r'^\/?@vite\/client']) and
                all(item['find']['flags'] == '' and item['replacement'].startswith('/@fs/project/node_modules/vite/dist/client/') for item in vite_rules), 'actual Vite alias inventory/checkout path differs')
        if not (directory / 'test-build.json').is_file():
            require(row['supported'] is False and row['decision'] == 'ordinary' and row['stage'] == 'compilation' and row.get('error'), 'compile failure was omitted or admitted')
            declines['compilation: ' + row['reason']] += 1
            continue
        built = load(directory / 'test-build.json')
        build(built)
        options = built['options']
        require(preparation['configurationSha256'] == sha(encoded(options['vitest'], True)), 'prepared actual configuration differs from compile options')
        require(options.get('zodAlias') == alias and options['providedContext'] == context and options['vitest']['name'] == row['project'] and options['vitest']['isolate'] is True, 'compiled alias/context/isolation differs')
        require(options['file'] == '/project/' + row['file'] and built['identity']['testEntry'] == options['file'], 'compiled actual test source identity differs')
        for resolution in built['identity']['resolutions']:
            if alias and re.match(r'^zod(?:/|$)', resolution['specifier']):
                require(resolution['target'].startswith('/project/node_modules/' + alias + '/'), 'closed dependency bypassed actual project Zod alias')
        if row['file'].endswith('/stream.property.test.ts'):
            sources = [source for source in built['identity']['sources'] if source['source'] == options['file']]
            require(len(sources) == 1 and sources[0]['sourceSha256'] == ORIGINAL_47_SOURCE_SHA, 'original47 property source changed')
        if not (directory / 'managed/invocation.json').is_file():
            require(row['supported'] is False and row['decision'] == 'ordinary' and row['stage'] == 'execution' and row.get('error'), 'execution never started and failure was omitted or admitted')
            declines['execution startup: ' + row['reason']] += 1
            continue
        data = phase(directory / 'managed', built)
        actual_supported = (data is not None and not data.get('error') and completed(data) and data['boundary']['unsupported'] == [] and
                            isinstance(data.get('rows'), list) and all(isinstance(test.get('name'), str) and test.get('state') in ('pass', 'fail', 'skip', 'todo') and isinstance(test.get('errors'), list) for test in data['rows']) and
                            isinstance(data.get('files'), list) and len(data['files']) == 1)
        require(row['supported'] is bool(actual_supported), 'actual support classification differs')
        if not actual_supported:
            require(row['decision'] == 'ordinary', 'incomplete/unsupported managed result became authoritative')
            declines['execution: ' + row['reason'] + ' ' + str(row.get('unsupported', []))] += 1
            continue
        left, right = state_keys(data['rows']), state_keys(native_instance['rows'])
        equal = left == right and data['files'][0]['name'] == native_instance['relativeModuleId'] and data['files'][0]['state'] == native_instance['state']
        disagreements += int(not equal)
        require(equal, 'supported per-project managed names/states differ: ' + str(key(row)))
        compare = row['comparison']
        require(compare['equal'] is (left == right) and compare['managedSha256'] == sha(encoded(left)) and compare['nativeSha256'] == sha(encoded(right)), 'reported per-instance comparison hash differs')
        failed = any(test['state'] == 'fail' for test in data['rows']) or any(file['state'] == 'fail' for file in data['files'])
        failures += int(failed)
        managed_passing += int(not failed and data.get('ok') is True)
        require(row['decision'] == ('supported-failure' if failed else 'managed' if data.get('ok') is True else 'ordinary'), 'supported failure/result decision differs')
        supported_roster.append({'instanceId': instance_id, 'project': row['project'], 'file': row['file'], 'tests': len(data['rows']), 'workerSeconds': row['workerSeconds']})
    coverage = {'selected': 120, 'prepared': len(prepared), 'supported': len(supported_roster), 'managedPassing': managed_passing,
                'ordinary': sum(row['decision'] == 'ordinary' for row in instances), 'supportedFailures': failures,
                'disagreements': disagreements, 'nativeAssertions': len(api_rows)}
    require(result['coverage'] == coverage, 'reported actual coverage differs')
    checks = []
    if b'ordinary absolute alias path is canonical' in probe_source:
        checks.extend((name, True) for name in [
            'ordinary absolute alias path is canonical',
            'Vite filesystem URL alias path is canonical',
            'unrelated alias string is preserved',
            'non-path or sibling checkout alias is rejected: /reviewed/checkout-sibling/client.mjs',
            'non-path or sibling checkout alias is rejected: /@fs/reviewed/checkout-sibling/client.mjs',
            'non-path or sibling checkout alias is rejected: arbitrary:/reviewed/checkout/client.mjs',
        ])
    checks.extend((name, True) for name in [
        'unreviewed alias is rejected',
        'alias and actual project must agree',
        'alias and provided context must agree',
        'root provided context cannot request an alias',
        'private alias strings cannot request compiler capabilities',
        'planner rejects an extra prepared instance',
        'planner rejects an extra declined instance',
        'both fresh preflights retain all 31 capability checks and complete effects',
        'fresh preflight random outputs and ambient inventories agree',
        'complete native selection and reporting finished without errors',
        'all actual project/file instances were retained',
        'every prepared or declined specification remains visible and native-executed',
    ])
    checks.extend([
        ('every supported actual managed name/state multiset agrees per project/file', disagreements == 0),
        ('supported managed failures cannot be erased by ordinary execution', failures == 0),
        ('at least one complete original managed instance passes', managed_passing > 0),
    ])
    require(result['checks'] == [{'name': name, 'pass': passed} for name, passed in checks], 'fixed control/check roster or derived pass states differ')
    require(result['preflightPassed'] is True, 'private preflight result flag differs')
    require(result['passed'] is (not result.get('error') and all(passed for _, passed in checks)), 'whole experiment pass differs from original checks/sticky failures')
    integrity = not issues
    passed = integrity and provenance and result['passed'] is True and managed_passing > 0 and failures == 0 and disagreements == 0
    return {'schema': 'quod-managed-matrix-independent-audit-v1', 'passed': passed, 'dataIntegrityPassed': integrity,
            'verifiedProvenance': provenance, 'experimentPassed': result['passed'], 'sourcePin': args.source_pin,
            'coverage': coverage, 'supportedTests': sum(row['tests'] for row in supported_roster),
            'supportedProjects': dict(collections.Counter(row['project'] for row in supported_roster)),
            'supportedRoster': supported_roster, 'declines': dict(declines),
            'nativeStatesSha256': rows_hash(json_rows), 'nativeProjectStatesSha256': rows_hash(project_rows),
            'retainedFiles': sum(path.is_file() for path in root.rglob('*')), 'issues': issues,
            'limits': 'Correctness investigation only; each prepared instance has a fresh process; no receipt admission or amortized performance claim.'}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root')
    parser.add_argument('source_pin')
    parser.add_argument('--archive')
    parser.add_argument('--run-json')
    parser.add_argument('--artifacts-json')
    arguments = parser.parse_args()
    try:
        output = audit(arguments)
    except Exception as error:
        output = {'schema': 'quod-managed-matrix-independent-audit-v1', 'passed': False,
                  'dataIntegrityPassed': False, 'issues': [type(error).__name__ + ': ' + str(error)]}
    print(json.dumps(output, indent=2))
    sys.exit(0 if output['passed'] else 1)

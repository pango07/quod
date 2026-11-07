#!/usr/bin/env python3
"""Read-only exact-byte audit of the pinned paired source-tree experiment."""
import argparse
import hashlib
import json
import pathlib
import sys
import zipfile

SHA = '318bc380e9495f0ee1cbc641b44f7f19fab4b8093283cbef42ea2bf8682023fe'
SIZE = 43616858
SOURCE = '7b7a9db7882b2a96bf4b39d810e49adc0f12e57a'
def sha(data):
    return hashlib.sha256(data).hexdigest()
def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=False).encode()
def audit(args):
    root, archive = pathlib.Path(args.root), pathlib.Path(args.archive)
    assert archive.stat().st_size == SIZE and sha(archive.read_bytes()) == SHA
    assert not any(file.is_symlink() for file in root.rglob('*'))
    with zipfile.ZipFile(archive) as zipped:
        files = [row for row in zipped.infolist() if not row.is_dir()]
        assert len(files) == 1256 and len({row.filename for row in files}) == len(files)
        for row in files:
            path = pathlib.PurePosixPath(row.filename)
            assert not path.is_absolute() and '..' not in path.parts and '\\' not in row.filename
            assert sha(root.joinpath(*path.parts).read_bytes()) == sha(zipped.read(row))
        assert {row.filename for row in files} == {str(file.relative_to(root)) for file in root.rglob('*') if file.is_file()}
    folder = root / '_temp/source-tree-cache'
    result = json.loads((folder / 'result.json').read_bytes())
    assert result['passed'] and all(row['passed'] is True for row in result['checks'])
    assert len(result['checks']) == 85 and len({row['name'] for row in result['checks']}) == 85
    assert all(result[name] is False for name in ['receiptAdmission', 'sourceReadsCached', 'resolutionsCached', 'finalBundleASTCached'])
    pairs = result['pairs']; assert len(pairs) == 12 and len({row['instanceId'] for row in pairs}) == 12
    assert sorted(row['project'] for row in pairs) == ['convex-helpers'] * 5 + ['zod-4.' + str(i) for i in range(7)]
    seconds = {'on': 0, 'off': 0}
    faster_pairs = 0
    for index, pair in enumerate(pairs):
        assert pair['order'] == (['on', 'off'] if index % 2 else ['off', 'on'])
        directory = folder / pair['instanceId']
        encoded = {}
        for mode in ['on', 'off']:
            data = (directory / (mode + '.mjs')).read_bytes()
            metadata = json.loads((directory / (mode + '.json')).read_bytes())
            telemetry = pair['builds'][mode]
            assert sha(data) == metadata['sha256'] == telemetry['bundleSha256'] == metadata['identity']['bundleSha256']
            assert sha(canonical(metadata['identity'])) == metadata['identitySha256'] == telemetry['identitySha256']
            assert telemetry['sourceTreeCache'] == metadata['sourceTreeCache']
            cache = telemetry['sourceTreeCache']
            assert cache['enabled'] is (mode == 'on') and cache['finalBundleParses'] == index + 1
            assert 0 <= cache['retainedEntries'] <= 128 and 0 <= cache['retainedSourceBytes'] <= 8 * 1024 * 1024
            assert cache['limitEntries'] == 128 and cache['limitSourceBytes'] == 8 * 1024 * 1024 and cache['maxEntrySourceBytes'] == 512 * 1024
            if mode == 'off':
                assert cache['hits'] == cache['crossBuildHits'] == cache['retainedEntries'] == cache['retainedSourceBytes'] == 0
            encoded[mode] = (data, metadata['identity'])
            seconds[mode] += telemetry['seconds']
        assert encoded['on'] == encoded['off']
        faster_pairs += pair['builds']['on']['seconds'] < pair['builds']['off']['seconds']
    assert pairs[-1]['builds']['on']['sourceTreeCache']['crossBuildHits'] > 0
    assert seconds == result['seconds']
    matrix = json.loads((root / '_temp/managed-matrix/result.json').read_bytes())
    assert matrix['passed'] and matrix['coverage'] == {'selected': 120, 'prepared': 118, 'supported': 108,
        'managedPassing': 108, 'ordinary': 12, 'supportedFailures': 0, 'disagreements': 0, 'nativeAssertions': 4131}
    return {'schema': 'quod-source-tree-cache-independent-audit-v1', 'passed': True, 'issues': [],
            'sourcePin': SOURCE, 'artifactSha256': SHA, 'pairedBuilds': 12, 'checks': 85,
            'allBundleBytesAndIdentitiesEqual': True, 'seconds': seconds, 'cachedFasterPairs': faster_pairs,
            'observedPairedTotalReduction': 1 - seconds['on'] / seconds['off'],
            'crossBuildHits': pairs[-1]['builds']['on']['sourceTreeCache']['crossBuildHits'],
            'coverage': matrix['coverage'], 'limit': 'One alternating same-runner diagnostic; no general speedup or full-job claim.'}
if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root'); parser.add_argument('--archive', required=True)
    try:
        result = audit(parser.parse_args())
    except Exception as error:
        result = {'schema': 'quod-source-tree-cache-independent-audit-v1', 'passed': False,
                  'issues': [type(error).__name__ + ': ' + str(error)]}
    print(json.dumps(result, indent=2)); sys.exit(0 if result['passed'] else 1)

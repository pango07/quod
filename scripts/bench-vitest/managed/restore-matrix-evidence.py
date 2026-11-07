#!/usr/bin/env python3
"""Fetch one explicitly pinned matrix artifact as data; never execute its files."""
import concurrent.futures
import hashlib
import json
import os
import pathlib
import re
import stat
import subprocess
import sys
import zipfile

artifact_id, source_pin, run_id, expected_size, expected_sha, output = sys.argv[1:]
assert os.environ.get('GITHUB_ACTIONS') == 'true', 'remote evidence gate only'
assert artifact_id.isdigit() and run_id.isdigit() and re.fullmatch('[0-9a-f]{40}', source_pin)
assert re.fullmatch('[0-9a-f]{64}', expected_sha)
size = int(expected_size)
assert 0 < size <= 100 * 1024 * 1024
root = pathlib.Path(output)
assert not root.exists() and not root.is_symlink(), 'evidence output must be new'
archive = pathlib.Path(output + '.zip')
assert not archive.exists() and not archive.is_symlink()

def api(endpoint, headers=()):
    command = ['gh', 'api', *sum((['-H', header] for header in headers), []), endpoint]
    response = subprocess.run(command, capture_output=True, timeout=60)
    if response.returncode:
        raise RuntimeError('GitHub evidence fetch failed: ' + response.stderr.decode(errors='replace')[:300])
    return response.stdout

run_bytes = api('repos/pango07/quod/actions/runs/' + run_id)
artifact_bytes = api('repos/pango07/quod/actions/runs/' + run_id + '/artifacts')
run, artifacts = json.loads(run_bytes), json.loads(artifact_bytes)
assert run['head_sha'] == source_pin and run['run_attempt'] == 1
assert run['status'] == 'completed' and run['conclusion'] == 'success'
matching = [item for item in artifacts['artifacts'] if str(item['id']) == artifact_id]
assert len(matching) == 1
item = matching[0]
assert item['name'] == 'managed-matrix' and not item['expired']
assert item['size_in_bytes'] == size and item['digest'] == 'sha256:' + expected_sha
assert item['workflow_run']['id'] == int(run_id) and item['workflow_run']['head_sha'] == source_pin
chunk = 2 * 1024 * 1024
ranges = [(start, min(size - 1, start + chunk - 1)) for start in range(0, size, chunk)]
def fetch(pair):
    start, end = pair
    data = api('repos/pango07/quod/actions/artifacts/' + artifact_id + '/zip',
               ['Range: bytes=' + str(start) + '-' + str(end)])
    assert len(data) == end - start + 1, 'artifact redirect did not honor exact byte range'
    return data
first = fetch(ranges[0])
with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
    data = first + b''.join(pool.map(fetch, ranges[1:]))
assert len(data) == size and hashlib.sha256(data).hexdigest() == expected_sha
archive.write_bytes(data)
with zipfile.ZipFile(archive) as zipped:
    entries = zipped.infolist()
    assert len(entries) <= 10000 and sum(entry.file_size for entry in entries) <= 500 * 1024 * 1024
    names = set()
    for entry in entries:
        name = pathlib.PurePosixPath(entry.filename)
        assert name.parts and not name.is_absolute() and '..' not in name.parts and '\\' not in entry.filename
        assert str(name) == entry.filename.rstrip('/'), 'non-canonical artifact path'
        assert entry.filename not in names and not entry.flag_bits & 1
        names.add(entry.filename)
        mode = stat.S_IFMT(entry.external_attr >> 16)
        assert mode in (0, stat.S_IFREG, stat.S_IFDIR), 'non-data artifact entry'
        assert entry.file_size <= 32 * 1024 * 1024
    root.mkdir()
    zipped.extractall(root)
pathlib.Path(output + '-run.json').write_bytes(run_bytes)
pathlib.Path(output + '-artifacts.json').write_bytes(artifact_bytes)
print(json.dumps({'run': int(run_id), 'sourcePin': source_pin, 'artifact': int(artifact_id),
                  'size': size, 'sha256': expected_sha, 'entries': len(entries),
                  'uncompressedBytes': sum(entry.file_size for entry in entries)}))

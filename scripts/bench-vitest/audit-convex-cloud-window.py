#!/usr/bin/env python3
"""Independently audit downloaded convex-helpers job evidence (no build/test execution).

Run from the repository root: SCRIPT ARTIFACT_DIR OUTPUT_JSON COORDINATOR_ID
Optionally append a comma-separated variant list and final-window commit count.
"""
import collections, datetime, hashlib, json, math, pathlib, re, statistics, sys

root = pathlib.Path(sys.argv[1])
dest = pathlib.Path(sys.argv[2])
run_id = int(sys.argv[3])
variants = sys.argv[4].split(',') if len(sys.argv) > 4 else ['floor', 'plain', 'quod', 'quod-ts']
count = int(sys.argv[5]) if len(sys.argv) > 5 else 12
def jsonl(name):
    return [json.loads(s) for s in (root / name).read_text().splitlines() if s]
rows, expected = jsonl('jobs.jsonl'), jsonl('expected.jsonl')
key = lambda r: (r['id'], r['variant'], r['i'], r['commit'])
assert len(rows) == len(expected) == count * len(variants)
assert len({key(r) for r in rows}) == len(rows)
assert len({r['run'] for r in rows}) == len(rows)
assert len({r['job']['id'] for r in rows}) == len(rows)
assert {key(r) for r in rows} == {key(r) for r in expected}
assert {r['variant'] for r in rows} == set(variants)
assert {r['i'] for r in rows} == set(range(count))
assert {r['id'] for r in rows} == {'convex-helpers'}
assert json.loads((root / 'settings.json').read_text())['experimentalCloud'] is True
old = [json.loads(s) for s in pathlib.Path('docs/bench/convex-helpers-native-origin-jobs.jsonl').read_text().splitlines()]
commits = {r['i']: r['commit'] for r in old}
for r in rows:
    assert r['commit'] == commits[r['i'] + 12 - count], 'different original benchmark window'
    assert r['job']['conclusion'] == r['run_meta']['conclusion'] == 'success'
    assert r['image'] and re.fullmatch(r'ubuntu-\d+\.\d+/\d{8}\.\d+\.\d+', r['image'])

stamp = lambda s: datetime.datetime.fromisoformat(s.replace('Z', '+00:00'))
seconds = lambda s, e: (stamp(e) - stamp(s)).total_seconds()
plain = {r['i']: r for r in rows if r['variant'] == 'plain'}
metrics = json.loads((root / 'job-metrics.json').read_text())['per']
provided = {r['run']: r for r in metrics}
assert len(metrics) == len(rows)
per = []
for r in sorted(rows, key=lambda r: (r['i'], r['variant'])):
    j = r['job']
    duration = seconds(j['started_at'], j['completed_at'])
    assert 0 < duration < 3600 and duration.is_integer()
    rounded = math.ceil(duration / 60)
    m = provided[r['run']]
    assert m['runner'] == duration and m['billed'] == rounded and m['image'] == r['image']
    groups = collections.defaultdict(int)
    for s in j['steps']:
        if s['conclusion'] == 'skipped': continue
        assert s['conclusion'] == 'success', s['name']
        d = seconds(s['started_at'], s['completed_at'])
        assert 0 <= d <= duration
        name = s['name']
        g = name if name in ['install', 'build', 'test'] else 'post' if name.startswith('Post ') or name == 'Complete job' else 'action' if name == 'quod' else 'other'
        groups[g] += d
    q = r['quod']
    ran = [tuple(map(int, x)) for x in re.findall(r'quod vitest: ran (\d+) files \((\d+) recorded, (\d+) failed, (\d+) not recordable\), replayed (\d+), audited (\d+)', q)]
    lineage = [tuple(map(int, x)) for x in re.findall(r'lineage: (\d+) promoted, (\d+) plain audits, (\d+) disagreed, (\d+) revoked', q)]
    cloud = [int(x) for x in re.findall(r'saved the ledger to Quod Cloud \(.*?, (\d+) ms\)', q)]
    wrong = None
    if r['variant'] not in ['floor', 'plain']:
        p = plain[r['i']]
        assert re.fullmatch('[0-9a-f]{64}', r['semantic'])
        assert p['semantic'] == r['semantic']
        assert r['sig'] and p['sig']
        assert collections.Counter(r['sig'].splitlines()) == collections.Counter(p['sig'].splitlines())
        assert m['wrong'] == []
        assert len(ran) == len(lineage) == 1
        assert ran[0][2] == 0
        if ran[0][0] > 0: assert m['hit'] == 'miss'
        assert lineage[0][2] == 0
        wrong = 0
    per.append({'variant':r['variant'], 'i':r['i'], 'commit':r['commit'], 'run':r['run'], 'job':j['id'],
        'runnerSeconds':int(duration), 'modeledMinutes':rounded, 'image':r['image'],
        'semantic':r['semantic'], 'wrong':wrong, 'steps':dict(groups),
        'ran':sum(x[0] for x in ran), 'recorded':sum(x[1] for x in ran), 'replayed':sum(x[4] for x in ran),
        'unitReplays':len(re.findall(r'^quod: .* answered from its record', q, re.M)),
        'lineage':dict(zip(['promoted','audits','disagreed','revoked'], [sum(x[i] for x in lineage) for i in range(4)])),
        'cloudSavesMs':cloud, 'hit':m['hit']})
summary = {}
for v in variants:
    xs = [r for r in per if r['variant'] == v]
    def total(rs): return {'runnerSeconds':sum(r['runnerSeconds'] for r in rs), 'modeledMinutes':sum(r['modeledMinutes'] for r in rs)}
    s = {'jobs':len(xs), 'window':total(xs), 'afterFirst':total([r for r in xs if r['i'] > 0]),
         'medianJobSeconds':statistics.median(r['runnerSeconds'] for r in xs),
         'replayedFiles':sum(r['replayed'] for r in xs), 'ranFiles':sum(r['ran'] for r in xs),
         'unitReplays':sum(r['unitReplays'] for r in xs), 'fullHits':sum(r['hit']=='hit' for r in xs),
         'lineage':{k:sum(r['lineage'][k] for r in xs) for k in ['promoted','audits','disagreed','revoked']},
         'stepTotals':{g:sum(r['steps'].get(g,0) for r in xs) for g in ['install','build','test','action','post','other']}}
    if s['fullHits'] == 0:
        s['bestHitAboveFloorSeconds'] = s['medianHitAboveFloorSeconds'] = None
    else:
        gaps = []
        for r in xs:
            if r['hit'] != 'hit': continue
            assert r['ran'] == 0
            floor = next(f for f in per if f['variant']=='floor' and f['i']==r['i'])
            gaps.append(r['runnerSeconds']-floor['runnerSeconds'])
        s['bestHitAboveFloorSeconds'], s['medianHitAboveFloorSeconds'] = min(gaps), statistics.median(gaps)
    summary[v] = s
for v in variants:
    for period in ['window','afterFirst']:
        for measure in ['runnerSeconds','modeledMinutes']:
            summary[v][period][measure+'RatioToPlain'] = summary[v][period][measure] / summary['plain'][period][measure]
out = {'coordinator':run_id,'verifiedJobs':len(rows),'observedWrong':sum(r['wrong'] or 0 for r in per),
       'cloudEnabled':True,'summary':summary,'per':per,
       'files':{p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in root.iterdir() if p.is_file()}}
dest.write_text(json.dumps(out,indent=2)+'\n')
print(json.dumps({'verifiedJobs':len(rows),'observedWrong':out['observedWrong'],'summary':summary},indent=2))

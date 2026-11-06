#!/usr/bin/env python3
"""Independently check this benchmark's incremental Vitest reporter artifacts.

Usage: python3 scripts/bench-vitest/verify-test-progress.py ARTIFACT_DIRECTORY
Accepts partial phases so a timed-out diagnostic can be analyzed. A partial
journal never proves whole-suite agreement. Receipt spans include reporter
delivery delays and are not a substitute for Vitest's own test durations.
"""
from pathlib import Path
from collections import Counter
import json, math, sys

root = Path(sys.argv[1])
out = {'scope': 'Incremental reporter receipt order; no replay authority',
       'timingScope': 'Parent reporter receipt spans, including delivery delays',
       'replayEligible': False, 'phases': {}}
completed_by_phase = {}
for phase in ['baseline', 'before', 'after']:
    journal = root / (phase + '-test-progress.jsonl')
    if not journal.exists():
        continue
    rows = [json.loads(line) for line in journal.read_text().splitlines()]
    assert rows, (phase, 'empty progress journal')
    active, completed, states, durations = {}, {}, Counter(), []
    end, elapsed, ready_count = None, 0, 0
    for i, row in enumerate(rows):
        assert row['sequence'] == i and end is None
        assert math.isfinite(row['elapsedMs']) and row['elapsedMs'] >= elapsed
        elapsed = row['elapsedMs']
        if row['event'] == 'run-end':
            assert row['reason'] in ['passed', 'failed', 'interrupted']
            end = row
            continue
        assert row['event'] in ['ready', 'result']
        assert all(isinstance(row[k], str) for k in ['project', 'file', 'id', 'name'])
        key = (row['project'], row['file'], row['id'])
        if row['event'] == 'ready':
            assert key not in active and key not in completed
            active[key] = row
            ready_count += 1
        else:
            assert key in active and key not in completed
            start = active.pop(key)
            assert row['name'] == start['name']
            assert row['state'] in ['passed', 'failed', 'skipped']
            completed[key] = row
            states[row['state']] += 1
            durations.append({k: row[k] for k in ['project', 'file', 'id', 'name', 'state']} |
                             {'receiptSpanMs': elapsed - start['elapsedMs']})
    if end is not None and end['reason'] == 'passed':
        assert not active and states['failed'] == 0
    summary_path = root / (phase + '-test-progress-summary.json')
    summary = json.loads(summary_path.read_text()) if summary_path.exists() else None
    if summary:
        assert summary['events'] == len(rows) and summary['ready'] == ready_count
        assert summary['results'] == len(completed) and summary['ended'] == (end is not None)
        assert summary['failure'] is None and summary['overflow'] is False
    full_path = root / (phase + '-tests.json')
    full = json.loads(full_path.read_text()) if full_path.exists() else None
    if full:
        expected = Counter()
        for module in full['testResults']:
            file = module['name'].split('/profile-project/', 1)[1]
            for case in module['assertionResults']:
                name = ' > '.join([*case['ancestorTitles'], case['title']])
                state = {'pending': 'skipped', 'todo': 'skipped'}.get(case['status'], case['status'])
                expected[(file, name, state)] += 1
        actual = Counter((r['file'], r['name'], r['state']) for r in completed.values())
        assert expected == actual, (phase, list((expected - actual).items())[:3], list((actual - expected).items())[:3])
        assert not active and end is not None and summary is not None
        assert end['modules'] == len(full['testResults'])
    phase_path = root / (phase + '-phase.json')
    out['phases'][phase] = {
        'phaseResult': json.loads(phase_path.read_text()) if phase_path.exists() else None,
        'events': len(rows), 'ready': ready_count, 'completed': len(completed), 'states': dict(states),
        'unfinished': list(active.values()), 'runEnd': end, 'summary': summary,
        'fullJsonMatched': full is not None, 'fullJsonSuccess': full['success'] if full else None,
        'lastReceiptMs': elapsed, 'slowestReceiptSpans': sorted(durations, key=lambda d: d['receiptSpanMs'], reverse=True)[:12],
        'lastEvents': rows[-8:],
    }
    completed_by_phase[phase] = {key: (row['name'], row['state']) for key, row in completed.items()}
assert out['phases'], 'no incremental test journals found'
baseline_name = next((name for name in ['baseline', 'before']
                      if out['phases'].get(name, {}).get('fullJsonMatched')), None)
out['comparisonBaseline'] = baseline_name
baseline = completed_by_phase.get(baseline_name)
if baseline is not None:
    for phase, completed in completed_by_phase.items():
        mismatch = [dict(project=k[0], file=k[1], id=k[2], observed=v, baseline=baseline.get(k))
                    for k, v in completed.items() if baseline.get(k) != v]
        out['phases'][phase]['completedResultMismatchesAgainstBaseline'] = mismatch
        out['phases'][phase]['unobservedBaselineResults'] = len(baseline.keys() - completed.keys())
print(json.dumps(out, indent=2))

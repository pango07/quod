import copy,hashlib,json,sys
from pathlib import Path
p=Path(sys.argv[1]);mode=sys.argv[2]
load=lambda f:json.loads((p/f).read_text())
sha=lambda f:hashlib.sha256((p/f).read_bytes()).hexdigest()
fault=load('fault-injection.json');before=load('agreeing-audit-ledger.json');injected=load('fault-injected-ledger.json')
expected=copy.deepcopy(before)
raw=(p/'agreeing-audit-ledger.json').read_text();modified=raw
for id in fault['batch']:
    expected['files'][id]['deps'].remove(fault['keyId'])
    old='"deps":'+json.dumps(before['files'][id]['deps'],separators=(',',':'))
    new='"deps":'+json.dumps(expected['files'][id]['deps'],separators=(',',':'))
    assert modified.count(old)==1
    modified=modified.replace(old,new)
assert injected==expected and modified==(p/'fault-injected-ledger.json').read_text()
assert fault['originalSha256']==sha('agreeing-audit-ledger.json')
assert fault['injectedSha256']==sha('fault-injected-ledger.json')
assert before['files'][fault['auditedId']]['lineage'] is None
assert before['files'][fault['auditedId']]['plain_n']>=1
rows=load('runs.json');reports=[];actual={}
for row in rows:
    name=row['label']+'-report.json';data=load(name)
    states=sorted((Path(f['name']).name,sorted((a.get('fullName',a.get('title')),a['status']) for a in f['assertionResults'])) for f in data['testResults'])
    assert len(states)==3 and sum(len(ts) for _,ts in states)==3
    assert row['reportSha256']==sha(name)
    assert json.loads(json.dumps(states))==row['states']
    actual[row['label']]=states
    reports.append({'phase':row['label'],'sha256':sha(name)})
by={r['label']:r for r in rows}
assert by['warm']['summary']['replayed']==3
assert by['promote']['summary']['lineage_promoted']==2
assert by['agreeing-audit']['summary']['lineage_audits']==1
plain=actual['fault-plain'];assert by['fault-plain']['exit']==1
assert sum(state=='failed' for _,ts in plain for _,state in ts)==2
after=load('fault-quod-ledger.json')
for id in fault['batch']:
    r=after['files'][id]
    assert r['w']['n']==1 and r['plain_n']==0 and r['lineage'] is None and r['promotion_stamp'] is None
control=next(id for id,r in before['files'].items() if r['file'].endswith('cohort-control.test.mjs'))
assert after['files'][control]['w']==before['files'][control]['w']
assert after['files'][control]['plain_n']==before['files'][control]['plain_n']
result=load('result.json');assert result['expected']==mode
if mode=='recovery-failure':
    assert result['quodExit']==1 and not (p/'fault-quod-report.json').exists()
    summary=load('fault-quod-summary.json')
    assert summary['lineage_recovery_failed'] and summary['exit']==result['quodExit']
    wrong=None
else:
    wrong=sum(a!=b for a,b in zip(actual['fault-quod'],plain))
    assert wrong==(1 if mode=='stale' else 0)
    assert by['fault-quod']['exit']==1
    s=by['fault-quod']['summary'];assert s['lineage_revoked']==2 and s['lineage_disagreed']==1
    if mode=='correct':
        assert s['ran']==2 and s['replayed']==1 and s['failed']==2
        assert s['lineage_recovery']['invalidated_replays']==1 and s['lineage_recovery']['executed']==2
        assert all(after['files'][id]['quarantined'] for id in fault['batch'])
assert result['wrong']==wrong
evidence={'result':result,'rawReports':reports,'snapshots':{f:sha(f) for f in ['agreeing-audit-ledger.json','fault-injected-ledger.json','fault-quod-ledger.json']},
          'verified':{'onlyTwoDependencyVectorsChanged':True,'previouslyAuditedPeerInBatch':True,'independentControlTrustPreserved':True,'revocationPersisted':True,'actualReportComparisonWrong':wrong}}
print(json.dumps(evidence,indent=2))

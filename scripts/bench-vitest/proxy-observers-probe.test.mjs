import test from 'node:test'
import assert from 'node:assert/strict'
import {spawnSync} from 'node:child_process'

const module=new URL('./promise-observers-probe.mjs',import.meta.url).href
for(const kind of ['none','isProxy','inspect','cached-isProxy','cached-inspect','nested-inspect']) {
  test('actual native proxy observation: '+kind,()=>{
    const source=`import {inspectPromiseObservers} from ${JSON.stringify(module)};
import util from 'node:util';
const kind=${JSON.stringify(kind)},cachedInspect=util.inspect,cachedIsProxy=util.types.isProxy;
let traps=0;
const proxy=new Proxy({sequence:{seed:123}},{get(){traps++;throw Error('unexpected get')},ownKeys(){traps++;throw Error('unexpected enumeration')}});
const p=inspectPromiseObservers({proxyAccess:true});
if(kind==='cached-isProxy')util.types.isProxy=()=>false;
if(kind==='cached-inspect')util.inspect=()=>'';
let value=null;
if(kind==='isProxy'||kind==='cached-isProxy')value=cachedIsProxy(proxy);
if(kind==='inspect'||kind==='cached-inspect')value=cachedInspect(proxy);
if(kind==='nested-inspect')value=cachedInspect({nested:proxy},{depth:5});
const report=p.proxyAccess();p.close();
console.log(JSON.stringify({value,traps,report}));`
    const r=spawnSync(process.execPath,['--input-type=module','-e',source],{encoding:'utf8',timeout:15000})
    assert.equal(r.status,0,r.stderr||String(r.error))
    const {value,traps,report}=JSON.parse(r.stdout)
    assert.equal(report.available,true,JSON.stringify(report))
    assert.equal(report.overflow,false)
    assert.equal(report.replayEligible,false)
    assert.equal(traps,0,'native inspection must bypass all installed read traps')
    const entries=report.entries.flatMap(e=>e.entries)
    if(kind==='none')assert.deepEqual(entries,[])
    else if(kind.endsWith('isProxy')) {
      assert.equal(value,true)
      assert(entries.includes('util.types.isProxy'),'cached native identity must be observed')
    } else {
      assert.match(value,/seed: 123/)
      assert(entries.includes('util.getProxyDetails'),'native target unwrapping must be observed')
    }
  })
}

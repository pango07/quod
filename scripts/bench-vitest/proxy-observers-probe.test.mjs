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
      assert(report.entries.filter(e=>e.entries.includes('util.types.isProxy')).every(e=>e.argument.available&&e.argument.kind==='proxy'),JSON.stringify(report))
    } else {
      assert.match(value,/seed: 123/)
      assert(entries.includes('util.getProxyDetails'),'native target unwrapping must be observed')
    }
  })
}

for(const kind of ['plain','primitive','bound','getter','self-replacing-getter','changed-binding-getter','plain-member','proxy-receiver','multiple','spread','call','private-field']) {
  test('native argument identity remains bounded: '+kind,()=>{
    const source=`import {inspectPromiseObservers} from ${JSON.stringify(module)};
import {types} from 'node:util';
const kind=${JSON.stringify(kind)},native=types.isProxy,proxy=new Proxy({},{}),plain={};
let getters=0,traps=0,first=plain;
const box={get value(){getters++;return proxy}},receiver=new Proxy({value:proxy},{get(t,k){traps++;return t[k]}});
const replacing={get value(){getters++;Object.defineProperty(this,'value',{value:plain});return proxy}};
let changing={get value(){getters++;changing={value:plain};return proxy}};
const ordinary={value:plain};
const bound=native.bind(null,proxy),args=[proxy];
class Private {#value=proxy;value=plain;inspect(){return native(this.#value)}}
const p=inspectPromiseObservers({proxyAccess:true});
let result;
if(kind==='plain')result=native(plain);
if(kind==='primitive')result=native(1);
if(kind==='bound')result=bound(plain);
if(kind==='getter')result=native(box.value);
if(kind==='self-replacing-getter')result=native(replacing.value);
if(kind==='changed-binding-getter')result=native(changing.value);
if(kind==='plain-member')result=native(ordinary.value);
if(kind==='proxy-receiver')result=native(receiver.value);
if(kind==='multiple')result=native(first,first=proxy);
if(kind==='spread')result=native(...args);
if(kind==='call')result=native.call(null,proxy);
if(kind==='private-field')result=new Private().inspect();
const report=p.proxyAccess();p.close();console.log(JSON.stringify({report,result,getters,traps}));`
    const r=spawnSync(process.execPath,['--input-type=module','-e',source],{encoding:'utf8',timeout:15000})
    assert.equal(r.status,0,r.stderr||String(r.error))
    const data=JSON.parse(r.stdout)
    assert.equal(data.report.available,true,JSON.stringify(data.report))
    assert.equal(data.result,!['plain','primitive','plain-member','multiple'].includes(kind))
    assert.equal(data.getters,['getter','self-replacing-getter','changed-binding-getter'].includes(kind)?1:0,'diagnostic must not invoke an accessor')
    assert.equal(data.traps,kind==='proxy-receiver'?1:0,'diagnostic must not invoke a receiver proxy')
    const entries=data.report.entries.filter(e=>e.entries.includes('util.types.isProxy'))
    assert(entries.length>0)
    for(const e of entries) {
      assert.equal(e.argument.available,['plain','primitive'].includes(kind),JSON.stringify(e))
      if(e.argument.available)assert.equal(e.argument.kind,kind==='plain'?'object':'primitive')
      assert.equal(e.argument.replayEligible,false)
    }
  })
}

import test from 'node:test'
import assert from 'node:assert/strict'
import {spawnSync} from 'node:child_process'
import {createHash} from 'node:crypto'
import {validateCompilationCounts} from './compiled-script-log.mjs'
import {nativeEntryFrames,validateNativeEntryLog} from './native-entry-log.mjs'

const module=new URL('./promise-observers-probe.mjs',import.meta.url).href
function fixture(body) {
  const source=`import {inspectPromiseObservers} from ${JSON.stringify(module)};
let gets=0;
const target={get tmp(){gets++;return 'private-value'}},key='tmp',change={value:42,configurable:true},changes={tmp:change};
${body}`
  const r=spawnSync(process.execPath,['--input-type=module','-e',source],{encoding:'utf8',timeout:20000})
  assert.equal(r.status,0,r.stderr||String(r.error))
  const data=JSON.parse(r.stdout)
  assert.equal(data.report.available,true,JSON.stringify(data.report))
  assert.equal(data.report.overflow,false);assert.equal(data.report.replayEligible,false)
  assert.equal(data.report.completeDescriptorCoverage,false)
  assert(!JSON.stringify(data.report).includes('private-value'))
  return data
}

test('integrated native arguments retain the addon load and add no per-call debugger pauses',()=>{
  const data=fixture(`const operations=[
    ['Object.getOwnPropertyDescriptor',t=>Object.getOwnPropertyDescriptor(t,key)],
    ['Object.getOwnPropertyDescriptors',t=>Object.getOwnPropertyDescriptors(t)],
    ['Reflect.getOwnPropertyDescriptor',t=>Reflect.getOwnPropertyDescriptor(t,key)],
    ['Object.defineProperty',t=>Object.defineProperty(t,key,change)],
    ['Object.defineProperties',t=>Object.defineProperties(t,changes)],
    ['Reflect.defineProperty',t=>Reflect.defineProperty(t,key,change)],
    ['Reflect.deleteProperty',t=>Reflect.deleteProperty(t,key)],
    ['Object.freeze',t=>Object.freeze(t)],['Object.seal',t=>Object.seal(t)],
    ['Object.preventExtensions',t=>Object.preventExtensions(t)],['Reflect.preventExtensions',t=>Reflect.preventExtensions(t)],
    ['Object.isFrozen',t=>Object.isFrozen(t)],['Object.isSealed',t=>Object.isSealed(t)],
    ['Object.setPrototypeOf',t=>Object.setPrototypeOf(t,null)],['Reflect.setPrototypeOf',t=>Reflect.setPrototypeOf(t,null)],
    ['Object.prototype.__defineGetter__',t=>Object.prototype.__defineGetter__.call(t,key,()=>2)],
    ['Object.prototype.__defineSetter__',t=>Object.prototype.__defineSetter__.call(t,key,()=>{})],
    ['Object.prototype.__lookupGetter__',t=>Object.prototype.__lookupGetter__.call(t,key)],
    ['Object.prototype.__lookupSetter__',t=>Object.prototype.__lookupSetter__.call(t,key)],
  ];
const checkpoints=[],p=inspectPromiseObservers({descriptorAccess:true,nativeEntries:true,compiledScripts:true,inspectorTiming:true,
  descriptorCheckpointEvery:5,onDescriptorCheckpoint:c=>checkpoints.push(c)});
const setupTiming=p.inspectorTiming(),setupOpaque=p.opaqueAccess();
for(const [,call] of operations)call({get tmp(){gets++;return 'private-value'}});
const report=p.descriptorAccess(),compilation=p.compiledScriptReport(),timing=p.inspectorTiming(),identity=p.nativeIdentity();p.close();
console.log(JSON.stringify({report,gets,compilation,timing,setupTiming,setupOpaque,identity,checkpoints,names:operations.map(([name])=>name)}));`)
  assert.equal(data.gets,0);assert.equal(data.report.calls,19)
  validateNativeEntryLog(data.report)
  assert(data.report.entries.every(e=>nativeEntryFrames(data.report,e).some(f=>f.script?.url?.startsWith('file:'))))
  assert.equal(data.identity.targetAccessorsOnly,true)
  assert.equal(data.identity.argumentMode,'direct')
  assert.equal(data.identity.targetInspection,'native-watched')
  assert.equal(data.identity.nativeCapture,true)
  assert.equal(data.identity.handleMode,'native-private-capture')
  assert.equal(data.report.sourceEvidence.available,true)
  assert.equal(data.report.sourceEvidence.scope,'frames-of-watched-accessor-targets')
  const sources=new Map(data.report.sourceEvidence.records.map(r=>[r.scriptId,r]))
  for(const e of data.report.entries)for(const f of nativeEntryFrames(data.report,e)) {
    const source=sources.get(f.location.scriptId)
    assert(source?.available&&source.code.encoding==='utf16le'&&source.code.sha256.length===64)
  }
  assert.equal(data.setupTiming.pauses.calls,1,'the diagnostic addon load must remain observed')
  assert.deepEqual(data.setupOpaque.entries.flatMap(e=>e.entries),['dlopen'])
  assert.equal(data.timing.pauses.calls,data.setupTiming.pauses.calls,'descriptor calls must add no full pauses')
  assert.equal(data.identity.mode,'native-entry-arguments')
  assert.equal(data.identity.available,true);assert.equal(data.identity.calls,19);assert.equal(data.identity.cleared,19)
  assert.deepEqual(data.identity.errors,[]);assert.equal(data.identity.snapshotsRequested,0)
  assert.deepEqual(data.checkpoints.map(c=>c.calls),[5,10,15])
  assert.equal(data.timing.nativeCallbacks.events,data.timing.nativeCallbacks.completed)
  assert(data.timing.nativeCallbacks.events>=19&&data.timing.nativeCallbacks.elapsedMs>0)
  for(const checkpoint of data.checkpoints) {
    const timing=checkpoint.timing
    assert.equal(timing.enabled,true)
    assert.equal(timing.nativeCallbacks.events,timing.nativeCallbacks.completed+1,'current callback remains in flight at its checkpoint')
    assert(timing.nativeCallbacks.completed<data.timing.nativeCallbacks.completed)
    assert(timing.commands['Runtime.callFunctionOn'].calls>=checkpoint.calls)
    assert(timing.commands['Runtime.releaseObject'].calls>=checkpoint.calls-1)
  }
  assert(data.checkpoints[0].timing.nativeCallbacks.events<data.checkpoints.at(-1).timing.nativeCallbacks.events)
  assert.deepEqual(data.report.entries.flatMap(e=>e.entries).sort(),data.names.sort())
  assert(data.report.entries.every(e=>e.count===1&&e.argument.actualNativeEntry&&e.argument.available&&
    e.argument.target.watchedAccessorKeys.includes('tmp')&&e.argument.target.inspectionScope==='watched-properties'&&
    !Object.hasOwn(e.argument.target,'ownAccessorCount')))
  assert.equal(data.compilation.encoding,'block-dictionary-v1');assert.deepEqual(data.compilation.errors,[])
  assert.equal(data.compilation.scriptMetadataIndex.overflow,false)
  assert.equal(data.compilation.debuggerScriptCacheBytes,0)
  assert.equal(data.report.sourceEvidence.collection,'at-native-entry')
  assert.equal(data.compilation.scriptMetadataIndex.events+data.compilation.failedToParse,data.compilation.events)
  assert(data.compilation.scriptMetadataIndex.entries>data.compilation.scriptMetadataIndex.metadata)
  validateCompilationCounts(data.compilation)
})

test('watched caller evidence survives collection without any late source request',()=>{
  const code='return Object.getOwnPropertyDescriptor(value,"tmp")',filename='quod-collected-watched-caller'
  const source=`import {inspectPromiseObservers} from ${JSON.stringify(module)};
import {Session} from 'node:inspector';import vm from 'node:vm';
let gets=0;const target={get tmp(){gets++;return 'private-value'}};
const p=inspectPromiseObservers({descriptorAccess:true,nativeEntries:true,compiledScripts:true});
function invoke(){const fn=vm.compileFunction(${JSON.stringify(code)},['value'],{filename:${JSON.stringify(filename)}});fn(target);return new WeakRef(fn)}
const reference=invoke();await new Promise(resolve=>setImmediate(resolve));
for(let i=0;i<5;i++)globalThis.gc();const collected=reference.deref()===undefined;
const original=Session.prototype.post;let lateRequests=0,report,compilation;
Session.prototype.post=function(method,params,callback){if(method==='Debugger.getScriptSource'){lateRequests++;callback(Error('retired source unavailable'));return}return original.call(this,method,params,callback)};
try{report=p.descriptorAccess();compilation=p.compiledScriptReport()}finally{Session.prototype.post=original;p.close()}
console.log(JSON.stringify({report,compilation,gets,collected,lateRequests}));`
  const r=spawnSync(process.execPath,['--expose-gc','--input-type=module','-e',source],{encoding:'utf8',timeout:20000})
  assert.equal(r.status,0,r.stderr||String(r.error));const data=JSON.parse(r.stdout)
  assert(data.collected);assert.equal(data.lateRequests,0);assert.equal(data.gets,0)
  assert(data.report.available&&data.report.sourceEvidence.available)
  assert.equal(data.report.sourceEvidence.collection,'at-native-entry')
  assert.equal(data.compilation.debuggerScriptCacheBytes,0);assert.deepEqual(data.compilation.errors,[])
  const evidence=data.report.sourceEvidence.records.filter(s=>s.script.url===filename)
  assert.equal(evidence.length,1)
  assert.equal(evidence[0].code.sha256,createHash('sha256').update(code,'utf16le').digest('hex'))
  validateCompilationCounts(data.compilation)
})

test('measure Inspector retired-source cache while checking every compilation before collection',t=>{
  const rows=[]
  for(const bounded of [false,true]) {
    const source=`import {Session} from 'node:inspector';import vm from 'node:vm';
const session=new Session();session.connect();let matches=0,errors=0;
function post(method,params={}){let done=false,error,result;session.post(method,params,(e,r)=>{done=true;error=e;result=r});if(!done||error)throw error??Error('async');return result}
session.on('Debugger.scriptParsed',({params})=>{if(!params.url.startsWith('quod-source-cache-'))return;
try{const index=Number(params.url.slice('quod-source-cache-'.length));const code='1+1; // '+index+'x'.repeat(512);
if(post('Debugger.getScriptSource',{scriptId:params.scriptId}).scriptSource===code)matches++;else errors++}catch{errors++}});
post('Debugger.enable',${bounded?'{maxScriptsCacheSize:0}':'{}'});globalThis.gc();const before=process.memoryUsage();
for(let i=0;i<25000;i++)vm.runInThisContext('1+1; // '+i+'x'.repeat(512),{filename:'quod-source-cache-'+i});
await new Promise(resolve=>setImmediate(resolve));for(let i=0;i<4;i++)globalThis.gc();const after=process.memoryUsage();
session.disconnect();console.log(JSON.stringify({bounded:${bounded},compilations:25000,matches,errors,rssDelta:after.rss-before.rss,heapDelta:after.heapUsed-before.heapUsed,externalDelta:after.external-before.external}));`
    const r=spawnSync(process.execPath,['--expose-gc','--input-type=module','-e',source],{encoding:'utf8',timeout:20000})
    assert.equal(r.status,0,r.stderr||String(r.error));const row=JSON.parse(r.stdout)
    assert.equal(row.matches,row.compilations);assert.equal(row.errors,0);rows.push(row)
  }
  t.diagnostic(JSON.stringify({inspectorSourceCacheMeasurements:rows,replayEligible:false}))
})

test('integrated native arguments retain the passed target through bound replacing callees and later mutations',()=>{
  const data=fixture(`const native=Object.getOwnPropertyDescriptor,define=Object.defineProperty;let calls=0,first=target;
let box={get native(){calls++;define(this,'native',{value:native});return native.bind(null,target)}};
const p=inspectPromiseObservers({descriptorAccess:true,nativeEntries:true,compiledScripts:true});
const a=box.native(key),b=native(first,(first={},key));
const report=p.descriptorAccess();p.close();
console.log(JSON.stringify({report,gets,calls,found:[typeof a.get,typeof b.get]}));`)
  assert.equal(data.calls,1);assert.equal(data.gets,0);assert.deepEqual(data.found,['function','function'])
  const entries=data.report.entries.filter(e=>e.entries.includes('Object.getOwnPropertyDescriptor'))
  assert.equal(entries.reduce((n,e)=>n+e.count,0),2)
  assert(entries.every(e=>e.argument.actualNativeEntry&&e.argument.target.watchedAccessorKeys.includes('tmp')))
})

test('watched native target caller evidence uses actual source despite a forged diagnostic filename',()=>{
  const source='native(target,key)',url=new URL('./scalar-property-probe.mjs',import.meta.url).href
  const data=fixture(`const vm=await import('node:vm');
const p=inspectPromiseObservers({descriptorAccess:true,nativeEntries:true});
vm.runInNewContext(${JSON.stringify(source)},{native:Object.getOwnPropertyDescriptor,target,key},{filename:${JSON.stringify(url)}});
const report=p.descriptorAccess();p.close();console.log(JSON.stringify({report,gets}));`)
  assert.equal(data.gets,0);assert.equal(data.report.calls,1)
  assert.equal(data.report.sourceEvidence.available,true)
  const sources=data.report.sourceEvidence.records.filter(r=>r.script.url===url)
  assert.equal(sources.length,1)
  assert.equal(sources[0].code.sha256,createHash('sha256').update(source,'utf16le').digest('hex'))
  assert.equal(sources[0].script.isModule,false)
  assert(nativeEntryFrames(data.report,data.report.entries[0]).some(f=>f.location.scriptId===sources[0].scriptId))
})

test('integrated native arguments preserve native errors and do not invoke proxy traps',()=>{
  const data=fixture(`const isProxy=process.getBuiltinModule('util').types.isProxy;
let traps=0,failed=false;
const proxy=new Proxy({}, {getOwnPropertyDescriptor(){traps++;return undefined}});
const p=inspectPromiseObservers({descriptorAccess:true,proxyAccess:true,nativeEntries:true});
const detected=isProxy(proxy);Object.getOwnPropertyDescriptor(proxy,key);
try{Object.getOwnPropertyDescriptor()}catch(e){failed=e instanceof TypeError}
const report=p.descriptorAccess(),proxyReport=p.proxyAccess(),identity=p.nativeIdentity();p.close();
console.log(JSON.stringify({report,gets,traps,failed,detected,proxyReport,identity}));`)
  assert.equal(data.traps,1,'only the application descriptor call may trigger the trap')
  assert(data.failed&&data.detected&&data.proxyReport.available&&data.identity.available)
  validateNativeEntryLog(data.report);validateNativeEntryLog(data.proxyReport)
  assert.deepEqual(data.report.entries.map(e=>e.argument.kind),['proxy','primitive'])
  assert.equal(data.report.entries[1].argument.argumentCount,0)
  assert.equal(data.proxyReport.entries.length,1);assert.equal(data.proxyReport.entries[0].argument.kind,'proxy')
  assert.equal(data.identity.calls,3);assert.equal(data.identity.cleared,3)
  assert.equal(data.identity.argumentMode,'direct')
})

test('integrated native capture rejects an interceptor target without retaining its wrapper',()=>{
  const source=`import {inspectPromiseObservers} from ${JSON.stringify(module)};
const p=inspectPromiseObservers({descriptorAccess:true,nativeEntries:true});
const key='QUOD_NATIVE_CAPTURE_ADDON',before=Object.getOwnPropertyDescriptor(process.env,key);
const report=p.descriptorAccess(),identity=p.nativeIdentity();p.close();
console.log(JSON.stringify({report,identity,preserved:before.value===process.env[key]}));`
  const r=spawnSync(process.execPath,['--input-type=module','-e',source],{encoding:'utf8',timeout:20000})
  assert.equal(r.status,0,r.stderr||String(r.error))
  const data=JSON.parse(r.stdout)
  assert(data.preserved)
  assert.equal(data.report.available,false);assert.equal(data.identity.available,false)
  assert.equal(data.identity.calls,1);assert.equal(data.identity.cleared,1)
  assert.equal(data.identity.errors.length,1);assert.equal(data.report.entries.length,1)
  const entry=data.report.entries[0]
  assert.equal(entry.argument.available,false);assert.equal(entry.argument.kind,'object')
  assert(['native capture unavailable: api-wrapper','native capture unavailable: interceptor'].includes(entry.argument.reason))
  assert(nativeEntryFrames(data.report,entry).length>0)
  assert.equal(data.identity.failureCounts[entry.argument.reason],1)
})

for(const [api,args] of [
  ['Object.getOwnPropertyDescriptor','target,key'],['Reflect.getOwnPropertyDescriptor','target,key'],
  ['Object.getOwnPropertyDescriptors','target'],['Object.defineProperty','target,key,change'],
  ['Reflect.defineProperty','target,key,change'],['Object.defineProperties','target,changes'],
  ['Reflect.deleteProperty','target,key'],['Object.freeze','target'],['Object.seal','target'],
  ['Object.preventExtensions','target'],['Reflect.preventExtensions','target'],
  ['Object.isFrozen','target'],['Object.isSealed','target'],
  ['Object.setPrototypeOf','target,null'],['Reflect.setPrototypeOf','target,null'],
])test('cached native descriptor entry and target: '+api,()=>{
  const data=fixture(`const native=${api};const p=inspectPromiseObservers({descriptorAccess:true});
native(${args});const report=p.descriptorAccess();p.close();console.log(JSON.stringify({report,gets}));`)
  assert.equal(data.gets,0,'inspecting the target must not call its accessor')
  const events=data.report.entries.filter(e=>e.entries.includes(api))
  assert.equal(events.reduce((n,e)=>n+e.count,0),1,JSON.stringify(data.report))
  assert(events.every(e=>e.argument.available&&e.argument.kind==='object'&&e.argument.target.watchedAccessorKeys.includes('tmp')),JSON.stringify(events))
})

test('legacy getter lookup is observed through a cached intrinsic and retains an unknown receiver',()=>{
  const data=fixture(`const lookup=Object.prototype.__lookupGetter__;const p=inspectPromiseObservers({descriptorAccess:true});
const getter=lookup.call(target,key);const report=p.descriptorAccess();p.close();console.log(JSON.stringify({report,gets,found:typeof getter==='function'}));`)
  assert.equal(data.found,true);assert.equal(data.gets,0)
  const event=data.report.entries.find(e=>e.entries.includes('Object.prototype.__lookupGetter__'))
  assert(event);assert.equal(event.argument.available,false)
})

test('a later argument that changes the target binding is never reconstructed as its new value',()=>{
  const data=fixture(`const native=Object.getOwnPropertyDescriptor;let first=target;
const p=inspectPromiseObservers({descriptorAccess:true});
const result=native(first,(first={},key));const report=p.descriptorAccess();p.close();
console.log(JSON.stringify({report,gets,found:typeof result.get==='function'}));`)
  assert.equal(data.found,true);assert.equal(data.gets,0)
  const event=data.report.entries.find(e=>e.entries.includes('Object.getOwnPropertyDescriptor'))
  assert(event);assert.equal(event.argument.available,false)
})

test('self-replacing argument getter stays unknown without an extra getter invocation',()=>{
  const data=fixture(`const native=Object.getOwnPropertyDescriptor;let calls=0;
const box={get value(){calls++;Object.defineProperty(this,'value',{value:{}});return target}};
const p=inspectPromiseObservers({descriptorAccess:true});const result=native(box.value,key);
const report=p.descriptorAccess();p.close();console.log(JSON.stringify({report,gets,calls,found:typeof result.get==='function'}));`)
  assert.equal(data.found,true);assert.equal(data.gets,0);assert.equal(data.calls,1)
  const event=data.report.entries.find(e=>e.entries.includes('Object.getOwnPropertyDescriptor'))
  assert(event);assert.equal(event.argument.available,false)
})

for(const replaceOwner of [false,true])test('a callee getter returning a bound intrinsic stays unknown: '+(replaceOwner?'owner replacement':'property replacement'),()=>{
  const data=fixture(`const native=Object.getOwnPropertyDescriptor,define=Object.defineProperty;let calls=0;
let box={get native(){calls++;${replaceOwner?"box={native}":"define(this,'native',{value:native})"};return native.bind(null,target)}};
const p=inspectPromiseObservers({descriptorAccess:true,heapIdentity:true,moduleBindingEvaluation:true});
const result=box.native(key);const report=p.descriptorAccess();p.close();
console.log(JSON.stringify({report,gets,calls,found:typeof result.get==='function',replaced:box.native===native}));`)
  assert.equal(data.found,true);assert.equal(data.replaced,true);assert.equal(data.calls,1)
  const entries=data.report.entries.filter(e=>e.entries.includes('Object.getOwnPropertyDescriptor'))
  assert.equal(entries.length,1)
  assert.equal(entries[0].argument.available,false,
    'the current unbound property is not the function that received this call')
  assert.equal(entries[0].argument.reason,'native callee expression may have changed during evaluation')
})

test('an ordinary member callee also stays unknown because its earlier descriptor is unavailable',()=>{
  const data=fixture(`const box={native:Object.getOwnPropertyDescriptor};
const p=inspectPromiseObservers({descriptorAccess:true});const result=box.native(target,key);
const report=p.descriptorAccess();p.close();console.log(JSON.stringify({report,gets,found:typeof result.get==='function'}));`)
  assert.equal(data.found,true)
  assert.equal(data.report.entries[0].argument.available,false)
  assert.equal(data.report.entries[0].argument.reason,'native callee expression may have changed during evaluation')
})

test('direct syntax deletion remains an explicit uncovered channel',()=>{
  const data=fixture(`const p=inspectPromiseObservers({descriptorAccess:true});delete target.tmp;
const report=p.descriptorAccess();p.close();console.log(JSON.stringify({report,gets,present:'tmp' in target}));`)
  assert.equal(data.present,false);assert.equal(data.gets,0);assert.deepEqual(data.report.entries,[])
})

test('progress checkpoints retain cumulative calls without replacing the final observation',()=>{
  const data=fixture(`const native=Object.getOwnPropertyDescriptor,checkpoints=[];
const p=inspectPromiseObservers({descriptorAccess:true,descriptorCheckpointEvery:2,onDescriptorCheckpoint:c=>checkpoints.push(c)});
for(let i=0;i<5;i++)native(target,key);
const report=p.descriptorAccess();p.close();console.log(JSON.stringify({report,gets,checkpoints}));`)
  assert.equal(data.gets,0);assert.deepEqual(data.checkpoints.map(c=>c.calls),[2,4])
  assert.equal(data.report.calls,5)
  assert.equal(data.report.entries.reduce((n,e)=>n+e.count,0),5)
  assert(data.checkpoints.every(c=>c.apis['Object.getOwnPropertyDescriptor']===c.calls&&c.arguments.object===c.calls&&!c.replayEligible))
})

for(const [kind,expression,known] of [
  ['primitive object fields','{value:42,configurable:!0}',true],
  ['lexical object fields','{value:change,configurable:true}',true],
  ['unevaluated getter and default parameter','{get(){return 42},set:((value=(first={}))=>{})}',true],
  ['nested literals and functions','{value:[change,{get ignored(){first={};return 1}},()=>first],configurable:!!1}',true],
  ['computed literal key','{["value"]:42,configurable:true}',true],
  ['computed key assignment','{[(first={},"value")]:42}',false],
  ['object spread getter','{...spread}',false],
  ['nested member getter','{value:box.value}',false],
  ['conversion hook','{value:+numeric}',false],
  ['immediately called function','{value:(()=>{first={};return 42})()}',false],
])test('later descriptor construction preserves or invalidates target recovery: '+kind,()=>{
  const data=fixture(`const native=Object.defineProperty;let first=target,sideEffects=0;
const spread={get value(){sideEffects++;first={};return 42}};
const box={get value(){sideEffects++;first={};return 42}};
const numeric={valueOf(){sideEffects++;first={};return 42}};
const p=inspectPromiseObservers({descriptorAccess:true});
native(first,key,${expression});const report=p.descriptorAccess();p.close();
console.log(JSON.stringify({report,gets,sideEffects,originalChanged:Object.getOwnPropertyDescriptor(target,key).value!==undefined||typeof Object.getOwnPropertyDescriptor(target,key).set==='function'}));`)
  assert.equal(data.gets,0);assert.equal(data.originalChanged,true)
  assert.equal(data.sideEffects,['object spread getter','nested member getter','conversion hook'].includes(kind)?1:0)
  const events=data.report.entries.filter(e=>e.entries.includes('Object.defineProperty'))
  assert.equal(events.length,1)
  assert.equal(events[0].argument.available,known,JSON.stringify(events))
  if(known)assert(events[0].argument.target.watchedAccessorKeys.includes('tmp'))
})

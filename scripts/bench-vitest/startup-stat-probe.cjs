// Diagnostic only. Load first through NODE_OPTIONS, before user preloads.
// Captured identities are never a replay certificate on their own.
'use strict'
const fs=require('node:fs')
const path=require('node:path')
const descriptor=Object.getOwnPropertyDescriptor,descriptors=Object.getOwnPropertyDescriptors
const getPrototypeOf=Object.getPrototypeOf,ownKeys=Reflect.ownKeys,same=Object.is
const freeze=Object.freeze
const marker='--require="'+__filename.replace(/\\/g,'\\\\').replace(/"/g,'\\"')+'"'
const originalOptions=process.env.NODE_OPTIONS
const prefixed=originalOptions===marker||originalOptions?.startsWith(marker+' ')
const firstPreload=module.isPreloading===true && process._preload_modules?.[0]===__filename
// A current production binary prepends its own native-origin bootstrap.
// Accept this diagnostic immediately after that known sibling of the
// driver only if its already-loaded origin still recognizes native stat.
// This is diagnostic ordering evidence, never a replay certificate.
const driverDirectory=process.argv[1]&&path.dirname(path.resolve(process.argv[1]))
const productionPreload=driverDirectory&&path.join(driverDirectory,'origin-preload.cjs')
const productionOrigin=driverDirectory&&require.cache[path.join(driverDirectory,'stat-origin.cjs')]?.exports
const precededByProductionOrigin=module.isPreloading===true &&
  process._preload_modules?.[0]===productionPreload && process._preload_modules?.[1]===__filename &&
  productionOrigin?.permitsStatSync(fs.statSync)===true
// Restore the observable environment before the following user preloads
// run. Absence and an explicitly empty string remain distinct, and child
// processes do not inherit the main-process diagnostic accidentally.
if(prefixed) {
  if(originalOptions===marker) delete process.env.NODE_OPTIONS
  else process.env.NODE_OPTIONS=originalOptions.slice(marker.length+1)
}
function words(value) {
  const out=[];let word='',started=false,quoted=false
  for(let i=0;i<value.length;i++) {
    const c=value[i]
    if(c==='"'){quoted=!quoted;started=true}
    else if(c==='\\') {if(++i===value.length)return null;word+=value[i];started=true}
    else if(/\s/.test(c)&&!quoted){if(started){out.push(word);word='';started=false}}
    else {word+=c;started=true}
  }
  if(quoted)return null
  if(started)out.push(word)
  return out
}
function ordinaryPreloads(args) {
  if(!args)return false
  for(let i=0;i<args.length;i++) {
    if(['--require','-r','--import'].includes(args[i])) {
      if(!args[++i]||args[i].startsWith('-'))return false
    } else if(!/^--(?:require|import)=.+/.test(args[i]))return false
  }
  return true
}
const supportedOptions=ordinaryPreloads(words(process.env.NODE_OPTIONS??'')) && ordinaryPreloads(process.execArgv)
const startup=freeze({firstPreload,precededByProductionOrigin,prefixed,supportedOptions,
  available:(firstPreload||precededByProductionOrigin)&&prefixed&&supportedOptions,replayEligible:false})
const binding=process.binding('fs'),nativeStat=binding.stat,nativeSync=fs.statSync,nativeAsync=fs.promises.stat
const publicStats=fs.Stats,prototype=publicStats.prototype,constructor=descriptor(prototype,'constructor')?.value
const shapes=[]
for(let p=prototype;p;p=getPrototypeOf(p))shapes.push({object:p,parent:getPrototypeOf(p),shape:descriptors(p)})
function unchangedShapes() {
  // A preload may replace Array.prototype.every or its iterator. The
  // comparison itself uses captured primitives and indexed own arrays.
  for(let i=0;i<shapes.length;i++) {
    const {object,parent,shape}=shapes[i],keys=ownKeys(shape)
    if(getPrototypeOf(object)!==parent||ownKeys(object).length!==keys.length)return false
    for(let j=0;j<keys.length;j++) {
      const a=shape[keys[j]],b=descriptor(object,keys[j])
      if(!b||!same(a.value,b.value)||a.get!==b.get||a.set!==b.set||a.writable!==b.writable||
        a.configurable!==b.configurable||a.enumerable!==b.enumerable)return false
    }
  }
  return true
}
function snapshot() {
  let prototypeUnchanged=false
  try {
    prototypeUnchanged=descriptor(fs,'Stats')?.value===publicStats &&
      descriptor(publicStats,'prototype')?.value===prototype &&
      descriptor(constructor,'prototype')?.value===prototype && unchangedShapes()
  }catch{}
  return {startup,nativeBindingUnchanged:descriptor(binding,'stat')?.value===nativeStat,
    publicSyncUnchanged:descriptor(fs,'statSync')?.value===nativeSync,
    publicAsyncUnchanged:descriptor(fs.promises,'stat')?.value===nativeAsync,
    prototypeUnchanged,nativeArtifacts:nativeLoads.snapshot(),replayEligible:false}
}
// The same inspector stays alive across preloads and the main driver.
// Its native references are captured before a user preload can replace
// the binding or observe Stats construction. Unsupported options remain
// diagnostic facts; they never become an origin certificate.
const {inspectPromiseObservers}=require('./promise-observers-probe.mjs')
const observerRegistry=inspectPromiseObservers()
const provenance=observerRegistry.nativeStatProvenance()
// Capture file identity before a user preload can load an addon. A digest
// match is only provenance evidence; it grants no native capability trust.
const {observeNativeLoads}=require('./native-load-probe.mjs')
const nativeLoads=observeNativeLoads({digests:[
  '09efa62dc8d03fb62f06ab2aadbbc26bde416acb7da20a927f7d53b2bd440322',
]})
module.exports=freeze({startup,snapshot,observerRegistry,provenance,nativeLoads})

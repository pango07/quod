// Native identities captured before user preloads. No wrappers, Inspector
// connection, global bridge, or narrower key is installed by this module.
'use strict'
const fs=require('node:fs'),path=require('node:path')
const descriptor=Object.getOwnPropertyDescriptor,descriptors=Object.getOwnPropertyDescriptors
const getPrototypeOf=Object.getPrototypeOf,ownKeys=Reflect.ownKeys,same=Object.is
const preload=path.join(__dirname,'origin-preload.cjs')
function words(value) {
  const out=[];let word='',started=false,quoted=false
  for(let i=0;i<value.length;i++) {
    const c=value[i]
    if(c==='"'){quoted=!quoted;started=true}
    else if(c==='\\'){if(++i===value.length)return null;word+=value[i];started=true}
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
let available=module.parent?.filename===preload && module.parent?.isPreloading===true &&
  process._preload_modules?.[0]===preload && ordinaryPreloads(words(process.env.NODE_OPTIONS??'')) &&
  ordinaryPreloads(process.execArgv)
const binding=process.binding('fs'),nativeBindingStat=binding.stat,nativeSync=fs.statSync
const Stats=fs.Stats,prototype=Stats.prototype,ctor=descriptor(prototype,'constructor')?.value
const shapes=[],fields=['dev','mode','nlink','uid','gid','rdev','blksize','ino','size','blocks',
  'atimeMs','mtimeMs','ctimeMs','birthtimeMs']
for(let p=prototype;p;p=getPrototypeOf(p)) {
  shapes.push({object:p,parent:getPrototypeOf(p),shape:descriptors(p)})
  for(let i=0;i<fields.length;i++) if(descriptor(p,fields[i])!==undefined)available=false
}
function permitsStatSync(candidate) {
  if(!available||candidate!==nativeSync) return false
  try {
    if(descriptor(binding,'stat')?.value!==nativeBindingStat||descriptor(fs,'Stats')?.value!==Stats||
       descriptor(Stats,'prototype')?.value!==prototype||descriptor(ctor,'prototype')?.value!==prototype)return false
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
  }catch{return false}
}
// Disabling is irreversible; no consumer can bless a new initial snapshot.
module.exports=Object.freeze({permitsStatSync,disable(){available=false}})

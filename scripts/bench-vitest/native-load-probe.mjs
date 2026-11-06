// Diagnostic only. Establish artifact identity around a real dlopen call,
// not the addon's capabilities or permission to refine a dependency key.
// Capture these functions before recorder/user wrappers are installed.
import fs from 'node:fs'
import {createHash} from 'node:crypto'
import {Buffer} from 'node:buffer'

const {openSync,closeSync,fstatSync,statSync,readFileSync,readSync}=fs
const apply=Reflect.apply
const noAtime=fs.constants.O_NOATIME,readOnly=fs.constants.O_RDONLY
const fields=['dev','ino','mode','nlink','size','mtimeNs','ctimeNs']
const identity=stat=>Object.fromEntries(fields.map(k=>[k,stat[k].toString()]))
const same=(a,b)=>fields.every(k=>a[k]===b[k])
function maps() {
  return readFileSync('/proc/self/maps','utf8').trimEnd().split('\n').map(line=>{
    const m=/^([\da-f]+)-([\da-f]+)\s+([rwxps-]+)\s+([\da-f]+)\s+([\da-f]+):([\da-f]+)\s+(\d+)(?:\s+(.*))?$/.exec(line)
    if(!m)throw Error('unknown Linux maps row')
    return {start:m[1],end:m[2],permissions:m[3],offset:m[4],major:BigInt('0x'+m[5]),
      minor:BigInt('0x'+m[6]),inode:m[7],path:m[8]??''}
  })
}
function mappingsFor(stat) {
  const dev=BigInt(stat.dev)
  const major=((dev>>8n)&0xfffn)|((dev>>32n)&~0xfffn)
  const minor=(dev&0xffn)|((dev>>12n)&~0xffn)
  return maps().filter(row=>row.major===major&&row.minor===minor&&row.inode===stat.ino)
    .map(({major,minor,...row})=>row)
}

export function observeNativeLoads({digests=[]}={}) {
  const allowed=new Set(digests),loads=[],original=process.dlopen
  let closed=false
  function capture(file) {
    const record={file:typeof file==='string'?file:null,sha256:null,expectedDigest:false,
      noAtimeRead:false,before:null,after:null,mappings:[],mappedIdentity:false,
      unchanged:false,loadReturned:false,reason:null,replayEligible:false}
    const state={record,fd:null}
    loads.push(state)
    try {
      if(process.platform!=='linux'||!Number.isInteger(noAtime))throw Error('unsupported native artifact reader')
      if(typeof file!=='string'||!file.startsWith('/'))throw Error('non-absolute native artifact path')
      state.fd=openSync(file,readOnly|noAtime)
      const before=fstatSync(state.fd,{bigint:true})
      if(!before.isFile()||before.size<=0n||before.size>268435456n)throw Error('unsupported native artifact size or kind')
      record.before=identity(before)
      const hash=createHash('sha256'),chunk=Buffer.alloc(65536)
      let position=0
      while(position<Number(before.size)) {
        const count=readSync(state.fd,chunk,0,Math.min(chunk.length,Number(before.size)-position),position)
        if(!count)throw Error('short native artifact read')
        hash.update(chunk.subarray(0,count));position+=count
      }
      const after=fstatSync(state.fd,{bigint:true})
      if(!same(record.before,identity(after)))throw Error('native artifact changed during hashing')
      record.noAtimeRead=before.atimeNs===after.atimeNs
      if(!record.noAtimeRead)throw Error('native artifact hash changed access time')
      record.sha256=hash.digest('hex');record.expectedDigest=allowed.has(record.sha256)
    }catch(e){record.reason=e.message}
    return state
  }
  function check(state,{mapping=false}={}) {
    const {record,fd}=state
    if(record.reason||fd===null||!record.before)return false
    try {
      const held=identity(fstatSync(fd,{bigint:true}))
      const named=identity(statSync(record.file,{bigint:true}))
      if(!same(record.before,held)||!same(held,named))throw Error('native artifact identity or contents may have changed')
      record.after=held
      if(mapping) {
        record.mappings=mappingsFor(held)
        record.mappedIdentity=record.mappings.some(m=>m.permissions.includes('x'))
        if(!record.mappedIdentity)throw Error('no executable mapping for the captured native artifact')
      }
      record.unchanged=true
      return true
    }catch(e){record.reason=e.message;record.unchanged=false;return false}
  }
  const wrapper=function dlopen(...args) {
    const state=capture(args[1])
    try {
      const value=apply(original,this,args)
      state.record.loadReturned=true
      return value
    }finally {
      // Diagnostics never replace the loader's return value or thrown error.
      // Failure is sticky and remains visible even if a path is restored.
      check(state,{mapping:true})
    }
  }
  process.dlopen=wrapper
  return {
    snapshot() {
      if(closed)return {available:false,reason:'closed',replayEligible:false}
      for(const state of loads)check(state)
      return {available:true,loads:loads.length,
        matchedArtifacts:loads.filter(({record:r})=>r.expectedDigest&&r.noAtimeRead&&r.loadReturned&&r.unchanged&&r.mappedIdentity&&!r.reason).length,
        reasons:loads.flatMap(({record:r})=>r.reason?[r.reason]:[]),replayEligible:false}
    },
    report() {
      for(const state of loads)check(state,{mapping:true})
      return {...this.snapshot(),records:loads.map(({record})=>({...record})),
        limitations:['file identity at load and checkpoints does not exclude transient concurrent writes',
          'mapped inode identity does not prove private memory pages were never modified',
          'digest matches do not establish native capabilities or authorize replay']}
    },
    close() {
      if(closed)return
      closed=true
      if(process.dlopen===wrapper)process.dlopen=original
      for(const state of loads)if(state.fd!==null){try{closeSync(state.fd)}catch{};state.fd=null}
    },
  }
}

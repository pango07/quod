// Diagnostic only. Loader evidence describes one execution, never an addon
// capability certificate. A post-run file digest is not a load-time digest.
import fs from 'node:fs'
import path from 'node:path'
import crypto from 'node:crypto'

export function readNativeSymbolAudit(directory,{pid:onlyPid=null}={}) {
  const files=fs.readdirSync(directory).filter(n=>/^loader-\d+\.tsv$/.test(n)).sort()
  if(!files.length)throw Error('no native loader audit files')
  let events=[]
  for(const name of files) {
    const text=fs.readFileSync(path.join(directory,name),'utf8')
    if(!text.endsWith('\n'))throw Error('truncated native loader audit: '+name)
    for(const line of text.slice(0,-1).split('\n')) {
      const parts=line.split('\t')
      if(parts.length!==6 || !['version','object','bind'].includes(parts[0]) ||
        !/^\d+$/.test(parts[1]) || !/^\d+$/.test(parts[2]))throw Error('invalid native loader audit row')
      const [kind,pid,flags,...fields]=parts
      events.push({kind,pid:Number(pid),flags:Number(flags),fields:fields.map(decodeURIComponent)})
    }
  }
  if(onlyPid!==null) {
    if(!Number.isSafeInteger(onlyPid)||onlyPid<=0)throw Error('invalid native audit PID')
    events=events.filter(e=>e.pid===onlyPid)
    if(!events.length)throw Error('no native audit events for requested process')
  }
  const pids=[...new Set(events.map(e=>e.pid))]
  for(const pid of pids)if(!events.some(e=>e.pid===pid&&e.kind==='version'&&e.fields[0]==='quod-native-symbol-audit-v1'))
    throw Error('missing loader handshake for process '+pid)
  const objectPaths=[...new Set(events.filter(e=>e.kind==='object').map(e=>e.fields[0]))]
  const addons=objectPaths.filter(p=>p.endsWith('.node')).map(file=>{
    let postRunSha256=null,postRunBytes=null,postRunHashError=null
    try {
      const bytes=fs.readFileSync(file);postRunBytes=bytes.length
      postRunSha256=crypto.createHash('sha256').update(bytes).digest('hex')
    }catch{postRunHashError='loaded path could not be read after execution'}
    const bindings=events.filter(e=>e.kind==='bind'&&e.fields[0]===file)
    const unique=rows=>[...new Set(rows)].sort()
    return {path:file,postRunSha256,postRunBytes,postRunHashError,
      pids:unique(events.filter(e=>e.kind==='object'&&e.fields[0]===file).map(e=>e.pid)),
      // LA_SYMB_DLSYM is 0x08 in glibc link.h. The live native fixture
      // independently checks successful dlsym resolution is classified here.
      dynamicSymbols:unique(bindings.filter(e=>e.flags&8).map(e=>e.fields[2])),
      linkedSymbols:unique(bindings.filter(e=>!(e.flags&8)).map(e=>e.fields[2])),
      providers:unique(bindings.map(e=>e.fields[1])),
      bindings:unique(bindings.map(e=>JSON.stringify({symbol:e.fields[2],provider:e.fields[1],dynamic:!!(e.flags&8)}))).map(JSON.parse)}
  })
  return {schema:1,replayEligible:false,files:files.length,processes:pids.length,
    events:events.length,addons,
    limitations:['successful symbol bindings, not invocation counts',
      'failed lookups, direct/static calls and raw memory access are not observed',
      'post-run digests do not prove loaded bytes were immutable',
      'matching names or digests alone does not certify a native addon']}
}

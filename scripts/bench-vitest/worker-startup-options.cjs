// Diagnostic launch contract. Unknown startup modes run no boundary experiment;
// this does not constrain production Vitest or authorize replay.
'use strict'
const path=require('node:path')
const fs=require('node:fs')
const {createHash}=require('node:crypto')
const preload=path.join(__dirname,'worker-boundary-preload.cjs')
const bootstrap=path.join(__dirname,'worker-module-metadata-bootstrap.mjs')
const warningHash='a5012da3d2027007781baecda154d343d8c7ad2327b79bec65b1b60a41f42dd1'
const switches=new Set(['--experimental-import-meta-resolve','--experimental-vm-modules','--no-warnings','--expose-gc'])
function validateWorkerLaunch(execArgv,env,{installed=false}={}) {
  if(!Array.isArray(execArgv)||execArgv.some(a=>typeof a!=='string'))throw Error('worker startup arguments unavailable')
  // A later process.execArgv snapshot omits NODE_OPTIONS. Do not silently
  // discard environment preloads or accept an unparsed startup language.
  for(const key of ['NODE_OPTIONS','LD_PRELOAD','LD_AUDIT','DYLD_INSERT_LIBRARIES'])
    if(env[key]?.trim())throw Error('unsupported worker startup environment: '+key)
  let ownPreloads=0,imports=0,warningPreloads=0
  for(let i=0;i<execArgv.length;i++) {
    const arg=execArgv[i]
    if(switches.has(arg))continue
    if(arg==='--conditions'||arg==='-C') {
      if(!execArgv[++i]||execArgv[i].startsWith('-'))throw Error('invalid worker condition option')
      continue
    }
    if(arg.startsWith('--conditions=')&&arg.length>'--conditions='.length)continue
    if(arg==='--require'||arg==='-r') {
      const file=execArgv[++i]
      if(installed&&file===preload&&i===1){ownPreloads++;continue}
      if(!file||!path.isAbsolute(file))throw Error('worker preload must have an absolute path')
      if(createHash('sha256').update(fs.readFileSync(file)).digest('hex')!==warningHash)
        throw Error('worker preload is not the pinned warning wrapper')
      warningPreloads++;continue
    }
    if(arg==='--import'&&execArgv[++i]===bootstrap){imports++;continue}
    // In particular, snapshots, custom loaders, config files and inspector
    // startup modes must not be accepted because their later state looks clean.
    throw Error('unsupported worker startup option: '+arg.split('=')[0])
  }
  if(ownPreloads!==(installed?1:0)||imports>1||warningPreloads>1)throw Error('ambiguous worker startup order')
  return {validated:true,ownPreloads,imports,warningPreloads,completeStartupIntegrity:false,replayEligible:false}
}
function workerStartupArguments(execArgv,env) {
  validateWorkerLaunch(execArgv,env)
  return ['--require',preload,...execArgv]
}
module.exports={validateWorkerLaunch,workerStartupArguments,preload}

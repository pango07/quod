// First synchronous preload, before the pinned Vitest warning wrapper and
// every ESM import. All imports below are owned diagnostic code or builtins.
'use strict'
const cachedBefore=Object.keys(require.cache).filter(file=>file!==__filename)
const firstRequire=Array.isArray(process._preload_modules)&&process._preload_modules[0]===__filename
const preloadParent=module.parent?.id==='internal/preload'
const {validateWorkerLaunch}=require('./worker-startup-options.cjs')
if(process.versions.node!=='24.18.0'||typeof process.send!=='function')throw Error('worker startup requires pinned isolated Node')
const launch=validateWorkerLaunch(process.execArgv,process.env,{installed:true})
if(!firstRequire||!preloadParent||cachedBefore.length)throw Error('worker observer was not the first CommonJS preload')
const {createWorkerStartup}=require('./worker-startup-probe.mjs')
module.exports=createWorkerStartup({launch,firstRequire,preloadParent,priorCachedModules:cachedBefore.length})

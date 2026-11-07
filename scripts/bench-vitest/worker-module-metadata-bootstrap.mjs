// Worker-only diagnostic; never enables replay or changes ledger keys.
import fs from 'node:fs'
import path from 'node:path'
import {createHash} from 'node:crypto'
import {createRequire} from 'node:module'
import {pathToFileURL} from 'node:url'
import {inspectPromiseObservers} from './promise-observers-probe.mjs'
import {observeModuleMetadata} from './worker-module-metadata-probe.mjs'
import {observeTransportMetadata} from './worker-transport-metadata-probe.mjs'
import {observeModuleEvaluation} from './worker-module-probe.mjs'
import {observeStartConfiguration} from './worker-start-input-probe.mjs'
import {nativeOwnerGuard} from './native-owner-guard.mjs'

const directory=process.env.QUOD_WORKER_MODULE_METADATA_REPORT_DIR
if(!directory||!path.isAbsolute(directory)||typeof process.send!=='function')throw Error('metadata probe requires an isolated worker')
const combined=process.env.QUOD_WORKER_BOUNDARY_PROBE==='1'
const scalarObservation=combined?(process.env.QUOD_WORKER_SCALAR_OBSERVATION??'accessor'):'accessor'
if(!['accessor','native-data'].includes(scalarObservation))throw Error('unknown scalar observation mode')
const files=[],unknown=[],send=process.send,write=fs.writeFileSync,apply=Reflect.apply
let project=null
const progressDirectory=path.join(directory,'progress')
if(combined)fs.mkdirSync(progressDirectory,{recursive:true})
const startup=combined?createRequire(import.meta.url)('./worker-boundary-preload.cjs'):null
const registry=startup?.registry??inspectPromiseObservers({proxyAccess:true,hookAccess:true})
const require=createRequire(path.join(process.cwd(),'package.json'))
const entry=require.resolve('vite/module-runner')
const entrySha256=createHash('sha256').update(fs.readFileSync(entry)).digest('hex')
if(entrySha256!=='9e3f0bc4ea6d01e55bb9a73ee3666c750e139340bb84fa46e35f92085f2535b7')throw Error('review module graph interface before changing Vite')
const {EvaluatedModules}=await import(pathToFileURL(entry).href)
const ownerGuard=combined?nativeOwnerGuard():null
const observer=observeModuleMetadata(EvaluatedModules,{sourceForScript:registry.sourceForScript,observation:combined?scalarObservation:'proxy',ownerGuard})
const vitestPackage=require.resolve('vitest/package.json')
const transportEntry=path.join(path.dirname(vitestPackage),'dist/chunks/index.1_nbEjJY.js')
const transportSha256=createHash('sha256').update(fs.readFileSync(transportEntry)).digest('hex')
if(transportSha256!=='9868c71ba3a06ca6f9890c60a9e6b298762b0fca74b09595a8f76340d3f92fe9')throw Error('review transport interface before changing Vitest')
const expectedReaderSources=[entry,transportEntry].map(file=>({url:pathToFileURL(file).href,
  sha256:createHash('sha256').update(fs.readFileSync(file,'utf8'),'utf16le').digest('hex')}))
const {b:VitestTransport}=await import(pathToFileURL(transportEntry).href)
const transport=observeTransportMetadata(VitestTransport,observer,{delivery:combined?'consumer':'reaction'})
const {observeConfig}=combined?createRequire(import.meta.url)('./worker-config-probe.cjs'):{}
let evaluation=null,config=null,seed=null
if(combined) {
  const {VitestModuleEvaluator}=await import(pathToFileURL(path.join(path.dirname(vitestPackage),'dist/module-evaluator.js')).href)
  evaluation=observeModuleEvaluation(VitestModuleEvaluator,{metadataForNode:observer.metadataForNode})
}
const output=path.join(directory,`${process.pid}.json`)
function prepareWorkerMessage(message) {
  transport.received(message)
  if(message?.__vitest_worker_request__!==true)return true
  if(message.type==='start') {
    if(project!==null)unknown.push('reused worker')
    project=message.context?.config?.name??''
    if(combined) {
      config=observeStartConfiguration(message.context.config,
        {sourceForScript:registry.sourceForScript,ownerGuard,observation:scalarObservation,observeConfig})
      seed=config.seed
      if(config.value!==message.context.config)message.context.config=config.value
    }
  }
  if(message.type==='run'||message.type==='collect')files.push(...message.context.files.map(f=>f.filepath))
  return true
}
function observeWorkerMessage(message) {
  // Combined mode has already prepared the decoded object before dispatch.
  // This redundant transport notification is idempotent and retains the
  // exact diagnostic binding used by the transport continuation observer.
  if(combined)transport.received(message)
  else prepareWorkerMessage(message)
}
if(combined) {
  startup.handoff({transport,prepare:prepareWorkerMessage,
    onCheckpoint:checkpoint=>write(path.join(progressDirectory,`${process.pid}.json`),
      JSON.stringify({project,files,checkpoint})+'\n')})
  registry.captureTransportContinuation(pathToFileURL(entry).href)
}else process.prependListener('message',observeWorkerMessage)
process.send=function(...args) {
  const message=args[0]
  transport.sent(message)
  if(message?.__vitest_worker_response__===true&&['testfileFinished','stopped'].includes(message.type)) {
    try{write(output,JSON.stringify({replayEligible:false,project,files,unknown,entrySha256,transportSha256,expectedReaderSources,
      boundary:combined?{scalarObservation,startup:startup.report(),messageDispatch:registry.messageDispatch(),ipcDelivery:registry.ipcDelivery(),transportContinuation:registry.transportContinuation(),evaluation:evaluation.report(),compilation:registry.compiledScriptReport(),config:config?.report()??null,seed:seed?.report()??null}:null,
      metadata:observer.report(),transport:transport.report(),native:{opaque:registry.opaqueAccess(),inspector:registry.inspectorAccess(),proxy:registry.proxyAccess(),hooks:registry.hookAccess(),descriptors:registry.descriptorAccess(),identity:registry.nativeIdentity(),timing:registry.inspectorTiming()}})+'\n')}
    catch{/* absent evidence fails the parent check */}
  }
  return apply(send,this,args)
}

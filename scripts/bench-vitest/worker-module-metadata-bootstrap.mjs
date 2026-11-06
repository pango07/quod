// Worker-only diagnostic; never enables replay or changes ledger keys.
import fs from 'node:fs'
import path from 'node:path'
import {createHash} from 'node:crypto'
import {createRequire} from 'node:module'
import {pathToFileURL} from 'node:url'
import {inspectPromiseObservers} from './promise-observers-probe.mjs'
import {observeModuleMetadata} from './worker-module-metadata-probe.mjs'
import {observeTransportMetadata} from './worker-transport-metadata-probe.mjs'

const directory=process.env.QUOD_WORKER_MODULE_METADATA_REPORT_DIR
if(!directory||!path.isAbsolute(directory)||typeof process.send!=='function')throw Error('metadata probe requires an isolated worker')
const registry=inspectPromiseObservers({proxyAccess:true,hookAccess:true})
const require=createRequire(path.join(process.cwd(),'package.json'))
const entry=require.resolve('vite/module-runner')
const entrySha256=createHash('sha256').update(fs.readFileSync(entry)).digest('hex')
if(entrySha256!=='9e3f0bc4ea6d01e55bb9a73ee3666c750e139340bb84fa46e35f92085f2535b7')throw Error('review module graph interface before changing Vite')
const {EvaluatedModules}=await import(pathToFileURL(entry).href)
const observer=observeModuleMetadata(EvaluatedModules,{sourceForScript:registry.sourceForScript})
const vitestPackage=require.resolve('vitest/package.json')
const transportEntry=path.join(path.dirname(vitestPackage),'dist/chunks/index.1_nbEjJY.js')
const transportSha256=createHash('sha256').update(fs.readFileSync(transportEntry)).digest('hex')
if(transportSha256!=='9868c71ba3a06ca6f9890c60a9e6b298762b0fca74b09595a8f76340d3f92fe9')throw Error('review transport interface before changing Vitest')
const expectedReaderSources=[entry,transportEntry].map(file=>({url:pathToFileURL(file).href,
  sha256:createHash('sha256').update(fs.readFileSync(file,'utf8'),'utf16le').digest('hex')}))
const {b:VitestTransport}=await import(pathToFileURL(transportEntry).href)
const transport=observeTransportMetadata(VitestTransport,observer)
const files=[],unknown=[],send=process.send,write=fs.writeFileSync,apply=Reflect.apply
const output=path.join(directory,`${process.pid}.json`)
let project=null
process.prependListener('message',message=>{
  transport.received(message)
  if(message?.__vitest_worker_request__!==true)return
  if(message.type==='start') {
    if(project!==null)unknown.push('reused worker')
    project=message.context?.config?.name??''
  }
  if(message.type==='run'||message.type==='collect')files.push(...message.context.files.map(f=>f.filepath))
})
process.send=function(...args) {
  const message=args[0]
  transport.sent(message)
  if(message?.__vitest_worker_response__===true&&['testfileFinished','stopped'].includes(message.type)) {
    try{write(output,JSON.stringify({replayEligible:false,project,files,unknown,entrySha256,transportSha256,expectedReaderSources,
      metadata:observer.report(),transport:transport.report(),native:{opaque:registry.opaqueAccess(),inspector:registry.inspectorAccess(),proxy:registry.proxyAccess(),hooks:registry.hookAccess()}})+'\n')}
    catch{/* absent evidence fails the parent check */}
  }
  return apply(send,this,args)
}

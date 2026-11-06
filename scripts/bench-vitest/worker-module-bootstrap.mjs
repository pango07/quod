import fs from 'node:fs'
import path from 'node:path'
import {createRequire} from 'node:module'
import {pathToFileURL} from 'node:url'
import {observeCompiledScripts,observeModuleEvaluation} from './worker-module-probe.mjs'

const directory=process.env.QUOD_WORKER_MODULE_REPORT_DIR
if(!directory||!path.isAbsolute(directory)||typeof process.send!=='function')throw Error('module probe requires an isolated worker')
const compiled=observeCompiledScripts()
const require=createRequire(path.join(process.cwd(),'package.json'))
const packageFile=require.resolve('vitest/package.json'),pkg=JSON.parse(fs.readFileSync(packageFile))
if(pkg.version!=='5.0.0')throw Error('review evaluator interface before changing Vitest')
const {VitestModuleEvaluator}=await import(pathToFileURL(path.join(path.dirname(packageFile),'dist/module-evaluator.js')).href)
const evaluation=observeModuleEvaluation(VitestModuleEvaluator)
let project=null
const files=[],unknown=[],send=process.send,write=fs.writeFileSync,apply=Reflect.apply
const output=path.join(directory,`${process.pid}.json`)
process.prependListener('message',message=>{
  if(message?.__vitest_worker_request__!==true)return
  if(message.type==='start') {
    if(project!==null)unknown.push('reused worker')
    project=message.context?.config?.name??''
  }
  if(message.type==='run'||message.type==='collect')files.push(...message.context.files.map(f=>f.filepath))
})
process.send=function(...args) {
  const message=args[0]
  if(message?.__vitest_worker_response__===true&&['testfileFinished','stopped'].includes(message.type)) {
    try{write(output,JSON.stringify({replayEligible:false,project,files,unknown,evaluation:evaluation.report(),compilation:compiled.report()})+'\n')}
    catch{/* missing evidence fails the parent check */}
  }
  return apply(send,this,args)
}

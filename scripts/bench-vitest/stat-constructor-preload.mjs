// Remote diagnostic for the Vitest CLI's main process only. No replay policy.
import fs from 'node:fs'
import path from 'node:path'
import {createStatConstructorProbe} from './stat-constructor-probe.mjs'

const entry=path.resolve(process.argv[1]||'')
if(entry.endsWith('/node_modules/vitest/vitest.mjs')) {
  const output=process.env.QUOD_STAT_CONSTRUCTOR_REPORT
  if(!output||!path.isAbsolute(output))throw Error('absolute constructor report path required')
  const probe=createStatConstructorProbe({captureConsumers:true})
  process.once('exit',()=>{
    const report=probe.report()
    report.scope='Vitest CLI main process, from preload through this exit listener'
    report.limitations.push('workers, earlier startup and later exit listeners are outside this diagnostic')
    fs.writeFileSync(output,JSON.stringify(report,null,2)+'\n',{flag:'wx'})
    probe.close()
  })
}

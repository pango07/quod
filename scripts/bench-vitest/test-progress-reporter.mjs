// Parent-process diagnostics only. Incremental events survive a timed-out
// observed phase; they never replace the complete JSON test-state report.
// Pinned reporter API: https://vitest.dev/api/advanced/reporters
import path from 'node:path'

export function testProgressReporter(root,write,{now=()=>performance.now(),eventLimit=40000}={}) {
  if(!Number.isSafeInteger(eventLimit)||eventLimit<1)throw Error('invalid progress event limit')
  const started=now()
  let events=0,ready=0,results=0,ended=false,overflow=false,failure=null
  function emit(event,fields) {
    if(failure||overflow)return
    if(events>=eventLimit){overflow=true;return}
    try{write({event,sequence:events,elapsedMs:now()-started,...fields});events++}
    catch{failure='progress write failed'}
  }
  function identity(task) {
    if(typeof task?.id!=='string'||typeof task.fullName!=='string'||
      typeof task.module?.moduleId!=='string'||typeof (task.project?.config?.name??'')!=='string')
      throw Error('unsupported progress task')
    return {id:task.id,project:task.project?.config?.name??'',
      file:path.relative(root,task.module.moduleId),name:task.fullName}
  }
  const reporter={
    onTestCaseReady(task) {
      try{const fields=identity(task);ready++;emit('ready',fields)}
      catch{failure='unsupported progress task'}
    },
    onTestCaseResult(task) {
      try {
        const fields=identity(task),state=task.result().state
        if(!['passed','failed','skipped'].includes(state))throw Error('unfinished progress result')
        results++;emit('result',{...fields,state})
      }catch{failure='unsupported progress result'}
    },
    onTestRunEnd(modules,errors,reason) {
      ended=true
      if(!Array.isArray(modules)||!Array.isArray(errors)||!['passed','failed','interrupted'].includes(reason)) {
        failure='unsupported progress completion';return
      }
      emit('run-end',{modules:modules.length,unhandledErrors:errors.length,reason})
    },
  }
  return {reporter,report:()=>({events,ready,results,ended,overflow,failure,eventLimit,
    replayEligible:false,scope:'Reporter callback receipt order and elapsed time; complete JSON results remain authoritative'})}
}

// Diagnostic only. Observe Vite's existing await continuation, without adding
// a Promise reaction. Earlier consumers and initial runtime integrity remain
// unproven; matching invocation counts is not a per-Promise ownership proof.
import {createHash,randomBytes} from 'node:crypto'

const sourceHash='9e3f0bc4ea6d01e55bb9a73ee3666c750e139340bb84fa46e35f92085f2535b7'
const lineNumber=568
export function observeTransportContinuation(post,{entryUrl,callback}) {
  const group='quod-transport-continuation-'+randomBytes(16).toString('hex')
  const errors=[],sources=new Map()
  let breakpointId=null,installed=false,closed=false,entries=0,delivered=0,groupsCreated=0,groupsReleased=0
  let failures=0
  function fail(reason){failures++;if(errors.length<20)errors.push(reason)}
  function checkSource(location) {
    if(location.lineNumber!==lineNumber)throw Error('transport continuation moved from pinned line')
    if(!sources.has(location.scriptId)) {
      if(sources.size>=16)throw Error('transport continuation source capacity exceeded')
      const source=post('Debugger.getScriptSource',{scriptId:location.scriptId}).scriptSource
      if(typeof source!=='string'||createHash('sha256').update(source).digest('hex')!==sourceHash)
        throw Error('transport continuation source differs from pinned Vite')
      sources.set(location.scriptId,{line:location.lineNumber,column:location.columnNumber,sha256:sourceHash})
    }
  }
  function evaluate(frame,expression,eventGroup) {
    const r=post('Debugger.evaluateOnCallFrame',{callFrameId:frame.callFrameId,expression,objectGroup:eventGroup,silent:true,generatePreview:false})
    if(r.exceptionDetails||!r.result)throw Error('transport continuation local unavailable')
    return r.result
  }
  try {
    if(process.versions.node!=='24.18.0'||typeof entryUrl!=='string'||!entryUrl.startsWith('file:'))
      throw Error('transport continuation requires pinned Node and a file module')
    if(callback?.type!=='function'||callback.subtype==='proxy'||!callback.objectId)throw Error('transport callback handle unavailable')
    const r=post('Debugger.setBreakpointByUrl',{url:entryUrl,lineNumber,columnNumber:0,condition:'name === "fetchModule"'})
    breakpointId=r.breakpointId
    if(!breakpointId||r.locations?.length!==1)throw Error('transport continuation is not uniquely loaded')
    checkSource(r.locations[0]);installed=true
  }catch(e){fail(e.message)}
  return {
    handlePause(params) {
      if(!breakpointId||!params.hitBreakpoints?.includes(breakpointId))return false
      const eventGroup=group+'-'+(++groupsCreated)
      try {
        if(!installed||closed)throw Error('transport continuation is unavailable')
        const frame=params.callFrames?.[0]
        if(!frame||params.hitBreakpoints.length!==1)throw Error('ambiguous transport continuation pause')
        checkSource(frame.location)
        if(evaluate(frame,'name',eventGroup).value!=='fetchModule')throw Error('unexpected transport continuation method')
        entries++
        const value=evaluate(frame,'result',eventGroup)
        if(!value.objectId||value.type!=='object'||value.subtype==='proxy')throw Error('transport continuation result is not an ordinary object')
        const r=post('Runtime.callFunctionOn',{objectId:callback.objectId,
          functionDeclaration:'function(value){return this(value)}',arguments:[{objectId:value.objectId}],
          objectGroup:eventGroup,returnByValue:true,silent:true})
        if(r.exceptionDetails||r.result?.value!==true)throw Error('transport delivery callback did not accept the result')
        delivered++
      }catch(e){fail(e.message)}
      finally {
        try{post('Runtime.releaseObjectGroup',{objectGroup:eventGroup});groupsReleased++}
        catch{fail('transport continuation handles could not be released')}
      }
      return true
    },
    report:()=>({available:installed&&!closed&&!failures,installed,closed,entries,delivered,groupsCreated,groupsReleased,
      sources:[...sources.values()],failures,errors:[...errors],addsPromiseReaction:false,replayEligible:false,completeOwnership:false,
      limitation:'Only the pinned consumer continuation is observed; earlier consumers, rejected Promises, per-Promise pairing and initial runtime integrity remain unproven'}),
    close() {
      if(closed)return
      closed=true
      if(breakpointId)try{post('Debugger.removeBreakpoint',{breakpointId})}catch{fail('transport continuation breakpoint removal failed')}
    },
  }
}

// Diagnostic only. Correlate real Buffer identities and decoder/JSON call sites.
// Text equality is NOT a data-flow proof and never grants replay eligibility.
import path from 'node:path'
import { getCallSites } from 'node:util'
import { createHash } from 'node:crypto'

export function instrumentAsyncReads(source) {
  const changes = [
    ["const file = (a) => r.add('f', a[0])", "const file = (a) => { r.observeFileCall?.(a); r.add('f', a[0]) }"],
    ["(a, v) => pjText(r, a[0], v)", "(a, v) => { r.observeFileResult?.(a, v); pjText(r, a[0], v) }"],
    ["const v = Reflect.apply(parse, this, arguments)", "const v = Reflect.apply(parse, this, arguments)\n      r.observeJsonParse?.(text)"],
  ]
  for (const [before, after] of changes) {
    if (source.split(before).length !== 2) throw Error(`async probe injection point changed: ${before}`)
    source = source.replace(before, after)
  }
  return source
}

export function observeAsyncReads(recorder, root, manifests) {
  recorder.unobservable.add('diagnostic async-read instrumentation: never reuse')
  const calls = new WeakMap(), buffers = new WeakMap(), texts = new Map(), reads = []
  const wanted = new Set(manifests)
  const original = Buffer.prototype.toString
  const nativeString = String
  let inspecting = false
  const frames = () => getCallSites(16, { sourceMap: false }).map(f => ({
    scriptId:f.scriptId, file:f.scriptName, function:f.functionName, line:f.lineNumber, column:f.columnNumber,
  }))
  const digest = text => createHash('sha256').update(text).digest('hex')
  recorder.observeFileCall = args => {
    if (inspecting || typeof args[0] !== 'string' || !wanted.has(path.resolve(args[0]))) return
    const read = { id:reads.length, path:path.relative(root,path.resolve(args[0])), stringNativeAtRead:String===nativeString,
      callback:typeof args.at(-1)==='function', call:frames(), result:null, conversions:[], parses:[] }
    calls.set(args, read)
    reads.push(read)
  }
  recorder.observeFileResult = (args, value) => {
    const read = calls.get(args)
    if (!read) return
    read.result = Buffer.isBuffer(value) ? 'buffer' : typeof value
    if (Buffer.isBuffer(value)) buffers.set(value, read)
    // Keep primitive-string returns visible without granting identity proof.
    else if (typeof value === 'string') {
      const key=digest(value)
      if (!texts.has(key)) texts.set(key,[])
      texts.get(key).push(read)
    }
  }
  function decode(...args) {
    const value = Reflect.apply(original, this, args)
    const read = buffers.get(this)
    if (read && !inspecting) {
      inspecting = true
      try {
        read.conversions.push({ args:args.map(x=>typeof x==='string'||typeof x==='number'?x:typeof x), call:frames() })
        const key=digest(value)
        if (!texts.has(key)) texts.set(key,[])
        if (!texts.get(key).includes(read)) texts.get(key).push(read)
      } finally { inspecting = false }
    }
    return value
  }
  Buffer.prototype.toString = decode
  recorder.observeJsonParse = text => {
    if (inspecting || typeof text !== 'string') return
    inspecting = true
    try {
      const candidates=texts.get(digest(text))
      if (!candidates) return
      const call=frames()
      for (const read of candidates) read.parses.push({ call, candidateReadIds:candidates.map(r=>r.id), proven:false })
    } finally { inspecting = false }
  }
  return {
    report: () => ({ replayEligible:false, attribution:'Buffer identity for decoding; text-equality candidates for parsing, not proof', reads }),
    close() { if (Buffer.prototype.toString === decode) Buffer.prototype.toString = original },
  }
}

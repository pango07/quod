// Measurement only: the profile harness marks every affected file
// unrecordable. This code is not embedded in the shipped recorder.
import { runInNewContext } from 'node:vm'

// A private realm owns Error and its formatter, leaving the application's
// Error.prepareStackTrace untouched. V8's structured stack includes async
// causal parents; util.getCallSites is not substituted for that stack.
const nativeCapture = runInNewContext(`(() => {
  Error.stackTraceLimit = Infinity;
  Error.prepareStackTrace = (_, frames) => frames.map(f => ({
    file: f.getFileName(), name: f.getFunctionName(),
    native: f.isNative(), eval: f.isEval(), async: f.isAsync(),
    promise: f.getPromiseIndex(), line: f.getLineNumber(), column: f.getColumnNumber()
  }));
  return () => new Error().stack;
})()`, Object.create(null), { filename: import.meta.url })

export function nativeCaller(npm, self, helpers, resolvers, children, npmHelpers) {
  let frames
  try { frames = nativeCapture() } catch { return { tool: false, frame: 'native stack unavailable' } }
  let tool = null, spawn = false
  for (const f of frames) {
    const file = f.file ?? ''
    if (f.eval) return { tool: false, frame: `eval: ${file}` }
    if (file === import.meta.url || file === self || file.includes('/.quod-vitest/')) continue
    if (file.startsWith('node:child_process') || file.startsWith('node:internal/child_process')) spawn = true
    if (file.startsWith('node:')) continue
    if (!file) {
      if (f.native || (f.async && Number.isInteger(f.promise))) continue
      return { tool: false, frame: `unknown: ${f.name ?? '?'}` }
    }
    const frame = `${file}:${f.line}:${f.column}`
    if (spawn && !tool && children.some(c => file.includes(c))) return { tool: true, child: true, frame }
    if (helpers.some(h => file.includes(h)) || resolvers.some(h => file.includes(h)) || (npm && npmHelpers.some(h => file.includes(h)))) {
      tool ??= frame
      continue
    }
    return { tool: false, frame }
  }
  return tool ? { tool: true, frame: tool } : { tool: false, frame: '?' }
}

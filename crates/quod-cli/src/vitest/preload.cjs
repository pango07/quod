// quod vitest: preloaded into every recorded worker (execArgv --require),
// before vitest or any library loads, so no code can hold on to the
// original Function.prototype.toString. A test file that reads function
// source gets no function-level narrowing: a change to the body of a
// function it never ran could still change what it printed or compared.
// The wrapper reads as the native function (its own source text, name and
// length are the native one's).
'use strict'
if (!globalThis.__quod_toString) {
  const state = { used: false }
  const fts = Function.prototype.toString
  const wrapped = {
    toString() {
      if (this === wrapped) return fts.call(fts)
      state.used = true
      return fts.call(this)
    },
  }.toString
  Object.defineProperty(Function.prototype, 'toString', { value: wrapped, writable: true, configurable: true })
  Object.defineProperty(globalThis, '__quod_toString', { value: state, enumerable: false })
}

// QUOD_VITEST_COV_EARLY=1: V8 precise coverage from here, while the heap is
// small (starting it later walks the heap and gives every live function a
// feedback vector); setup.mjs takes the session over.
if (process.env.QUOD_VITEST_COV_EARLY === '1' && !globalThis.__quod_cov) {
  try {
    const inspector = require('node:inspector')
    const s = new inspector.Session()
    s.connect()
    s.post('Profiler.enable')
    s.post('Profiler.startPreciseCoverage', { callCount: false, detailed: false })
    Object.defineProperty(globalThis, '__quod_cov', { value: s, enumerable: false })
  } catch {}
}

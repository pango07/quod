// quod vitest: preloaded (NODE_OPTIONS --import) into a node child process
// a recorded test file started (record.mjs traceChild). It records what the
// child reads, as setup.mjs does for the test file's worker, and at exit
// writes its dump (QUOD_VITEST_CHILD), which the test file's record takes
// in with the dumps of the child's own children. A child that leaves none
// (killed, still running) makes the test file unobservable.
import path from 'node:path'
import { createRecorder, patchEnv, patchEscapes, patchFs, patchLoader, scratch, syncESM, writeJson } from './record.mjs'

const out = process.env.QUOD_VITEST_CHILD
if (out && !globalThis.__quod_vitest__) {
  const r = createRecorder({
    stateDir: process.env.QUOD_VITEST_STATE ?? path.dirname(out),
    codeDir: import.meta.dirname,
    mode: 'worker',
    wsRoot: process.env.QUOD_VITEST_WS || null,
  })
  globalThis.__quod_vitest__ = r
  r.trace = { codeDir: import.meta.dirname, base: out.replace(/\.json$/, ''), n: 0 }
  patchFs(r, { sync: false })
  patchEnv(r)
  patchLoader(r)
  patchEscapes(r, { sync: false })
  syncESM()
  // The program itself (node reads the main script before any hook runs).
  if (process.argv[1]) r.add('f', path.resolve(process.argv[1]))
  process.on('exit', () => {
    r.on = false
    try {
      writeJson(out, { keys: [...r.keys], writes: scratch(r), unobservable: [...r.unobservable], children: r.children, net: !!r.net })
    } catch {}
  })
}

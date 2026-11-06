// Worker-only diagnostic. Start observation before the Vitest worker's
// entry module loads, then observe config passed over its real IPC channel.
import { createRequire } from 'node:module'
import { inspectPromiseObservers } from './promise-observers-probe.mjs'
const registry = inspectPromiseObservers()
const { installProcessProbe } = createRequire(import.meta.url)('./worker-config-probe.cjs')
installProcessProbe(registry)

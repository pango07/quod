// Feasibility evidence only: observing loads is not an absence-of-escape
// proof. Preserve the original loader's return value and thrown exception.
'use strict'
const fs = require('node:fs')
const path = require('node:path')
const { threadId } = require('node:worker_threads')
const directory = process.env.QUOD_WASI_LOAD_REPORT
if (!directory || !path.isAbsolute(directory)) throw Error('WASI diagnostic report directory is required')
const output = path.join(directory, `${process.pid}-${threadId}.json`)
const write = fs.writeFileSync, apply = Reflect.apply, original = process.dlopen
const events = []
const initial = process.report.getReport().sharedObjects.filter(file => file.endsWith('.node'))
function save(exited = false) {
  write(output, JSON.stringify({ pid: process.pid, threadId, initial, events, exited,
    wasiModules: Object.keys(require.cache).filter(file => /rolldown-binding\.wasi\.cjs$/.test(file)),
    nativeModules: Object.keys(require.cache).filter(file => file.endsWith('.node')) }) + '\n')
}
save()
process.dlopen = function (...args) {
  const event = { path: typeof args[1] === 'string' ? args[1] : '<non-string>', completed: false }
  events.push(event)
  save()
  try {
    const result = apply(original, this, args)
    event.completed = true
    return result
  } finally { save() }
}
process.once('exit', () => save(true))

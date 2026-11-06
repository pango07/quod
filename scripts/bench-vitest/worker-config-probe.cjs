'use strict'
// Diagnostic only. A Proxy can itself be observed and native code can
// inspect its target without traps. These observations never permit replay.
const fs = require('node:fs')
const path = require('node:path')
const { types } = require('node:util')
const own = Object.hasOwn
const getDescriptor = Reflect.getOwnPropertyDescriptor
const getPrototype = Reflect.getPrototypeOf
const apply = Reflect.apply
const write = fs.writeFileSync

function observeConfig(config) {
  const cache = new WeakMap(), events = new Map(), gaps = new Set(), stacks = []
  let overflow = false
  const segment = key => typeof key === 'symbol' ? `[symbol:${key.description ?? ''}]` : key
  function note(operation, keys) {
    const id = JSON.stringify([operation, keys])
    if (events.has(id)) events.get(id).count++
    else if (events.size < 10000) events.set(id, { operation, path: keys, count: 1 })
    else overflow = true
    if (keys.join('.') === 'sequence.seed' && stacks.length < 12) {
      stacks.push({ operation, stack: new Error('sequence.seed diagnostic').stack })
    }
  }
  function wrap(value, keys) {
    if (!value || typeof value !== 'object') return value
    if (cache.has(value)) return cache.get(value)
    if (types.isProxy(value)) { gaps.add('existing proxy'); return value }
    const p = getPrototype(value)
    if (p !== Object.prototype && p !== Array.prototype && p !== null) {
      gaps.add('non-plain object'); return value
    }
    const canReplace = (target, key) => {
      const d = getDescriptor(target, key)
      return !d || d.configurable || !own(d, 'value') || d.writable
    }
    const proxy = new Proxy(value, {
      get(target, key, receiver) {
        const next = [...keys, segment(key)]
        note('get', next)
        const v = Reflect.get(target, key, receiver)
        if (!canReplace(target, key)) {
          if (v && typeof v === 'object') gaps.add('fixed object property')
          return v
        }
        return wrap(v, next)
      },
      has(target, key) { note('has', [...keys, segment(key)]); return Reflect.has(target, key) },
      ownKeys(target) { note('keys', keys); return Reflect.ownKeys(target) },
      getOwnPropertyDescriptor(target, key) {
        note('descriptor', [...keys, segment(key)])
        const d = getDescriptor(target, key)
        if (d && own(d, 'value') && d.value && typeof d.value === 'object') {
          if (!d.configurable) { gaps.add('fixed descriptor object'); return d }
          return { ...d, value: wrap(d.value, [...keys, segment(key)]) }
        }
        return d
      },
      set(target, key, v, receiver) {
        note('set', [...keys, segment(key)])
        return Reflect.set(target, key, v, receiver)
      },
      defineProperty(target, key, d) {
        note('define', [...keys, segment(key)])
        return Reflect.defineProperty(target, key, d)
      },
      deleteProperty(target, key) {
        note('delete', [...keys, segment(key)])
        return Reflect.deleteProperty(target, key)
      },
    })
    cache.set(value, proxy)
    return proxy
  }
  const value = wrap(config, [])
  return {
    value,
    report: () => ({ replayEligible: false, proxyObservable: true, nativeReadsExcluded: false,
      overflow, gaps: [...gaps], events: [...events.values()], seedStacks: stacks }),
  }
}

module.exports = { observeConfig }

const reportDir = process.env.QUOD_WORKER_CONFIG_REPORT_DIR
if (reportDir && path.isAbsolute(reportDir) && typeof process.send === 'function') {
  const reports = [], unknown = [], originalSend = process.send
  const reportFile = path.join(reportDir, `${process.pid}.json`)
  const flush = () => {
    try {
      write(reportFile, JSON.stringify({ replayEligible: false, unknown,
        reports: reports.map(r => ({ project: r.project, files: r.files, ...r.observer.report() })) }) + '\n')
    } catch { /* missing evidence makes the parent check fail */ }
  }
  process.prependListener('message', message => {
    if (message?.__vitest_worker_request__ !== true) return
    if (message.type === 'start') {
      if (!message.context?.config || typeof message.context.config !== 'object') {
        unknown.push('missing config'); flush(); return
      }
      const project = message.context.config.name ?? ''
      const observer = observeConfig(message.context.config)
      message.context.config = observer.value
      reports.push({ project, files: [], observer })
    } else if (message.type === 'run' || message.type === 'collect') {
      const current = reports.at(-1)
      if (!current) unknown.push('run before config')
      else current.files.push(...message.context.files.map(f => f.filepath))
    }
  })
  process.send = function (...args) {
    const message = args[0]
    if (message?.__vitest_worker_response__ === true && ['testfileFinished', 'stopped'].includes(message.type)) flush()
    return apply(originalSend, this, args)
  }
}

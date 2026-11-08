// Opt-in research scheduling only. Callers must finish every SDK lookup,
// source/compiler fence and current execution/refresh decision before start().
// Tasks own their process/stream cleanup; this helper joins their complete
// returned promises. It never starts compilers, SDK operations or children.
import assert from 'node:assert/strict'

export const MANAGED_TASK_POOL_POLICY = 'bounded-original-start-order-managed-task-pool-v1-research'
const tasksMax = 128
const concurrencyMax = 4
const errorData = error => ({name: String(error?.name || 'Error').slice(0, 128),
  message: String(error?.message || error).slice(0, 2048),
  code: error?.code === undefined ? null : String(error.code).slice(0, 128)})

function reserveTasks(tasks) {
  assert(Array.isArray(tasks) && tasks.length > 0 && tasks.length <= tasksMax,
    'bounded nonempty managed task list required')
  assert(Object.getOwnPropertySymbols(tasks).length === 0 &&
    Object.getOwnPropertyNames(tasks).length === tasks.length + 1, 'dense managed task array required')
  const ids = new Set(), reserved = []
  for (let index = 0; index < tasks.length; index++) {
    const descriptor = Object.getOwnPropertyDescriptor(tasks, String(index))
    assert(descriptor?.enumerable && 'value' in descriptor, 'managed task array accessors unsupported')
    const task = descriptor.value
    assert(task && (Object.getPrototypeOf(task) === Object.prototype || Object.getPrototypeOf(task) === null) &&
      Object.getOwnPropertySymbols(task).length === 0, 'plain managed task descriptor required')
    const fields = Object.getOwnPropertyDescriptors(task)
    assert(Object.keys(fields).sort().join(',') === 'id,run' &&
      fields.id.enumerable && 'value' in fields.id && fields.run.enumerable && 'value' in fields.run,
    'managed task requires only own data id/run fields')
    const id = fields.id.value, run = fields.run.value
    assert(typeof id === 'string' && id.length > 0 && Buffer.byteLength(id) <= 256 && !id.includes('\0') &&
      !ids.has(id) && typeof run === 'function', 'unique bounded managed task id/function required')
    ids.add(id); reserved.push(Object.freeze({id, index, run}))
  }
  return reserved
}

export function createManagedTaskPool({maxConcurrency}) {
  assert(Number.isSafeInteger(maxConcurrency) && maxConcurrency >= 1 && maxConcurrency <= concurrencyMax,
    'explicit managed concurrency must be an integer from 1 through 4')
  let promise, state = 'unstarted', tasks = [], rows = [], next = 0, active = 0, maxActive = 0, joins = 0
  let startedMonoMs = null, settledMonoMs = null, failed = false, fatalReason
  const starts = [], completions = [], values = []
  const fail = reason => {if (!failed) {failed = true; fatalReason = reason}}
  const snapshot = () => ({schema: 'quod-managed-task-pool-v1-research', policy: MANAGED_TASK_POOL_POLICY,
    maxConcurrency, concurrencyMax, tasksMax, taskCount: tasks.length, state, active, maxActive, joins,
    stopped: failed, completed: state === 'settled', startedMonoMs, settledMonoMs,
    fatal: failed ? errorData(fatalReason) : null, starts: [...starts], completions: [...completions],
    unstarted: rows.filter(row => row.state === 'unstarted' || row.state === 'queued').map(row => row.id),
    tasks: rows.map(row => ({...row}))})

  async function worker() {
    // The shared index is reserved synchronously before each callback. Initial
    // workers and every subsequent dequeue therefore preserve array START
    // order, regardless of completion order. No user callback runs in between
    // the reservation and its start stamp/accounting.
    while (!failed && next < tasks.length) {
      const task = tasks[next++], row = rows[task.index]
      row.state = 'running'; row.startedMonoMs = performance.now()
      active++; maxActive = Math.max(maxActive, active); starts.push(task.id)
      try {
        values[task.index] = await task.run()
        row.state = 'fulfilled'
      } catch (reason) {
        row.state = 'rejected'; fail(reason)
      } finally {
        active--; row.settledMonoMs = performance.now(); completions.push(task.id)
      }
    }
  }

  function start(input) {
    assert.equal(state, 'unstarted', 'managed task pool is single use')
    // Copy ids/functions before reserving so caller mutations cannot replace
    // callbacks or reorder the private queue after start returns.
    const reserved = reserveTasks(input)
    tasks = reserved; rows = tasks.map(({id, index}) => ({id, index, state: 'queued',
      startedMonoMs: null, settledMonoMs: null}))
    state = 'running'; startedMonoMs = performance.now()
    // Install the handled promise before any callback can execute. Fatal task
    // rejection stops future dequeues but does not fail-fast past active tasks.
    promise = Promise.resolve().then(async () => {
      const running = Array.from({length: Math.min(maxConcurrency, tasks.length)}, () => worker())
      const settled = await Promise.allSettled(running)
      for (const result of settled) if (result.status === 'rejected') fail(result.reason)
      for (const row of rows) if (row.state === 'queued') row.state = 'unstarted'
      state = 'settled'; settledMonoMs = performance.now()
      if (failed) return {status: 'rejected', reason: fatalReason}
      return {status: 'fulfilled', value: tasks.map(task => ({id: task.id, value: values[task.index]}))}
    })
    return promise
  }
  function join() {joins++; return promise || null}
  return Object.freeze({start, join, snapshot})
}

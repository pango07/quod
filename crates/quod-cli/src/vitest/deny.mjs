// quod vitest: the egress-denied witness. A file whose record used the
// network (a connection that was not to this machine, a DNS lookup) is
// recorded on the condition that its second witness, run plain and with no
// network at all, gives the same per-test results: then the result follows
// from the recorded inputs alone, and the network was no input of it. This
// file denies the network to that witness: as a setup file in the test
// file's worker (the driver's plan.deny lists the files), and preloaded
// (NODE_OPTIONS --import) into every node child process it starts. A
// connection to this machine (loopback, a unix socket) is allowed: a test's
// own server is part of the run. Every TCP connection made in JS goes
// through net.Socket.prototype.connect (http, https, tls, undici's fetch),
// UDP through dgram, names through dns. Native code with sockets of its own
// is ruled out at recording (a networked file that loads an addon is not
// recordable), and a native child process makes a file unrecordable anyway.
import nodeModule from 'node:module'
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'

const self = fileURLToPath(import.meta.url)
const ws = globalThis.__vitest_worker__
let on = process.env.QUOD_VITEST_DENY_ALL === '1'
if (!on && ws && process.env.QUOD_VITEST_DENY_FILES) {
  try {
    const list = new Set(JSON.parse(readFileSync(process.env.QUOD_VITEST_DENY_FILES, 'utf8')))
    on = list.has((ws.current?.file ?? ws.current)?.filepath)
  } catch {}
}
if (on && !globalThis.__quod_deny__) {
  globalThis.__quod_deny__ = true
  const local = (h) => {
    if (h == null || h === '') return true
    const s = String(h).replace(/^\[|\]$/g, '')
    return s === 'localhost' || s.endsWith('.localhost') || /^127\./.test(s) || s === '::1' || s === '0.0.0.0' || s === '::' || /^::ffff:127\./.test(s)
  }
  const refuse = (host) => Object.assign(new Error(`connect ENETUNREACH ${host} (quod: this witness runs with no network)`), { code: 'ENETUNREACH', errno: -101, syscall: 'connect' })
  const notFound = (host) => Object.assign(new Error(`getaddrinfo ENOTFOUND ${host} (quod: this witness runs with no network)`), { code: 'ENOTFOUND', errno: -3008, syscall: 'getaddrinfo', hostname: host })
  const net = process.getBuiltinModule('node:net')
  const proto = net.Socket.prototype
  const connect = proto.connect
  proto.connect = function (...args) {
    let o = args[0]
    if (Array.isArray(o)) o = o[0]
    let host
    let isPath = false
    if (o && typeof o === 'object') {
      if (o.path != null) isPath = true
      host = o.host ?? 'localhost'
    } else if (typeof o === 'string' && !/^\d+$/.test(o)) isPath = true
    else host = typeof args[1] === 'string' ? args[1] : 'localhost'
    if (isPath || local(host)) return Reflect.apply(connect, this, args)
    const err = refuse(host)
    process.nextTick(() => this.destroy(err))
    return this
  }
  const dns = process.getBuiltinModule('node:dns')
  const lookup = dns.lookup
  dns.lookup = function (host, opts, cb) {
    const done = typeof opts === 'function' ? opts : cb
    if (local(host) || net.isIP(String(host))) return Reflect.apply(lookup, this, arguments)
    process.nextTick(() => done(notFound(host)))
    return {}
  }
  const plookup = dns.promises.lookup
  dns.promises.lookup = function (host, ...rest) {
    if (local(host) || net.isIP(String(host))) return Reflect.apply(plookup, this, [host, ...rest])
    return Promise.reject(notFound(host))
  }
  for (const n of Object.keys(dns)) {
    if (!/^resolve/.test(n) || typeof dns[n] !== 'function') continue
    dns[n] = function (host, ...rest) {
      const done = rest.at(-1)
      if (typeof done === 'function') process.nextTick(() => done(notFound(host)))
    }
    if (typeof dns.promises[n] === 'function') dns.promises[n] = (host) => Promise.reject(notFound(host))
  }
  for (const R of [dns.Resolver, dns.promises.Resolver]) {
    if (!R?.prototype) continue
    for (const n of Object.getOwnPropertyNames(R.prototype)) {
      if (!/^resolve/.test(n)) continue
      R.prototype[n] = function (host, ...rest) {
        const done = rest.at(-1)
        if (typeof done === 'function') process.nextTick(() => done(notFound(host)))
        else return Promise.reject(notFound(host))
      }
    }
  }
  const dgram = process.getBuiltinModule('node:dgram')
  const send = dgram.Socket.prototype.send
  dgram.Socket.prototype.send = function (...args) {
    const host = args.find((a, i) => i > 0 && typeof a === 'string')
    if (local(host)) return Reflect.apply(send, this, args)
    const done = args.at(-1)
    if (typeof done === 'function') process.nextTick(() => done(refuse(host)))
  }
  try {
    nodeModule.syncBuiltinESMExports()
  } catch {}
  // node children: denied too.
  const opt = `--import ${JSON.stringify(self)}`
  process.env.QUOD_VITEST_DENY_ALL = '1'
  if (!(process.env.NODE_OPTIONS ?? '').includes(self)) process.env.NODE_OPTIONS = `${process.env.NODE_OPTIONS ?? ''} ${opt}`.trim()
}

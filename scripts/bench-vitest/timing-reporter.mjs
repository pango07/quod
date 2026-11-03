// A vitest reporter that prints when each phase happened, in ms since the
// process started (performance.now()), plus each test module's diagnostic
// (environment setup, prepare, collect, setup, tests, imports). For
// breakdown.sh.
import { performance } from 'node:perf_hooks'
const t = () => performance.now().toFixed(0)
const out = []
const say = (...a) => out.push(a.join(' '))
export default class TimingReporter {
  onInit() {
    say('T', t(), 'init')
  }
  onTestRunStart(specs) {
    say('T', t(), 'runStart', specs.length)
  }
  onTestModuleQueued(m) {
    say('T', t(), 'queued', m.moduleId.split('/').slice(-2).join('/'))
  }
  onTestModuleCollected(m) {
    say('T', t(), 'collected', m.moduleId.split('/').slice(-2).join('/'))
  }
  onTestModuleStart(m) {
    say('T', t(), 'start', m.moduleId.split('/').slice(-2).join('/'))
  }
  onTestModuleEnd(m) {
    const d = m.diagnostic()
    say('T', t(), 'end', m.moduleId.split('/').slice(-2).join('/'),
      `env ${d.environmentSetupDuration?.toFixed(0)} prepare ${d.prepareDuration?.toFixed(0)} collect ${d.collectDuration?.toFixed(0)} setup ${d.setupDuration?.toFixed(0)} tests ${d.duration?.toFixed(0)}`)
    const imp = Object.entries(d.importDurations ?? {}).sort((a, b) => b[1].totalTime - a[1].totalTime).slice(0, 6)
    for (const [k, v] of imp) say('   import', v.totalTime?.toFixed(0), v.selfTime?.toFixed(0), k.split('node_modules/').pop())
  }
  onTestRunEnd() {
    say('T', t(), 'runEnd')
    process.on('exit', () => {
      say('T', t(), 'exit')
      process.stderr.write(out.join('\n') + '\n')
    })
  }
}

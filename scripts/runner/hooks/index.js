// ACTIONS_RUNNER_CONTAINER_HOOKS entry point. The runner starts this with its
// bundled node and one JSON document on stdin; everything else happens in the
// quod binary, with stdio passed straight through so the step's output
// and the runner's `::` commands flow as they would from docker.
//
// The binary, the state directory and the policy come from quod.json
// next to this file, written by the installer, never from the environment.
// The step's own `env:` reaches the hook as data on stdin (the protocol's
// environmentVariables), not as this process's environment: a lab probe
// confirmed that a workflow's step-level NODE_OPTIONS, NODE_PATH and
// LD_PRELOAD do not appear in the hook process env, so they cannot steer
// this node at startup. Even so, every QUOD_* variable is dropped
// before the binary starts, so a policy can never be set from the
// environment.
'use strict';
const { spawnSync } = require('child_process');
const fs = require('fs');
const path = require('path');

let cfg;
try {
  cfg = JSON.parse(fs.readFileSync(path.join(__dirname, 'quod.json'), 'utf8'));
} catch (e) {
  process.stderr.write(`quod hooks: no usable quod.json next to the hook: ${e.message}\n`);
  process.exit(1);
}
const env = {};
const dropped = [];
for (const [k, v] of Object.entries(process.env)) {
  if (k.startsWith('QUOD_')) dropped.push(k);
  else env[k] = v;
}
if (dropped.length) {
  process.stderr.write(`quod hooks: ignoring ${dropped.join(', ')} from the step's environment\n`);
}
const args = ['--state', cfg.state, 'hooks', '--policy', JSON.stringify(cfg.policy || {})];
const r = spawnSync(cfg.bin, args, { stdio: 'inherit', env });
if (r.error) {
  process.stderr.write(`quod hooks: ${r.error.message}\n`);
  process.exit(1);
}
process.exit(r.status === null ? 1 : r.status);

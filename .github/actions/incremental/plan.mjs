// Native caches are hints to TypeScript/Next, never proof to skip a command.
// No imports of project code, process hooks, checkout copy, or file-tree hash.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import {pathToFileURL} from 'node:url';

const MAX_DESCRIPTOR = 8 * 1024 * 1024;
const HASH = /^[a-f0-9]{40,64}$/;
const NAMESPACE = /^[A-Za-z0-9_-]{1,64}$/;
const relativePath = p => typeof p === 'string' && p.length < 256 &&
  /^(?:[A-Za-z0-9_@.-]+\/)*[A-Za-z0-9_@.-]+$/.test(p) &&
  p.split('/').every(s => s !== '..' && (s !== '.' || p === '.'));
const inside = (root, p) => p === root || p.startsWith(root + path.sep);

// Check all existing ancestors, including dangling links. Cache extraction
// must not target a path redirected outside the checkout.
function safePath(root, relative) {
  if (!relativePath(relative)) throw new Error('unsupported path');
  let p = root;
  for (const segment of relative.split('/')) {
    p = path.join(p, segment);
    try { if (fs.lstatSync(p).isSymbolicLink()) throw new Error('symlink path'); }
    catch (error) { if (error.code !== 'ENOENT') throw error; }
  }
  if (!inside(root, p)) throw new Error('outside checkout');
  return p;
}

export function plan({workspace, directory = '.', next = false, namespace = 'default',
  revision, runtime = {node: process.version, platform: process.platform, arch: process.arch}}) {
  let bytesRead = 0;
  const miss = reason => ({eligible: false, reason, bytesRead, paths: []});
  try {
    if (!NAMESPACE.test(namespace) || !HASH.test(revision ?? '')) return miss('invalid cache scope');
    if (runtime.platform !== 'linux') return miss('unsupported platform');
    const ws = fs.realpathSync(workspace), root = safePath(ws, directory);
    const descriptor = (relative, optional = false) => {
      const file = safePath(root, relative);
      let stat;
      try { stat = fs.lstatSync(file); }
      catch (error) { if (optional && error.code === 'ENOENT') return null; throw error; }
      if (!stat.isFile() || stat.size > MAX_DESCRIPTOR) throw new Error('unsupported descriptor');
      const bytes = fs.readFileSync(file);
      if (bytes.length !== stat.size) throw new Error('descriptor changed');
      bytesRead += bytes.length;
      return bytes;
    };
    const pkgBytes = descriptor('package.json'), lockBytes = descriptor('package-lock.json');
    const pkg = JSON.parse(pkgBytes), lock = JSON.parse(lockBytes);
    if (pkg.workspaces || ![2, 3].includes(lock.lockfileVersion) || !lock.packages) return miss('unsupported npm project');
    for (const other of ['pnpm-lock.yaml', 'yarn.lock', 'bun.lock', 'bun.lockb', 'npm-shrinkwrap.json']) {
      if (descriptor(other, true)) return miss('ambiguous dependency manager');
    }
    const configBytes = descriptor('tsconfig.json', true);
    let config;
    try { config = configBytes && JSON.parse(configBytes); } catch { /* JSONC/extends need a later adapter. */ }
    const paths = [], tools = [];
    const options = config?.compilerOptions;
    const ts = lock.packages['node_modules/typescript']?.version;
    if (ts && options?.incremental === true && options.noEmit === true && !config.extends &&
      !config.references && !options.composite && !options.outDir && !options.outFile &&
      !options.tsBuildInfoFile) {
      paths.push(safePath(root, 'tsconfig.tsbuildinfo'));
      tools.push(['typescript', ts]);
    }
    const nextVersion = lock.packages['node_modules/next']?.version;
    if (next && nextVersion) {
      paths.push(safePath(root, '.next/cache'));
      tools.push(['next', nextVersion]);
    }
    if (!paths.length) return miss('no supported native cache');
    // Do not overwrite state another action or earlier command already made.
    for (const p of paths) {
      try { fs.lstatSync(p); return miss('native state already exists'); }
      catch (error) { if (error.code !== 'ENOENT') throw error; }
    }
    const digest = crypto.createHash('sha256');
    const add = (label, value) => digest.update(JSON.stringify([label, value]) + '\n');
    add('contract', 'quod-native-incremental-v1');
    add('project', directory); add('runtime', runtime); add('tools', tools);
    add('package', pkgBytes.toString('base64')); add('lock', lockBytes.toString('base64'));
    add('tsconfig', configBytes?.toString('base64') ?? null);
    for (const file of ['next.config.ts', 'next.config.mjs', 'next.config.js']) {
      add(file, descriptor(file, true)?.toString('base64') ?? null);
    }
    const prefix = `quod-incremental-v1-${namespace}-${digest.digest('hex')}-`;
    return {eligible: true, reason: 'native tools validate restored state', tools, paths,
      prefix, key: prefix + revision, bytesRead};
  } catch { return miss('unsupported or unreadable project'); }
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  const start = performance.now();
  const result = plan({workspace: process.env.GITHUB_WORKSPACE,
    directory: process.env.QUOD_INCREMENTAL_DIRECTORY,
    next: process.env.QUOD_INCREMENTAL_NEXT === 'true',
    namespace: process.env.QUOD_INCREMENTAL_NAMESPACE,
    revision: process.env.GITHUB_SHA});
  if (process.env.GITHUB_OUTPUT) {
    const delimiter = `quod_${crypto.randomBytes(16).toString('hex')}`;
    const lines = [`eligible=${result.eligible}`, `key=${result.key ?? ''}`, `prefix=${result.prefix ?? ''}`,
      `paths<<${delimiter}`, ...result.paths, delimiter, ''];
    fs.appendFileSync(process.env.GITHUB_OUTPUT, lines.join('\n'));
  }
  console.log(JSON.stringify({schema: 'quod-native-incremental-plan-v1', eligible: result.eligible,
    reason: result.reason, tools: result.tools ?? [], descriptorBytes: result.bytesRead,
    planningMs: performance.now() - start}));
}

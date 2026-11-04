# Install and typecheck, drop-in

In a Node check job, the install and the typecheck often take more than half
the time. In the pilot, the install took ~90 s and the typecheck ~93 s,
against ~140 s of tests. The pilot's PR #276 cut both by editing the
customer's YAML: an actions/cache of node_modules, and a turbo local cache.

Quod does the same with no YAML edit, from the one action line. This document
covers the three parts:

- the dependency cache, which runs in the action;
- the task cache, which spans the action and the server (`quod-server`
  `taskcache.rs`);
- plain `tsc`, which is designed here but not built.

## 1. Dependencies (pango07/quod-action `deps.cjs`)

**Detection.** The package manager comes from the lockfile at the workspace
root, or in the single subdirectory (at most two levels down) that has one.
`packageManager` breaks a tie when there are two lockfiles. A `.yarnrc.yml`,
or `yarn@2+`, means Yarn Berry.

**Key.** The key is a SHA-256 of everything the install reads:

- the lockfile;
- every `package.json` outside `node_modules` and dot-directories;
- `.npmrc`, `.yarnrc`, `.yarnrc.yml`, `pnpm-workspace.yaml`, `.pnpmfile.cjs`
  and `bunfig.toml`;
- for npm, the user and global config files too;
- node's exact version, and npm's version (for npm);
- the platform and architecture;
- `ImageOS`;
- `ImageVersion`, only when a dependency's install script may compile
  against the image. Install scripts that just fetch a prebuilt binary,
  like esbuild, workerd and unrs-resolver, can't. Otherwise every weekly
  image rollout, which runs the old and new image side by side for days,
  would split the cache. The first bench run lost half its hits to one.
- `NODE_ENV`;
- every `npm_config_*` variable.

**What is kept depends on the package manager.**

- **npm: `node_modules` itself.** Quod keeps the root's and every
  workspace's, except `.cache`, `.vite` and `.quod-vitest`. It is restored
  in the action step, before the customer's install.

  `npm ci` deletes `node_modules`, so the node shim answers it. The shim is
  `hook.cjs`, preloaded into the npm process, because npm is a
  `#!/usr/bin/env node` script and Quod's `node` is first on PATH. The shim
  exits 0 without touching anything only when all of these hold:

  - Quod restored this exact key in this job;
  - the key recomputed at `npm ci` time is still that key, so nothing
    changed a `package.json` or the lockfile in between;
  - the saved tree's marker (`node_modules/.quod-deps.json`) names the same
    key and the same `npm ci` arguments. Arguments that change only output
    are ignored, such as `--no-audit` and `--loglevel`;
  - a print of the tree (npm's hidden lockfile plus the top-level entries)
    equals the one taken right after the restore. So another cache step
    restoring over it, or anything else writing into it, makes npm run.

  Otherwise npm runs as always. A successful `npm ci` at the root is noted
  with a full print of the tree (every path, size, mtime and mode). Each
  `npm ci` clears the previous one's note, so a later failed install saves
  nothing. The post step saves the tree only when all of these hold:

  - the print is unchanged, so nothing wrote into `node_modules` after the
    install (`npm install --no-save`, a generator, a patch);
  - the job runs the repository's own code: push, same-repository
    `pull_request`, `merge_group`, `workflow_dispatch` or `schedule`. A
    `pull_request_target` or `workflow_run` job saves into the base
    branch's cache scope while it may run someone else's code, so it
    restores but never saves;
  - nothing outside the key can have gone into the tree:

  - none of the project's own `package.json` files has an install lifecycle
    script (`preinstall`, `install`, `postinstall`, `prepare`, ...). Those
    can read or write anything.
  - every dependency with `hasInstallScript` in the lockfile is on a list of
    packages whose install script writes only into its own directory and
    reads only itself and the platform: esbuild, core-js, workerd,
    unrs-resolver, sharp, ...

    @prisma/client (it reads `schema.prisma`), puppeteer, cypress
    (`~/.cache`) and husky are not on the list. A tree with one of them is
    never saved, so `npm ci` always runs.

  On a `node_modules` miss, if no `~/.npm` was restored (setup-node's
  `cache: npm` restores one), Quod restores npm's download cache by prefix
  (the nearest earlier lockfile's), and saves it after the install.
- **pnpm, Yarn and Bun: the store.** Quod keeps pnpm's store, Yarn Berry's
  cache, Yarn Classic's cache and Bun's install cache. It restores the exact
  key, else by prefix, and saves on a non-exact hit. The customer's install
  runs as always and verifies the store's content itself, so a stale or
  partial store is only slower, never wrong. Yarn Classic's `node_modules`
  is not kept, because its integrity check skips lifecycle scripts. A store
  that is already filled (setup-node's `cache:` restored it) is left alone.

**Trust.** The Actions cache is scoped by GitHub: a pull request reads its
base branch's entries and writes only into its own ref. So no pull request
can put a tree in front of main.

## 2. Task cache: turbo and Nx (quod-server `taskcache.rs`)

**Endpoints.**

- **turbo.** turbo reads `TURBO_API`, `TURBO_TOKEN` and `TURBO_TEAM` from the
  environment. The server implements turbo's Remote Cache API:
  - `GET`, `HEAD` and `PUT /v8/artifacts/{hash}`;
  - `POST /v8/artifacts` (a query of hashes);
  - `POST /v8/artifacts/events` (accepted, not kept);
  - `GET /v8/artifacts/status`.
- **Nx** (20.8+). Nx's self-hosted remote cache spec is
  `NX_SELF_HOSTED_REMOTE_CACHE_SERVER` plus `..._ACCESS_TOKEN`, with
  `GET` and `PUT /v1/cache/{hash}`. The server mounts it at
  `/nx/v1/cache/{hash}`, and a second PUT of a hash answers 409 as the spec
  says. It costs ~30 lines on top of turbo's, in the same table and blobs.

**Auth: a session.** A GitHub OIDC token lives 5 minutes, but `TURBO_TOKEN`
is read once and used all job long. So the action trades the job's OIDC token
(audience `quodci`, as everywhere else) for a session at
`POST /v1/task-cache/session`.

- **Format:** `qtc1_<hex JSON>_<hex BLAKE3 keyed MAC>`.
- **MAC key:** derived from the server's signing key ("quod-server task-cache
  session v1").
- **Lifetime:** 6 h, the longest job.
- **Contents:** team, repository key (GitHub's repository id), the scope it
  writes (if any), the scopes it reads, and the job's environment.

**The action's side.**

- It masks the token (`setSecret`).
- It adds the token to the post step's leak check, so a report artifact or
  cached ledger containing it is not uploaded.
- It sets the variables only when the customer has no remote cache of their
  own: `TURBO_API`, `TURBO_TOKEN`, `TURBO_TEAM` and `TURBO_TEAMID`, and Nx's
  `NX_SELF_HOSTED_REMOTE_CACHE_SERVER`, `NX_CLOUD_ACCESS_TOKEN` and
  `NX_NO_CLOUD`.

The session endpoint refuses a delegated token, because such a token states
its own scope.

**Trust scopes.** These are test replay's (`auth::oidc_scopes`):

- **Writers** are push, dispatch or schedule jobs on a writer ref (main,
  dev). They write and read `trusted`.
- **Other jobs of the repository** (another branch, a pull request, a merge
  queue) write their ref's scope. They read it first, then `trusted`.
- **Jobs on someone else's input** (`pull_request_target`, `workflow_run`)
  and fork scopes read `trusted` and never write. Fork pull requests get no
  OIDC token at all.

So a pull request's artifact, poisoned or not, is never what main, dev or
another pull request reads (`tests/task_cache.rs` K2).

**The key.** An artifact is keyed by (team, repository, environment, tool,
scope, the tool's hash). The environment is a hash of the OS, architecture,
node version and runner image (`ImageOS`) the action states. It is not in
turbo's or Nx's own hash, so it is what keeps a node-20 job of a matrix from
replaying a node-22 job's `test`.

**Rows are only added.** The first artifact of a key stays. turbo's hash
names the task's inputs, so a second upload would be the same task's output.
The bytes go into the team's blobs: the bucket in Quod Cloud, the CAS when
self-hosted. In Postgres, the table `task_cache` is created at start, outside
the versioned migrations, like the test-replay tables, so a rollback finds a
schema it knows.

**Correctness is turbo's hashing.** A task's hash includes:

- its package's files (git hashes);
- its resolved external dependencies (from the lockfile);
- the hashes of the tasks it depends on: `^typecheck` makes `web`'s hash
  include `shared`'s typecheck hash, so a type change in `shared` re-runs
  `web`;
- `env`, `globalEnv` and `globalDependencies`;
- turbo.json.

turbo never caches a failed task. The floor bench's `typeerror` case checks
this: a shared type change breaks two pages that did not change, and the
typecheck must still fail with the same errors.

**Bodies.** No handler buffers a body before the session checks out:

- **Uploads** stream to disk with `QUOD_MAX_BLOB_BYTES`.
- **The query** reads at most 64 KiB and answers 100 hashes.
- **Events and status** read nothing.

**Caveats.**

- **The environment** is stated when the action runs. A second setup-node
  later in the job, with another node, would replay across node versions.
  The Quod line belongs after the job's last setup-node.
- **Nx PUTs.** A read-only session (`pull_request_target`) gets 403 on
  PUT, which Nx's spec defines for read-only tokens.
- **Concurrent uploads.** Two concurrent uploads of one key both reach the
  bucket, and only the first is referenced.
- **No garbage collection** of old artifacts yet. They are in the bucket
  like test-replay ledgers.
- **No artifact signing.** `TURBO_REMOTE_CACHE_SIGNATURE_KEY` isn't needed,
  since the server scopes who wrote what.

## 3. Plain `tsc` (no task runner): design, not built

The question is whether Quod can skip `tsc --noEmit`, or
`next typegen && tsc --noEmit`, when nothing it reads changed.

**Full replay, keyed on what tsc reads.**

- **The input list.** `tsc --listFilesOnly` (or `--explainFiles`) lists the
  program's files after module resolution. Quod would key on:
  - the files' contents;
  - the tsconfig chain (`extends`, including packages in `node_modules`);
  - the compiler's version;
  - the options;
  - and every `typeRoots` directory listing.
- **Why it is not safe.** tsc's answer also depends on paths it probed and
  did not find. Resolution tries `./foo.ts`, `./foo.tsx`, `./foo.d.ts`,
  `./foo/index.ts`, `package.json` `exports` and `typesVersions`, and walks
  up `node_modules` directories. If a file appears at a probed path, the
  program changes, and the list of files read does not show it.

  tsc's watch mode tracks these "failed lookup locations" internally; no
  flag prints them.

  An `include` glob is re-evaluated by `--listFilesOnly`, but a key built
  from that list would have to run it every time. That costs a large part
  of the typecheck (it parses every file to find imports).
- **Verdict.** The saving is limited, and correctness needs the probe set.
  Not built.

**The light version: tsc's own incremental mode.** The node shim already
sees every node process. For `tsc --noEmit` without `--incremental`,
`--build`, `--watch` or `composite`, it can add
`--incremental --tsBuildInfoFile $RUNNER_TEMP/quod/tsbi/<id>.tsbuildinfo`.
The id is the tsconfig path plus the arguments. It restores that file from
the Actions cache by prefix and saves it after a passing run.

- **Why it is safe.** tsc re-resolves the whole program every time and
  compares each file's content hash with the buildinfo. It re-checks
  changed files and their dependents, and everything when a global
  declaration, an option or its own version changed. Quod would only
  supply the cache file. The repository is untouched, because the file
  lives in the runner's temp. Output and exit code are tsc's own.
- **Residual risk.** This trusts tsc's incremental correctness (the same
  trust `tsc -b` users already place in it).
- **What it buys.** It only re-checks what changed. Parsing and binding
  still happen. On the floor fixture, the hand-written version of this
  (bench-floor's `quod` strategy) saved little over a turbo cache. Its main
  value is repositories without a task runner.

**Next step.** Prototype it behind an input (`typecheck-cache`). Then measure
it on bench-turbo-dropin's no-turbo variant before turning it on by default.

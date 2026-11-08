# Native incremental execution

Experimental opt-in action: `.github/actions/incremental`. It preserves
TypeScript's dependency graph and optionally Next.js's build cache between
fresh runners. The native tools still inspect inputs, determine invalidation,
and execute every original command. No result replay or skipped test is claimed.

```yaml
- uses: actions/checkout@<reviewed-sha>
- uses: actions/setup-node@<reviewed-sha>
  with:
    node-version: '24.22.2'
- uses: pango07/quod/.github/actions/incremental@<reviewed-sha>
  with:
    working-directory: cloud
    next: 'true'
# Keep the existing install, lint, typecheck, test and build steps.
```

The TypeScript adapter supports an npm project with a JSON tsconfig,
`incremental: true`, `noEmit: true`, and the default build-info location.
Extends, project references, composite projects, custom output locations,
JSONC and other package managers bypass this first adapter. Next is explicit
opt-in and supports the default `.next` directory. Configuration files are
hashed as data, never imported. Only `tsconfig.tsbuildinfo` and optionally
`.next/cache` are transported, not node_modules or final build outputs.

The small planning step reads bounded package/lock/config descriptors. It
does not scan source files, rehash dependency trees, install Quod's sandbox,
or intercept execution. A versioned key separates Node, OS, architecture,
package lock, configuration, project and adapter selection. The commit suffix
permits a new state to be saved after source changes. A compatible older
cache is a native compiler hint, not authority to accept the previous result.
Preexisting state and symlinked destinations decline restoration.
The optional `generation` input defaults to the job's commit SHA; workflows
that check out a different revision can supply that commit or a content digest.
It changes the save key, not the compatible-state prefix or native validation.

Transport is GitHub Actions cache with its repository/branch access rules.
No Cloud signing, API, production deployment or Class-management change is
part of this milestone. Existing native caching should remain enabled in
both benchmark arms. Cache setup, downloads and post-job uploads count as
overhead; a warm command is not a whole-job saving.

Validation: dispatch `gate.yml` with `only=incremental`, at the reviewed
branch and exact source SHA. First, actual compiler controls compare restored
and clean state on unchanged inputs, implementation changes, changed imported
types, added erroneous files, deleted dependencies, generated type changes and
compiler configuration changes. Every diagnostic and exit status must agree.
Then three matched pairs run all real Cloud commands on independent runners:
cold state (which also seeds Quod's cache), warm state and changed root-layout
metadata. Both changed builds must contain the new metadata in their server
output, guarding against a stale production build. Cache namespace is unique
to that workflow run. No same-runner replay
or repeated warmup commands occur in the matched jobs. The workflow waits for
the seed job's post steps before starting consumers.

Acceptance: all controls and full app checks pass; real cache transport works
on fresh runners; full-job measurements, including the initial learning/save
cost, justify further trials. Three pairs are an initial decision experiment,
not evidence of a general percentage improvement. A slow or inconclusive
result keeps the action experimental. Routine CI remains native.

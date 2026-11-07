# Experimental managed Vitest Action profile

The optional `managed-vitest: v0` Action input selects an explicit managed
execution contract. Its default remains `off`; `control` executes the same
managed workload without remote receipt reads or writes. This is an experimental
branch integration, separate from the ordinary Quod mode and Quod Cloud.

After `setup-node`, optional usage pinned to the built Action is:

```yaml
- {uses: pango07/quod-action@d79c8635f4dbc93c842ec1da8bd5ee6bf8e72928, with: {managed-vitest: v0}}
```

The initial profile handles
`packages/convex-helpers/server/stream.property.test.ts` under the reviewed
configuration of the original twelve-commit convex-helpers window. Its original
47 tests, three property seeds and 24 cases per seed remain intact. Other
selected files run through ordinary Vitest, preserving the original suite and
the commands after it. Custom commands, configuration, unsupported capabilities
and incomplete managed completion decline to ordinary execution.

The ordinary remainder uses Vitest 5's native specification API with the actual
resolved file/project instances. It excludes exactly the admitted instance,
checks the selected and executed instance lists against preparation, and
retains each remaining project's configuration, aliases, environment and setup.

The caller declares an epoch in milliseconds (default `1700000000000`), random
seed (`42`), and explicit environment (`{"NODE_ENV":"test"}`). Managed execution
uses UTC, the pinned workerd runtime and the capability/completion policy in
[the verified replay evidence](bench/managed-vitest-verified-replay.md). These
semantics are part of the opt-in; they do not assert equivalence to arbitrary
ordinary Node environments.

Every decision repeats native capability preflight, actual Vitest preparation
and closed-module compilation. Only the reviewed executable configuration
sources and finite package/lock variants are admitted. Package and lock hashes
are retained as admission evidence; the receipt identity includes the actual
transferred configuration, closed source and resolution bytes, module-name
sets, compiler, runtime, supervisor, declared inputs and native host class.
Changing an unused package version does not by itself change the managed
execution. An unreviewed package or configuration variant declines.

New receipts require passing, sealed, complete managed results and a successful
exit from the entire intercepted Vitest command, including its ordinary
remainder. Failed, unsupported or incomplete commands cannot publish new
receipts. A hit validates the complete receipt bytes and every effective
identity before publishing its test states and console effects. Test JSON
combines the admitted result with the real ordinary
report for the remaining selected files; an ordinary failure still fails the
command.

Receipt transport uses immutable GitHub Actions cache entries with a full key
containing the execution key and the receipt byte SHA-256. Only the key actually
returned by the authenticated cache client supplies the trusted receipt digest;
a local or downloaded manifest cannot supply authority. The trust scope is
GitHub's repository workflow writers and branch/ref cache rules. This transport
does not defend against an authorized malicious cache publisher. The existing
Action rules forbid fork pull requests, `pull_request_target` and `workflow_run`
jobs from publishing managed receipts.

The Action keeps its cache-service session in a private job-local file, injects
it only while calling the cache SDK, restores the bridge environment before
ordinary child execution, and removes the file in its post step. Evidence
uploads exclude this file and the Action configuration. This accommodates the
runner's separate environment handling for
[JavaScript actions](https://github.com/actions/runner/blob/main/src/Runner.Worker/Handlers/NodeScriptActionHandler.cs)
and [shell steps](https://github.com/actions/runner/blob/main/src/Runner.Worker/Handlers/ScriptHandler.cs).

The Linux x64 runtime is packaged on GitHub from a full Quod source commit and
shipped with the Action. Its archive digest is verified before extraction; the
project job does not install the runtime from npm. `cache-namespace` isolates
dependency and receipt caches so a benchmark can start genuinely cold.

The full-window evaluation retains ordinary `npm ci`, `npm run build` and
`npm run test`, measures complete fresh jobs at all twelve original commits,
and adds an independent uncached managed control. It reports runner-seconds and
the sum of per-job rounded minutes separately. A partial file hit is not a full
job hit. The original ordinary benchmark and its unmet result remain in
[the ordinary window evidence](bench/convex-helpers-current-cloud-window.md).

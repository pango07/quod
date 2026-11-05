# What the current benchmark establishes

Quod adds a GitHub Action to an existing job. It is not a replacement runner
or a universal CI/CD accelerator. Installation simplicity, replay eligibility,
correctness evidence and measured savings are separate claims.

The defensible description is:

> Add one GitHub Action to supported Vitest jobs. Keep your install and test
> commands. Quod reuses verified results when their inputs are unchanged and
> runs the rest.

## Eligibility and evidence

| Workload | Current boundary |
|---|---|
| One-shot Vitest on Linux x64 | The principal test-replay target. Node must provide `module.registerHooks` (22.15+ on the 22 line); the latest complete benchmark uses Node 24.18.0 and Vitest 5.0.0. |
| Vitest versions | Compatibility paths exist for 3.2, 4 and 5. The experimental action now admits only stable 3.2–3.x, 4.x and 5.x releases; future majors, prereleases and malformed versions stay plain. This bounds interception but does not certify every minor/configuration combination. Publish exact tested combinations. |
| Monorepos and multi-project Vitest | Supported by the per-project/file model; shared config remains an input. This benchmark runs 120 file/project instances across nine projects. A shared input change can invalidate most of them. |
| npm, pnpm, Yarn and Bun | npm can restore the installed dependency tree; other managers can reuse their stores. Dependency caching is distinct from test-result replay. Yarn Plug'n'Play stays plain. |
| npm build and script commands | Experimental whole-job engine observes selected `npm run <script> [-- args]` commands. Workspace flags and arbitrary command forms do not become supported merely because ordinary npm scripts work. |
| Coverage | The driver has V8/Istanbul integration and provider/version guards. Unknown providers and unsupported modes execute the tests; coverage needs its own compatibility tests and benchmarks. |
| Watch, browser/UI mode, snapshot updates, shards, changed-file filters, typecheck mode | The Action leaves these command modes plain. Browser projects, VM pools and non-isolated projects are not recorded by the driver. |
| External services, stateful effects and unobservable work | Do not promise replay. Files outside the recorder's observed boundary execute again; sandboxed commands and supported child processes have separate restrictions. |
| Windows, macOS and other architectures | No claim of Vitest test-result replay. Some dependency/task-cache features can work on macOS. |
| Jest, Playwright, Cypress, Python, Go and arbitrary shell jobs | No performance or compatibility claim from this Vitest window. Other Quod mechanisms must be validated independently. |
| Other CI providers and self-hosted runner configurations | Not established by these GitHub-hosted jobs. |

The eligibility guards are in `quod-action/hook.cjs`, `src/main.js` and
`quod`'s `crates/quod-cli/src/vitest/driver.mjs`. A fallback means the job
executes normally, not that it receives the same acceleration. Existing
release documentation must be checked against the exact action pin. The
experimental action's README and metadata have been updated to distinguish
one-line installation from eligibility and guaranteed savings.

The compatibility guard and passthrough regressions are in
[action commit 5752bc0](https://github.com/pango07/quod-action/commit/5752bc0).
They cover supported majors, future/malformed versions, prereleases,
watch/browser/shard/update/changed modes and Yarn Plug'n'Play. These changes
are on `exp/boolean-exports-codex`; they do not move any published release tag.

## Performance and correctness claims

The [complete 12-commit report](config-names-proof-window.md) measured
45.2% less runner time and 41.7% fewer per-job rounded minutes including
the cold start. After the cold start, runner time fell 51.7%. These are
measurements of that pinned window and experimental policy, not guaranteed
customer savings. Public jobs cost $0. The separate best warm hit was
floor + 3 seconds; it is not the duration of every commit's CI.

This policy enables lineage witness and the bounded configuration names
proof. Both are experimental; lineage witness is switchable and off in
the Action by default. The founder's veto remains open. Timing-sensitive
replay was off for the better-performing window.

All observed per-commit results agreed with plain runs. That establishes
zero observed wrong results, not universal recorder completeness. The
[audit](convex-helpers-audit.md) still identifies general file-stat metadata
as an unresolved input-observation risk. A sampled audit can discover an
error after a replay; it cannot prove the first replay was correct.

Cold recording, plain confirmations, audits, dependency changes and shared
configuration can make an individual job slower. Net savings must include
all those costs, setup and post steps. Speed and cost percentages are never
added together. A broader promise requires an exact compatibility matrix,
adversarial input tests and independent real repositories, with unsupported
cases and fallback behavior tested alongside successful replay.

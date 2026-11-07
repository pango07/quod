# Runtime boundary decision, 2026-11-07

This is a source review, not a compatibility run, replay implementation or
new benchmark. No test environment, Action pin or production setting changed.

## Decision

Do not replace the original benchmark's runtime with Cloudflare workerd and
count the result as transparent Node/Vitest acceleration. Its existing Vitest
integration is a plausible building block for a separately specified managed
mode, but does not supply Quod's deterministic input/effect contract by itself.
Do not start another compatibility-only benchmark before specifying that
contract and how it is enforced.

The original c11 project already selects `environment: "edge-runtime"` for
`packages/convex-helpers`. Its lockfile names Vitest 5.0.0 and
`@edge-runtime/vm` 5.0.0. That is not Cloudflare workerd. Vitest 5's
[environment adapter](https://github.com/vitest-dev/vitest/blob/v5.0.0/packages/vitest/src/integrations/env/edge-runtime.ts)
provides both `setupVM` and ordinary `setup`; the latter populates the worker's
global object from an EdgeVM context and supplies host values. The environment
name alone therefore does not establish a closed execution boundary. This
review does not assert which setup path a particular recorded worker used.

## Existing runtime assessment

| Requirement | Verified source fact | Consequence for Quod |
| --- | --- | --- |
| Vitest integration | The workers-sdk source package is named `@cloudflare/vitest-plugin`, version 1.6.0, with peer range `^4.1.0 || ^5.0.0`. | Vitest 5 is not excluded by the source peer range. Published package identity and actual convex compatibility were not verified. |
| Preparation boundary | Configuration and global setup run in Node; test setup and files run in workerd. Workers and module caches may be reused. | Node-side transformation, module loading, provided state and lifecycle remain part of the input/effect argument. A worker-only key is insufficient. |
| Filesystem | Workers provides a memory filesystem with read-only `/bundle`, request-local `/tmp`, and devices including `/dev/random`. Documented timestamps are the Unix epoch; permissions and ownership are unsupported. | This changes observable filesystem behavior. It cannot silently replace the original baseline, and randomness remains exposed. |
| Network | workerd's `globalOutbound` defaults to the `internet` service; services and network allow/deny rules are configurable. | Network access is not denied merely by choosing this runtime. Every reachable service and binding needs a policy. |
| Time | Local Workers timers advance even without I/O. | Isolation does not make time a stable input. A clock policy and enforcement are still needed. |
| Isolation | Cloudflare explicitly says Miniflare is not a hardened sandbox for untrusted code. | Do not infer a security guarantee from local runtime isolation. This is separate from the determinism and dependency-completeness requirements. |

Primary sources: [package metadata](https://github.com/cloudflare/workers-sdk/blob/main/packages/vitest-plugin/package.json),
[execution model](https://developers.cloudflare.com/workers/testing/vitest-integration/isolation-and-concurrency/),
[filesystem](https://developers.cloudflare.com/workers/runtime-apis/nodejs/fs/),
[workerd configuration](https://github.com/cloudflare/workerd/blob/main/src/workerd/server/workerd.capnp),
[local timers](https://developers.cloudflare.com/workers/runtime-apis/performance/),
[local testing warning](https://developers.cloudflare.com/workers/testing/).

The source inspection did not establish a complete policy for native code,
dynamic code, randomness, scheduling, exception observers or effects. A
successful test run in workerd would establish compatibility for that run,
not complete dependency tracking or deterministic replay. No package was
installed and no Cloudflare account or service was used.

## Product and engineering gate

The useful distinction is between one-line installation and eligibility for
replay. One line can install Quod and leave unsupported work running normally.
It cannot make every arbitrary test independent of its host, current time or
external services. Past agreement cannot establish that independence.

A managed candidate must first name its complete exposed capabilities and
the mechanism that mediates them. Specify module bytes and resolution,
configuration, environment, metadata, clocks, randomness, scheduling, output
ownership and unsupported operations. An operation outside the contract must
prevent reuse; falling back during a later execution cannot repair a replay
that already skipped that execution. Preserve ordinary failure semantics and
required effects, including when declining eligibility.

Then use the original expensive property test, with unchanged cases and
assertions, as the first bounded execution gate. Demonstrate a genuine hit on
a fresh runner, misses for relevant mutations, a hit for an unrelated change,
and rejection of hidden inputs/effects. Retain full result names/states and
required outputs. Include cold, miss, validation, audit and storage costs.
If the candidate changes the execution contract, keep both the ordinary
control and an uncached managed control, and report them separately.

Do not expand to the full window until this gate shows useful avoided work.
The current original window remains 1,538 versus 1,735 runner-seconds, with
zero test/unit replays. Its 50% threshold requires another 670.5 seconds of
reduction. The separate 2.440-second adaptive-recording comparison does not
close that gap. See the [verified window](convex-helpers-current-cloud-window.md)
and [recording comparison](recording-admission.md).

## Source identities

Fetched from the named refs on 2026-11-07; these are Git blob IDs, not release
artifact digests. Moving branches are not package pins.

| Repository / ref / path | Blob SHA |
| --- | --- |
| `vitest-dev/vitest` / `v5.0.0` / `packages/vitest/src/integrations/env/edge-runtime.ts` | `1ca0342f10868a731c809a5c48a33a9442235db3` |
| `cloudflare/workers-sdk` / `main` / `packages/vitest-plugin/package.json` | `0505031a32b50fcf41d80fcc878c8568ff95a634` |
| `cloudflare/workerd` / `main` / `src/workerd/server/workerd.capnp` | `bc79bba9fb19f9d4280ddd24dd7918f03d3b7f43` |
| `cloudflare/cloudflare-docs` / `production` / `src/content/docs/workers/testing/vitest-integration/isolation-and-concurrency.mdx` | `c0035e09a37e5ba2d6589c8ecfb494d0004caf37` |
| `cloudflare/cloudflare-docs` / `production` / `src/content/docs/workers/runtime-apis/nodejs/fs.mdx` | `5a58e20426663024a202f2602cfe4a8aca2ff089` |
| `cloudflare/cloudflare-docs` / `production` / `src/content/docs/workers/runtime-apis/performance.mdx` | `d53d18062cd2b9a6ae276b735bfb740e05df20fa` |
| `cloudflare/cloudflare-docs` / `production` / `src/content/partials/workers/local-development-security.mdx` | `ee81d2218271c99fedc554f3d2439c60497d51c6` |

# Cold preparation overlap

**Decision: retain as an experiment; do not adopt the overlap optimization.**
Correctness validation passed, but the final control showed no useful cold
performance win. Routine repository CI now uses native fallback through
[PR #10](https://github.com/pango07/quod/pull/10); this branch preserves the
client experiment and its reproduction controls.

The observed Cloud comparison at `fa321015` took 175 seconds through Quod
and 100 seconds plain. Its three commands all missed. Lint accounted for
44 seconds of the difference: the command itself ran for 8.7 seconds, but
the first preparation, import and seal added substantial cold work.

This experimental client starts the checkout import and the isolated Git
view beside environment pinning and Cloud metadata registration. Only the
checkout and Git view are image-independent. Profile dependencies under
the state directory are grafted after the image is known, using that
image's directory. Files, exclusions, stat-cache checks, seals, step keys,
trust policy, replay validation and commands retain their existing rules.
Network steps never start a checkout import. Both tasks are joined before
any proof decision or command; failures and panics are retained.

`quod run --serial-preparation` is a hidden diagnostic control that runs
the same work in the previous serial order. It does not affect the key or
the trust policy. The explicit `gate.yml` selector `cold-path` builds the
client from the dispatched source, checks unit regressions and runs this
control against the fixed Cloud workload `23b3694b` on one GitHub runner.
No release, backend deployment, main merge or Class-management change is
part of this gate.

The client runs through the setup action's existing root wrapper, with
the reviewed binary's hash checked against the installed one. Fresh state
directories live under the normal private `/var/lib/quod`, so the sandbox
does not need access through GitHub's runner-owned temporary directory.
The runner reads only its experiment event data through `sudo cat`;
state and sandbox permissions are unchanged.

Four fresh Quod stores use parallel/serial/serial/parallel order. Each
also runs a second execution against its reused store. Observe mode keeps
every lint command executing. The control rejects changes in the entire
input tree, step key, image, positive reads or compared outputs, requires
each manifest to match its actual input tree, and validates all 496 Cloud tests, the
route generation plus typecheck, and the native production build. Two
plain lint observations bracket the control. The OS page cache is shared;
the stores are fresh, not four independently cold hosts. These are command
diagnostics, not a new whole-job comparison or proof of replay savings.

New diagnostics report environment pin, host fingerprint, platform probe,
Cloud metadata registration, checkout import and end-to-end command time.
Some phases overlap and cannot be added. The existing `total_ms` field
keeps its prior meaning for compatibility. The outer process timer remains
the comparison's wall-clock measurement, including all preparation.

The first measured control retained matching step keys and images but
produced different manifest IDs, including repeated serial controls. The
bounded diagnostic established identical entire input trees and every
compared output hash across all eight executions. The only entry differences
were extra absent lookups for `node_modules` below the invite and reset-password
pages. Exact read-set identity was too strict for this valid resolver variation.
The corrected control retains every absence record in Quod's manifests and
checks each manifest against the actual input tree, with identical positive
reads, frame, complete input trees and compared outputs. Negative controls
reject content, mode, path, frame, output, key, image and validity changes.
Opt-in `--diagnose-inputs` writes bounded metadata and hashes into private
state; no file contents or environment values are exported.

## Validated result

[Gate 42518353307](https://github.com/pango07/quod/actions/runs/42518353307),
attempt 1 at `53938cc01e8f23dfee077348bb6dbc177884f912`, passed seven
negative-control tests, the CAS and CLI checks, format and Clippy, the
experimental release build, eight controlled lint executions, route
generation/typecheck, all 496 Cloud tests and the native production build.
The entire input tree, compared outputs, keys, images and positive reads
agreed; every actual manifest matched its input tree. Extra validated
absence observations remained in their original manifests.

These are process-wall medians from two observations per mode/state on
one runner, with observe mode executing the command every time:

| Quod state | Serial preparation | Overlapped preparation |
| --- | ---: | ---: |
| Fresh | 20.197s | 20.601s |
| Reused | 10.752s | 10.580s |

The two plain lint observations bracketing the control were 7.494s and
6.946s. In the fresh serial control, execution itself took 7.469s and the
remaining process overhead was 12.728s. Checkout import took 5.655s and
sealing 3.999s; fingerprint/platform pinning took 0.699s and Cloud metadata
registration 0.303s. Diagnostic metadata capture added about 0.097s and is
included in wall time. Some phases overlap or contain subphases, so this
is not an additive phase accounting table.

Cold overlap was 0.403s slower in this control; the reused-state difference
was only 0.172s faster. An earlier control had a small cold improvement,
so there is no repeatable performance justification for shipping this
change. The original 28.6s setup spike did not recur; the old aggregate
timing does not identify its exact cause. This control proves neither
fresh-runner replay nor net whole-job savings.

## Immediate change and next design

The separately released native fallback at main `59e7549` removes Quod
installation, environment pinning, checkout import, sealing and recording
from routine pushes and pull requests while reuse is unproven. Every
normal check command remains intact; explicit manual Quod recording and
matched comparisons remain available. Both PR and main native jobs passed
all 496 tests and the build in 112s complete job lifetimes. These are
unpaired observations; no difference against the old 175s job is claimed.

The next substantive design should make the normal path a cheap reuse
check, with all actual observer/training costs accounted for:

- Decline acceleration early when there is no usable candidate or runtime
  contract. Execute natively and record the bypass reason without claiming
  a replay. A negative hint may skip acceleration; it grants no authority.
- On a candidate, validate its actual recorded files, listings, symlinks,
  modes and absence states before replay, under the existing receipt and
  trust rules. Avoid importing unrelated workspace contents on that path.
- Establish immutable runtime and dependency snapshots once under a
  defined filesystem-metadata contract, then reuse them. A direct
  copy-to-hardlink substitution changes observable metadata and needs
  explicit treatment before adoption; read-only binding alone is not
  enough to establish equivalence.
- Keep learning on an explicit bounded budget. Report its full compute
  cost and require repeatable matched whole-job wins before enabling an
  observer path for routine hosted builds.

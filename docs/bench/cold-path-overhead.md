# Cold preparation overlap

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

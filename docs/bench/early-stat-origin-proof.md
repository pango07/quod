# Stat origins before user preloads

The diagnostic now snapshots native stat functions and the Stats prototype
chain in the first CommonJS preload, before ordinary user preloads. This
closes a measurement gap: a snapshot taken in the main driver cannot tell
whether a preload has already replaced a native method. This remains
diagnostic evidence, not authorization to omit metadata from a replay key.
The packaged production engine remains `7ff2b2f`.

The bootstrap restores the original `NODE_OPTIONS` before user preloads
execute, preserving the distinction between an absent variable and an empty
string. It accepts ordinary require/import options conservatively; other
startup flags leave the origin unavailable. Node documents preload ordering
in its [v24.18.0 CLI source](https://github.com/nodejs/node/blob/v24.18.0/doc/api/cli.md).

[Regression gate 42159121050](https://github.com/pango07/quod/actions/runs/42159121050)
at `b7f6d07` passes 28 observer tests and 58 production proof tests, with
zero failures. Thirteen startup fixtures cover absent/empty options, quoted
preload paths, require/import preloads, Stats method and setter changes,
native/public stat replacement, Inspector connections, prior/late loading,
unsupported flags, and a preload that poisons `Array.prototype.every`.
The prototype comparator uses captured primitives and indexed loops.

[Real driver gate 42158549345](https://github.com/pango07/quod/actions/runs/42158549345)
at `2ca4880` passes using the existing immutable `4d99319` diagnostic binary
and current JavaScript overrides. It precedes the comparator hardening in
`b7f6d07`; that later change has isolated regression coverage, not a repeated
full-driver measurement.

| Real driver observation | Result |
| --- | ---: |
| File/project instances | 120 |
| Test states matching the established fingerprint | 4,131 |
| Size consumers with unchanged observed startup origins | 120 / 120 |
| Size consumers with native outcomes and only owned hooks | 120 / 120 |
| Full metadata keys (`sm`, `sl`) | 299, 257 |
| Type keys (`st`) | 104 |
| Replayed files | 0 |

Two new watched Inspector connections came from Quod's own Boolean and
names proof helpers. The initial native-object census remains unclassified;
these counts do not exclude worker or native-addon observation channels.
The result fingerprint is
`09ebb9e3a722220d3a21ddfe9f63992a8a074b0fcaa95764e54c37be90d469cd`.

Production integration still requires an immutable origin interface,
complete handling of unsupported observer channels, consumer/lifetime
validation, and size/error query equivalence. The launcher must prepend the
bootstrap only to the spawned Node child's environment, preserving the
planner's original environment key. No async size keys are enabled by this
work. There is no new hit-job or 12-commit savings measurement.

See [machine-readable evidence](early-stat-origin-proof.json) and the
[Inspector regression evidence](inspector-stat-observers.md).

# Native unit metadata regression

The shipped `081f20a` binary can replay stale output after metadata-only
changes. [Gate 42170375367](https://github.com/pango07/quod/actions/runs/42170375367)
reproduces eight failures in nine real-syscall fixtures. For example,
after changing only a file's mtime, plain prints `STAMP=1700000600` while
Quod replays `STAMP=1700000000`. File contents are verified unchanged.

| Case | Shipped engine stale | Fix stale | Upgrade from old record stale |
| --- | ---: | ---: | ---: |
| File mtime | yes | no | no |
| Directory mtime | yes | no | no |
| File permissions | yes | no | no |
| Symlink mtime (`lstat`) | yes | no | no |
| Descriptor metadata (`fstat`) | yes | no | no |
| Descriptor metadata (`AT_EMPTY_PATH`) | yes | no | no |
| `statx` mtime | yes | no | no |
| Installed-package file mtime | yes | no | no |
| Symlink target, unchanged target bytes | no | no | no |

Fix `f9d8153` separates full stat/lstat keys from file-content and package
keys, traces descriptor metadata, and invalidates version-4 unit records.
Unsupported metadata queries (`statx`, filesystem statistics, xattrs),
unresolved descriptors, and metadata observed on paths modified by the
command fail closed. They need complete state representations before they
can replay. This is a correctness change with a likely hit-rate cost.

[Gate 42171117245](https://github.com/pango07/quod/actions/runs/42171117245)
has passed the CLI/recorder tests and all three nine-case metadata phases:
old binary, new binary, and old records opened by the new binary. Eight
supported unchanged cases still warm to a replay; metadata changes then
rerun correctly. `statx` runs normally. The fixture writes output without
buffered-stdio descriptor queries so anonymous stdout cannot accidentally
make every case uncacheable and conceal a missing metadata key.

The integration test retains record/plain-witness/replay/output-restoration
and E2 cross-input agreement checks using a content-only native copier and
counter. The shell copier remains covered for correct execution. Its
metadata observations can now make it uncacheable. This is not evidence
that ordinary npm build scripts retain their old hit rate.

The remote Rust tests, lint and release build passed. Packaged-release checks
and the Action bundle build are pending. The published action pin remains
`1aa08cacec95f243aa12adafdf815f68213c2783`; pilots must set job-level
`QUOD_UNITS: '0'` and keep plain CI required. These fixtures do not establish
complete soundness for every syscall, ambient input, or output side effect.
See [exact case evidence](unit-metadata-audit.json).

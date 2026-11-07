# Anonymous descriptor callers: bounded negative result

[Gate 42364307324](https://github.com/pango07/quod/actions/runs/42364307324)
at `303cb8e33e037e44140e4d699cd976d99f01c230` **failed** when
`test:release` reached its 120-second command deadline. Build completed
successfully. The evidence answers the narrower question: anonymous
descriptor observations are not all handle-type queries. No replay policy
changed, no full window was dispatched, and the timeout was not extended.

| Native caller | Completed build | Captured release prefix only |
| --- | ---: | ---: |
| Node/libuv `uv_guess_handle` | 29 | 301 |
| Node `ResetStdio` | 6 | 105 |
| Unresolved Node offset `0x4b7439` | 9 | 95 |
| Unresolved Git callers | 0 | 92 |
| libc `_IO_file_doallocate` | 0 | 51 |
| **Anonymous metadata calls observed** | **44** | **644** |

These are calls, not distinct descriptors. Release is an incomplete prefix,
not a full-command total or representative sample. Its 26 unfinished anonymous
entries were joined to matching resumed lines by PID before attributing
frames; all 644 observed calls have frames. The collector's `result.json`
reports only build. The separately checked
[evidence](unit-descriptor-stacks.json) includes the partial release.

## What this changes

The pinned source's
[`uv_guess_handle`](https://github.com/nodejs/node/blob/v24.18.0/deps/uv/src/unix/tty.c#L393)
consumes `st_mode` from its stat result, then uses terminal and socket
operations to determine handle type. That narrower use does not describe
every observed caller.

Node's
[`ResetStdio`](https://github.com/nodejs/node/blob/v24.18.0/src/node.cc#L657)
compares the current descriptor's device and inode with saved startup values
before restoring `O_NONBLOCK` flags.
[`PlatformInit`](https://github.com/nodejs/node/blob/v24.18.0/src/node.cc#L525)
saves the initial stat and flags. The unnamed startup frame is consistent
with this path but has not been symbol-resolved; that attribution remains
unproved. The source blobs are
`b861072026796240c77fb56e0d133544ebd6f6db` (`tty.c`) and
`6f899ec0621ada45fb04c921359bc9b9ea79132b` (`node.cc`).

This rules out a blanket type-only representation. It does **not** prove
that absolute inode numbers must be equal across jobs. A correct abstraction
could potentially preserve the relation “this is the original descriptor”
and the required flag-restoration effect. Establishing that requires
descriptor ownership, lifetime and aliasing, exact trusted code identity,
all relevant consumers and effects, and unsupported-call fallback. The
current unit trace does not include all `close`, `dup` and `fcntl`
operations needed for that argument. A stack-name allowlist is insufficient.

The additional Git/libc callers, unresolved and truncated frames, and release
timeout prevent claiming a complete or affordable boundary. Do not ship stack
unwinding as an optimization or continue broad caller-by-caller exceptions
from this evidence. Define the supported operation and effect contract
before implementing narrower keys.

## Method and identities

The [probe](../../scripts/bench-vitest/unit-descriptor-stacks.py) invokes the
bundled engine through its actual `quod unit` wrapper, captured stdio,
sandbox and commands, once each for build and release. It narrows strace to
`fstat,newfstatat,statx,io_uring_setup` and requests at most eight frames.
This deliberately incomplete trace is **not** input evidence. After copying
a completed trace, the wrapper empties Quod's copy, forcing the production
parser's `its trace is empty` rejection. Build verified that rejection and
an empty record list. Release never reached that assertion; its retained
trace is explicitly named `incomplete`.

Build has 682 descriptor metadata entries, including 44 anonymous entries.
The frame limit can emit a truncation marker in addition to eight frames.
Some frames cannot be unwound or lack symbols. This diagnostic is not a
comparison against plain test results or a performance benchmark.

- Original c11: `5eaa7e5160736054d53f8b71e2c522835eb427e0`.
- Engine: `c12a7fe32f4957b6bd1cc09edbe63c7292b1c1d3`; Action: `beb12a8c9c32500ad2d19a87b10ee7cc7a95c197`.
- Engine ELF SHA-256: `d18f2618e859b5773968e11ab9269b7c72bc23817fb277e8ab0ff1e8839f9e8b`.
- Node 24.18.0 ELF SHA-256: `41a74efb34cbde5c7632cdac0cf8bd1a14d0b8d73dc1e82755014d9a9ce70f5c`.
- strace 6.19; runner `ubuntu26/20261101.186.1`.
- Artifact `13570808244`: 500,858 bytes; SHA-256 `d3ae9f61b5595de0b0da2c8b6a619b78d027eb725be8a4194dff5d6318609e41`.

The artifact's size/digest, build trace hash, rejection and zero records,
both anonymous-call counts and caller categories were independently checked.
The evidence retains raw-trace hashes and representative exact stacks,
without mistaking the failed gate for a pass.

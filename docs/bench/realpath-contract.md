# Realpath: resolved results require more than leaf existence

[Gate 42362384290](https://github.com/pango07/quod/actions/runs/42362384290)
reproduces four stale per-test passes in `bin-800771d`, packaged in Action
`57269ba1c1b43ae003dd64a5a5cc6c0df8f0e4b3`. Only a parent symlink changes;
the final file's bytes and type remain equal. The mutation is outside the
project, so shared project-directory metadata cannot mask the missing input.

| Observation | Before mutation | Quod after mutation | Plain after mutation |
| --- | --- | --- | --- |
| `realpathSync` | Pass | Replayed pass | Fail |
| `realpathSync.native` | Pass | Replayed pass | Fail |
| `realpath.native` callback | Pass | Replayed pass | Fail |
| `fs.promises.realpath` | Pass | Replayed pass | Fail |
| `realpath` callback | Pass | Executed failure | Fail |
| Content-only control | Pass | Replayed pass | Pass |

Both changed-input commands exit **1**, despite four wrong assertions.
Exit agreement alone would miss this defect. The benchmark's per-test
comparison remains necessary.

An [earlier attempt](https://github.com/pango07/quod/actions/runs/42362251258)
mutated the link inside the project. That changed shared `sm:` directory
metadata and caused all six files to run correctly. The strengthened fixture
has five genuine warm replays and five changed-input replays, with shared
inputs unchanged. The [verified evidence](realpath-contract.json) retains
all six reports, their digests and the independently compared states.

## Gated correction

The original `e:` input records leaf existence, type and a direct symlink's
target. It does not represent the path returned after resolving parent
links. The correction adds an `rp:` input with a bounded resolution graph:
path prefixes, recursively encountered link targets, lexical normalization
and native canonical results. Existing keys and internal stat observations
remain in place. This change does not authorize ignoring inode or timestamp
reads by other consumers.

The new input retains parent traversal through symlinks and avoids package-
version substitution. Errors, special files, non-UTF-8 link targets and
excessive expansion have unrepeatable states, preventing replay. Buffer/URL
arguments, proxy options and accessor options run normally; the recorder
leaves their conversion and getter evaluation to Node. Ordinary data options
and Node's internal CommonJS cache remain supported without removing their
previous input keys. A direct `require.resolve` regression covers the latter.

Quod's embedded code remains covered by its code hash. A Vite config bundle
is treated as a current-run artifact only when the main recorder observed a
fresh write at its normalized `.vite-temp` timestamp filename. Pre-existing
files and unrelated newly written files retain their resolution input.

The first release gate exposed the need for that artifact boundary: keying
a generated, deleted config filename prevented the existing metadata
regression from warming. Blanket options-object rejection then prevented the
CommonJS loader from recording; admitting ordinary data descriptors resolved
that compatibility issue without evaluating application accessors.

[Gate 42363290768](https://github.com/pango07/quod/actions/runs/42363290768)
at `c12a7fe32f4957b6bd1cc09edbe63c7292b1c1d3` has passed 122 Rust tests,
92 JavaScript tests and Clippy, followed by the release build and all embedded
binary regressions. The corrected binary warms five files, reruns all five
realpath tests after mutation and retains one content-control replay. Every
changed test state matches plain, with zero wrong results. Downloaded artifact
digests, binary identity and all six reports were independently checked.

[Action gate 42363721621](https://github.com/pango07/quod/actions/runs/42363721621)
also passed. It rebuilt source-stage Action `ef4745d` remotely and exercised
the actual npm/node shim. Its six realpath runs agree with plain after the
mutation and retain the content-control replay. The existing lineage,
upgrade, configuration, environment, descriptor, deferred-save and cleanup
checks passed as well. The lineage fixture promoted two records and ran
one plain audit, with no disagreement or revocation; these are fixture
counts, not activity in the full benchmark window.

Action `beb12a8c9c32500ad2d19a87b10ee7cc7a95c197` contains the exact verified
remote bundles and `bin-c12a7fe`. The benchmark branch pins this package;
its workload and original cold window are unchanged. No new full-window
measurement or performance claim follows from this regression.

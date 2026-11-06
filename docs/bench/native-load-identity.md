# Native artifact identity at loading

The async size optimization remains disabled. Commit `31bbd31` adds a
diagnostic that identifies a native addon before `process.dlopen`, holds
its file descriptor, and compares its device/inode with executable Linux
mappings after the call. It checks file identity and metadata again at
consumer checkpoints. This narrows the earlier post-run hashing evidence;
it does not grant native capability trust or authorize replay.

[Gate 42166985276](https://github.com/pango07/quod/actions/runs/42166985276)
passes on Ubuntu 26.04 image `20261101.186.1`, Node 24.18.0. All compilation
and test execution happened on GitHub runners.

| Check | Result |
| --- | ---: |
| Observer regression tests | 45 passed |
| Production proof regression tests | 65 passed |
| Native artifact regression | 1 passed, covering 7 scenarios |
| Actual driver file/project instances | 120 |
| Test states matching the established fingerprint | 4,131 |
| Size consumers with matched native artifacts at checkpoints | 120 / 120 |
| Native artifacts loaded / matched | 1 / 1 |
| Replayed files | 0 |

The native fixture covers a known binary, an unknown digest, modification
after loading, replacement after loading, a detected change subsequently
restored, replacement during the addon initializer, and a failed load.
It also checks that the real loader determines the result and that closing
the probe restores the original loader function.

The initializer replacement is the important counterexample: binary A
returns its own answer while replacing its pathname with binary B. A
post-run hash matches B even though A ran. The new probe captures A's
digest before loading and rejects the changed identity afterward.

In the real driver, the captured Rolldown binary digest is
`09efa62dc8d03fb62f06ab2aadbbc26bde416acb7da20a927f7d53b2bd440322`,
matching the locked registry artifact. Its held descriptor and pathname
retain the same checked metadata, and its device/inode matches an
executable mapping. The hash read uses `O_NOATIME` and observes unchanged
access time across that read. The report retains the mapping evidence.

This remains incomplete as a trust argument: checkpoint observations do
not exclude every transient concurrent change; mapped inode identity does
not prove private memory pages never changed; and an exact digest does not
establish which observations or side effects the native code can perform.
Unknown or failed observations remain diagnostic failures. No digest
allowlist is enabled in the production recorder.

The real driver keeps all 299 `sm:`, 257 `sl:` and 104 `st:` dependency keys
and matches the prior 4,131-state fingerprint. Production engine `081f20a`
and the packaged Action are unchanged. This is not a timing benchmark.

## Repository readiness

A non-blocking trial alongside an existing required plain CI job is the
appropriate next deployment scope. Repository compatibility, replay
agreement across real changes, and total cost including recording overhead
must be measured before relying on Quod to approve changes. The earlier
40–50% savings came from a superseded engine whose metadata handling was
found unsound; those numbers do not establish savings for this corrected
engine. Its full 12-commit window and best `floor + N s` hit remain unproven.

See [machine-readable evidence](native-load-identity.json).

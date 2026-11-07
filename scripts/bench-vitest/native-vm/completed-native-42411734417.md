# Completed native build and release repeats

The [seventh native VM investigation](https://github.com/pango07/quod/actions/runs/42411734417) completed two unchanged `npm run build` commands and two unchanged `npm run test:release` commands in separately booted diskless Linux guests. Both release executions passed all 20 original tests, with zero failures, cancellations, skips or todos. They ran the original release test files and their existing local Git and HTTP fixtures.

The independent data audit passes with zero issues. Each command pair agrees exactly on raw stdout, stderr, the complete `dist` tree and its metadata, and the recorded kernel entropy sample. Both pairs also have byte-identical complete UART serial files and final JSON responses. Every positive command finishes with zero remaining processes. These are actual retained repeat results for the declared normalized guest profile. No receipt, cache hit, ordinary-runner equivalence, full 120-file command result or 12-commit result is claimed.

| Actual command | First host time | Second host time | Exact captured effects |
| --- | ---: | ---: | --- |
| Build | 139.112 s | 137.447 s | 248 stdout bytes, 43 stderr bytes, 130 output entries, 1,118,349 file bytes |
| Release tests | 371.525 s | 371.523 s | 11,419 stdout bytes, zero stderr bytes, one empty output-root entry, 20/20 passing tests |

The repeated release output includes the same original terminal test duration, `50510.507625` guest milliseconds. This is recorded virtual-clock output, distinct from the measured host times above. Full output metadata includes ownership, permissions, inode/device/link counts and all three timestamps. All 15 real boundary controls pass, the deliberate detached descendant is rejected and reaped, and the six remote symlink controls and 12 remote parser controls pass.

Both releases reach the unchanged 64-record diagnostic cap. Their separately marked final root envelopes remain mandatory and valid, with `captureComplete: true`, zero leftovers and the original 20-test terminal counters. Progress records grant no completion authority.

The source pin is `1ac7bab428835914dd14c6f5907ce5c02dfe3445`; the original project remains `5eaa7e5160736054d53f8b71e2c522835eb427e0`. The fixed CPU, image, guest C, environment and progress caps were preserved. Only the explicit release watchdog permits 720 host seconds; build remains capped at 180 and boundary controls at 60. The whole GitHub job took 1,217 seconds, including setup, image preparation, audit, upload and post steps.

The [machine-readable report](completed-native-42411734417.json) retains source, image, run, artifact, result and reader hashes, all 65 artifact file hashes, exact phase results and all six earlier failures. Artifact 13590591522 is 187,402,743 bytes with SHA-256 `554c38e5590ca2a840bb92927fc30aa68bba4ed224e52ea72d54af8eebb7f5d4`. It contains 187,391,675 uncompressed bytes. The audit binds every extracted payload and the complete 65-file/eight-directory inventory to the raw pinned ZIP before reading evidence.

To reproduce the data audit, retain the ZIP alongside its extracted directory as `ROOT.zip`, and run the trusted [completed-native-audit.py](completed-native-audit.py):

```sh
python3 -I -S scripts/bench-vitest/native-vm/completed-native-audit.py \
  /verified/native-vm-42411734417 /tmp/native-vm-audit.json \
  --trusted-source /trusted/native-vm-source
```

The trusted source directory must contain the exact `build.py`, `guest-init.c`, `guest-negative.c`, `probe.py` and `audit.py` bytes from the source pin above. The reader checks their fixed hashes and regular-file types before invoking the trusted data auditor with isolated Python. The default is the reader's own source directory. A future checkout with changed runtime bytes must use a separate trusted export of the pinned source. Artifact data is never a source of executable reader code; no downloader sidecar is required to establish the pinned ZIP digest.

The earlier failures remain failures: three image-preparation failures preceded a 180-second release cutoff without telemetry, another 180-second cutoff with 26 progress records, and a 360-second cutoff with 59 records. Their source/artifact pins and verification status remain in the report and earlier component records. The oversized second partial staging archive was retained as API metadata and was not downloaded.

Exhaustive emulator host-open and negative-lookup closure remains unproved. `RNDADDENTROPY` mixes a seed rather than resetting the kernel pool, so this report relies on observed per-command entropy agreement. Raw `ldd` address text also varies between image metadata records despite equal recorded emulator/library bytes. Two matching actual executions do not establish future replay authority.

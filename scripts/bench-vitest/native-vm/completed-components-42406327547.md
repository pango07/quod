# Retained native build components

The [native guest gate](https://github.com/pango07/quod/actions/runs/42406327547) completed two actual unchanged `npm run build` commands in separately booted diskless Linux guests. An independent data-only audit verifies exact agreement in captured stdout (248 bytes), stderr (43 bytes), all 130 output-tree entries and 1,118,349 file bytes, including inode/device/link counts, ownership, permissions and all three timestamps. The actual kernel entropy samples also agree for the repeated build command. Both builds finish with zero remaining processes; their host times are 83.069s and 82.945s.

The 15 positive/negative boundary controls pass. A deliberately detached background descendant is detected, killed and reaped, and its command is rejected as incomplete. The source mount is read-only, with a private writable output tree, one fixed virtual CPU, fixed virtual clock/seed, no NIC/block disk/host mount, and a root-only output UART. The exact kernel, canonical initramfs, emulator bytes, host policy and invocation are retained.

The full investigation failed: the first unchanged release command reached the 180-second host watchdog without a final response. Its serial file is empty because this version retains child output inside the guest until final quiescence. There is no release completion, second release result, receipt admission or future replay claim. A separately pinned diagnostic variant retains bounded progress records while keeping the watchdog unchanged.

Two matching builds establish the recorded agreement for this explicit normalized guest profile. Ordinary-runner equivalence and exhaustive emulator host-open/negative-lookup closure remain unproved.

The [machine-readable evidence](completed-components-42406327547.json) includes immutable source and artifact pins. [components-audit.py](components-audit.py) first verifies the pinned ZIP and streams every archived payload against the complete extracted inventory, then independently reads raw envelopes, output-tree bytes/metadata, entropy and invocation records. It imports only the trusted checkout's data reader and executes no downloaded code.

# Stillpass

CI that refuses to repeat itself. This repo is the Phase 0 spike: can a
host-side filesystem access log serve as a complete, replayable input
manifest for a real CI step, at under 10% overhead?

Linux only. On macOS, run everything through the dev container:

```sh
scripts/dev.sh scripts/check-host.sh   # every Environment row
scripts/dev.sh cargo test --workspace  # unit, property, golden, integration
```

See `DECISIONS.md` for the record of choices made along the way.

# Native stack attribution diagnostic

[GitHub profile 42105950948](https://github.com/pango07/quod/actions/runs/42105950948)
ran the current recorder and a native structured-stack prototype on the same
runner, at convex-helpers c4 (`6b17e1514`). Source: `93fe69a`. The CLI binary
was the measured `b9e34fc`, from action `0af7cd7`; JavaScript overrides were
used only by the diagnostic.

| Full suite, recording | Seconds | Exit |
|---|---:|---:|
| Current caller attribution | 51.995 | 0 |
| Private-realm native stack | 52.523 | 0 |

The prototype was 0.529 seconds slower (1.02x). A single pair cannot
establish a stable small difference, but it supplies no reason to adopt the
change. All 4,129 individual test names and states agreed across 120 files.
The seven focused synchronous/async/deep-stack/eval/formatter checks also
passed on GitHub. The prototype produced **zero reusable recordings**:
all 120 files were explicitly marked unrecordable by the diagnostic.

Decision: leave the shipped recorder unchanged. This experiment did not
participate in the concurrently running 12-commit benchmark and does not
improve its measured savings. Native source attribution also needs a
separate correctness argument before adoption, beyond these examples.

# Savings and duration on the overview

The overview leads with recent workflow duration, time saved and estimated
runner cost saved. Replay counts, the execution chart and verification
counts live under Savings details. A disagreement remains visible as an
alert; a zero check count must not imply universal correctness.

Workflow duration comes from GitHub's workflow timestamps, not a sum of
parallel jobs or test durations. The card names its sample: completed runs
among the latest 100 workflows, inside the selected range. The demo remains
explicitly sample data. Missing measurements show an empty state.

Legacy `savedMinutes` measures avoided step execution before whole-job
overhead. It is labeled Execution avoided. It must not be called developer
waiting time or used to calculate dollar savings. The old fixed
`costSavedUsd = savedMinutes * 0.008` is no longer displayed on the overview.

Optional `TeamTotals.jobSavings` supplies a comparison over the same jobs,
attempts and time range. Include cold recordings, plain witnesses, audits,
failed runs and action/post overhead. For each job, round its runner time
according to the provider's billing rules before summing. Apply its actual
runner rate and free-runner treatment separately. Unknown pricing is null;
free public runners are $0. Keep regressions visible as extra time/cost.
An estimated baseline is identified as estimated. Do not add time and cost
percentages or combine a best-hit result with cumulative window savings.

The existing server does **not** yet provide whole-job comparisons or
customer-specific billing. This optional contract does not manufacture
them. Until that telemetry exists, net cost says Measuring, with the
avoided-work estimate and observed CI duration still available. Remaining
work is ingesting complete paired/estimated job baselines and billing data,
with coverage and provenance, before claiming customer net savings.

Changes are prepared on an experimental branch. No Cloud deployment or
production branch update is authorized by this work.

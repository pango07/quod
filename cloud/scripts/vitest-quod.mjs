// `npm test` with output that is the same on every identical run, for
// `quod run` (.github/workflows/quod-cloud.yml). Vitest's own reporters print
// the wall clock ("Start at", "Duration"), so two identical runs never
// compare byte-identical and Quod quarantines the step. This runs vitest with
// the JSON reporter into a temporary file and prints only the counts and, for
// each failure, the test and its message. The exit code is vitest's.
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, relative } from "node:path";

const dir = mkdtempSync(join(tmpdir(), "vitest-quod-"));
const out = join(dir, "report.json");
const run = spawnSync("npx", ["vitest", "run", "--configLoader", "runner", "--reporter=json", `--outputFile=${out}`, ...process.argv.slice(2)], {
  encoding: "utf8",
});
let report;
try {
  report = JSON.parse(readFileSync(out, "utf8"));
} catch {
  // No report: vitest did not get as far as running tests. Its own output says why.
  process.stdout.write(run.stdout ?? "");
  process.stderr.write(run.stderr ?? "");
  rmSync(dir, { recursive: true, force: true });
  process.exit(run.status ?? 1);
}
rmSync(dir, { recursive: true, force: true });
const files = [...report.testResults].sort((a, b) => a.name.localeCompare(b.name));
for (const f of files) {
  for (const t of f.assertionResults) {
    if (t.status !== "failed") continue;
    console.log(`FAIL ${relative(process.cwd(), f.name)} > ${t.fullName}`);
    for (const m of t.failureMessages ?? []) console.log(m);
  }
  if (f.status === "failed" && f.assertionResults.length === 0) console.log(`FAIL ${relative(process.cwd(), f.name)}\n${f.message ?? ""}`);
}
console.log(
  `test files: ${report.numTotalTestSuites - report.numFailedTestSuites} passed, ${report.numFailedTestSuites} failed; ` +
    `tests: ${report.numPassedTests} passed, ${report.numFailedTests} failed, ${report.numPendingTests + (report.numTodoTests ?? 0)} skipped, ${report.numTotalTests} total`,
);
process.exit(run.status ?? (report.success ? 0 : 1));

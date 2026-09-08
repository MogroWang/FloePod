import { test } from "node:test";
import assert from "node:assert/strict";
import { privacySummary } from "./privacy.ts";

test("skipped, failed and legacy scans cannot claim no known issues", () => {
  for (const result of [
    { filesSkipped: 1, filesChecked: 0 },
    { filesFailed: 1, filesChecked: 0 },
    { filesScanned: 3 },
  ]) {
    assert.equal(privacySummary({ ...result, issues: [] }).canReportNoKnownIssues, false);
  }
});

test("fully completed scans distinguish actual findings", () => {
  assert.equal(
    privacySummary({ filesChecked: 2, filesSkipped: 0, filesFailed: 0, issues: [] })
      .canReportNoKnownIssues,
    true,
  );
  assert.equal(
    privacySummary({
      filesChecked: 1,
      issues: [
        { path: "a.jpg", code: "exif-gps", severity: "high", message: "GPS", canClean: true },
      ],
    }).canReportNoKnownIssues,
    false,
  );
});

test("incomplete file details preserve machine status and explanatory reason", () => {
  const summary = privacySummary({
    filesChecked: 1,
    filesSkipped: 1,
    files: [
      { path: "a.png", status: "checked", reason: null },
      { path: "b.pdf", status: "skipped", reason: "超过大小限制" },
    ],
  });
  assert.deepEqual(summary.pending, [{ path: "b.pdf", status: "skipped", reason: "超过大小限制" }]);
});

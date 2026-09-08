import type { PrivacyScanResult } from "./types";

/** Old saved reports lacking completion evidence must never become a clean bill of health. */
export function privacySummary(scan: Partial<PrivacyScanResult>) {
  const checked = scan.filesChecked ?? 0;
  const skipped = scan.filesSkipped ?? 0;
  const failed = scan.filesFailed ?? 0;
  const incomplete = skipped > 0 || failed > 0 || scan.filesChecked === undefined;
  return {
    checked,
    skipped,
    failed,
    incomplete,
    canReportNoKnownIssues: !incomplete && (scan.issues?.length ?? 0) === 0,
    pending: (scan.files ?? []).filter((file) => file.status !== "checked"),
  };
}

import { writeFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
const {
  GITHUB_REPOSITORY: repository,
  RELEASE_SHA: sha,
  GITHUB_WORKFLOW_SHA: workflowSha,
  GITHUB_RUN_ID: run,
  GITHUB_RUN_ATTEMPT: attempt,
} = process.env;
if (
  !/^[0-9a-f]{40}$/.test(sha ?? "") ||
  execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim() !== sha
)
  throw Error("Unverified build source");
// workflow_run's GITHUB_SHA describes the workflow's base, not necessarily the
// merged PR being released. Record both identities instead of misattributing it.
writeFileSync(
  "dist/release-provenance.json",
  JSON.stringify(
    {
      buildDefinition: {
        buildType: "https://github.com/MogroWang/FloePod/build/windows/v1",
        externalParameters: {
          source: { uri: `git+https://github.com/${repository}`, digest: { gitCommit: sha } },
          workflow: {
            repository: `https://github.com/${repository}`,
            path: ".github/workflows/release.yml",
            ref: workflowSha,
          },
        },
        internalParameters: { github: { run_id: run, run_attempt: attempt } },
        resolvedDependencies: [
          { uri: `git+https://github.com/${repository}@${sha}`, digest: { gitCommit: sha } },
        ],
      },
      runDetails: {
        builder: {
          id: `https://github.com/${repository}/.github/workflows/release.yml@${workflowSha}`,
        },
        metadata: {
          invocationId: `https://github.com/${repository}/actions/runs/${run}/attempts/${attempt}`,
        },
      },
    },
    null,
    2,
  ),
);

import { writeFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
const {
  GITHUB_REPOSITORY: repository,
  RELEASE_SHA: sha,
  GITHUB_REF: githubRef,
  GITHUB_WORKFLOW_SHA: workflowSha,
  GITHUB_RUN_ID: run,
  GITHUB_RUN_ATTEMPT: attempt,
} = process.env;
if (
  !/^[0-9a-f]{40}$/.test(sha ?? "") ||
  !/^refs\//.test(githubRef ?? "") ||
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
        // The attestation service only accepts the official GitHub Actions
        // buildType for SLSA v1 provenance predicates (actions/attest#195)
        // and validates the predicate against that build type's shape, so
        // ref-typed fields carry the run's git ref while the commit SHA only
        // appears in digest.gitCommit. The verified merged source stays
        // recorded in externalParameters.source and resolvedDependencies.
        buildType: "https://actions.github.io/buildtypes/workflow/v1",
        externalParameters: {
          source: { uri: `git+https://github.com/${repository}`, digest: { gitCommit: sha } },
          workflow: {
            repository: `https://github.com/${repository}`,
            path: ".github/workflows/release.yml",
            ref: githubRef,
          },
        },
        internalParameters: {
          github: { run_id: run, run_attempt: attempt, workflow_sha: workflowSha },
        },
        resolvedDependencies: [
          { uri: `git+https://github.com/${repository}@${githubRef}`, digest: { gitCommit: sha } },
        ],
      },
      runDetails: {
        builder: {
          id: `https://github.com/${repository}/.github/workflows/release.yml@${githubRef}`,
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

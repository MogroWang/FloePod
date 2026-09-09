import { writeFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";

const REQUIRED_CLAIMS = [
  "repository",
  "ref",
  "workflow_ref",
  "job_workflow_ref",
  "event_name",
  "repository_id",
  "repository_owner_id",
  "runner_environment",
  "run_id",
  "run_attempt",
];

export async function getOIDCClaims() {
  const { ACTIONS_ID_TOKEN_REQUEST_URL: url, ACTIONS_ID_TOKEN_REQUEST_TOKEN: token } = process.env;
  if (!url || !token) throw Error("OIDC endpoint unavailable; the job needs id-token: write");
  const response = await fetch(`${url}&audience=sigstore`, {
    headers: { Authorization: `Bearer ${token}` },
    signal: AbortSignal.timeout(10_000),
  });
  if (!response.ok) throw Error(`OIDC token request failed: HTTP ${response.status}`);
  const { value } = await response.json();
  if (typeof value !== "string") throw Error("Malformed OIDC token response");
  return JSON.parse(Buffer.from(value.split(".")[1], "base64url").toString("utf8"));
}

export function buildPredicate({ claims, sha, serverURL, workflowSha }) {
  for (const key of REQUIRED_CLAIMS) if (!claims[key]) throw Error(`OIDC claim missing: ${key}`);
  // workflow_run's GITHUB_SHA describes the workflow's base, not necessarily the
  // merged PR being released. Record both identities instead of misattributing it.
  const repoURL = `${serverURL}/${claims.repository}`;
  const [workflowPath] = claims.workflow_ref.replace(`${claims.repository}/`, "").split("@");
  return {
    buildDefinition: {
      // The attestation service only accepts the official GitHub Actions
      // buildType for SLSA v1 provenance predicates (actions/attest#195) and
      // validates the predicate against that build type's shape: ref-typed
      // fields must equal the run's git ref, the commit SHA only appears in
      // digest.gitCommit, and internalParameters.github requires the run's
      // OIDC identity fields. Claims therefore mirror the official generator;
      // the verified merged source stays recorded in externalParameters.source
      // and resolvedDependencies, the workflow definition SHA in
      // internalParameters.
      buildType: "https://actions.github.io/buildtypes/workflow/v1",
      externalParameters: {
        source: { uri: `git+${repoURL}`, digest: { gitCommit: sha } },
        workflow: { repository: repoURL, path: workflowPath, ref: claims.ref },
      },
      internalParameters: {
        github: {
          event_name: claims.event_name,
          repository_id: claims.repository_id,
          repository_owner_id: claims.repository_owner_id,
          runner_environment: claims.runner_environment,
          run_id: claims.run_id,
          run_attempt: claims.run_attempt,
          workflow_sha: workflowSha,
        },
      },
      resolvedDependencies: [{ uri: `git+${repoURL}@${claims.ref}`, digest: { gitCommit: sha } }],
    },
    runDetails: {
      builder: { id: `${serverURL}/${claims.job_workflow_ref}` },
      metadata: {
        invocationId: `${serverURL}/${claims.repository}/actions/runs/${claims.run_id}/attempts/${claims.run_attempt}`,
      },
    },
  };
}

async function main() {
  const {
    RELEASE_SHA: sha,
    GITHUB_SERVER_URL: serverURL,
    GITHUB_WORKFLOW_SHA: workflowSha,
  } = process.env;
  if (
    !/^[0-9a-f]{40}$/.test(sha ?? "") ||
    execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim() !== sha
  )
    throw Error("Unverified build source");
  const claims = await getOIDCClaims();
  writeFileSync(
    "dist/release-provenance.json",
    JSON.stringify(buildPredicate({ claims, sha, serverURL, workflowSha }), null, 2),
  );
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href)
  await main();

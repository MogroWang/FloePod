import assert from "node:assert/strict";
import test from "node:test";
import { buildPredicate, getOIDCClaims } from "./release-provenance.mjs";

const claims = {
  repository: "MogroWang/FloePod",
  ref: "refs/heads/main",
  workflow_ref: "MogroWang/FloePod/.github/workflows/release.yml@refs/heads/main",
  job_workflow_ref: "MogroWang/FloePod/.github/workflows/release.yml@refs/heads/main",
  event_name: "workflow_run",
  repository_id: "10",
  repository_owner_id: "20",
  runner_environment: "github-hosted",
  run_id: "34361131519",
  run_attempt: "1",
};
const sha = "a".repeat(40);
const env = { claims, sha, serverURL: "https://github.com", workflowSha: sha };

test("predicate mirrors the official workflow/v1 provenance shape", () => {
  const { buildDefinition, runDetails } = buildPredicate(env);
  assert.equal(buildDefinition.buildType, "https://actions.github.io/buildtypes/workflow/v1");
  assert.deepEqual(buildDefinition.externalParameters.workflow, {
    repository: "https://github.com/MogroWang/FloePod",
    path: ".github/workflows/release.yml",
    ref: "refs/heads/main",
  });
  assert.deepEqual(buildDefinition.internalParameters.github, {
    event_name: "workflow_run",
    repository_id: "10",
    repository_owner_id: "20",
    runner_environment: "github-hosted",
    run_id: "34361131519",
    run_attempt: "1",
    workflow_sha: sha,
  });
  assert.deepEqual(buildDefinition.resolvedDependencies, [
    { uri: "git+https://github.com/MogroWang/FloePod@refs/heads/main", digest: { gitCommit: sha } },
  ]);
  assert.equal(
    runDetails.builder.id,
    "https://github.com/MogroWang/FloePod/.github/workflows/release.yml@refs/heads/main",
  );
  assert.equal(
    runDetails.metadata.invocationId,
    "https://github.com/MogroWang/FloePod/actions/runs/34361131519/attempts/1",
  );
});

test("verified merged source stays recorded alongside the official shape", () => {
  const { buildDefinition } = buildPredicate(env);
  assert.deepEqual(buildDefinition.externalParameters.source, {
    uri: "git+https://github.com/MogroWang/FloePod",
    digest: { gitCommit: sha },
  });
});

test("missing OIDC identity claims fail fast before upload", () => {
  for (const key of Object.keys(claims))
    assert.throws(
      () => buildPredicate({ ...env, claims: { ...claims, [key]: undefined } }),
      new RegExp(key),
    );
});

test("OIDC claims decode from the audience-scoped job token", async () => {
  const saved = {
    url: process.env.ACTIONS_ID_TOKEN_REQUEST_URL,
    token: process.env.ACTIONS_ID_TOKEN_REQUEST_TOKEN,
  };
  process.env.ACTIONS_ID_TOKEN_REQUEST_URL =
    "https://vstoken.example/api/auth/tokens?correlation_id=1";
  process.env.ACTIONS_ID_TOKEN_REQUEST_TOKEN = "job-token";
  const seen = [];
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async (url, options) => {
    seen.push([url, options.headers.Authorization]);
    return {
      ok: true,
      json: async () => ({
        value: `h.${Buffer.from(JSON.stringify(claims)).toString("base64url")}.s`,
      }),
    };
  };
  try {
    assert.deepEqual(await getOIDCClaims(), claims);
  } finally {
    globalThis.fetch = originalFetch;
    if (saved.url === undefined) delete process.env.ACTIONS_ID_TOKEN_REQUEST_URL;
    else process.env.ACTIONS_ID_TOKEN_REQUEST_URL = saved.url;
    if (saved.token === undefined) delete process.env.ACTIONS_ID_TOKEN_REQUEST_TOKEN;
    else process.env.ACTIONS_ID_TOKEN_REQUEST_TOKEN = saved.token;
  }
  assert.equal(
    seen[0][0],
    "https://vstoken.example/api/auth/tokens?correlation_id=1&audience=sigstore",
  );
  assert.equal(seen[0][1], "Bearer job-token");
});

test("OIDC fetch fails fast without the token endpoint", async () => {
  const saved = {
    url: process.env.ACTIONS_ID_TOKEN_REQUEST_URL,
    token: process.env.ACTIONS_ID_TOKEN_REQUEST_TOKEN,
  };
  delete process.env.ACTIONS_ID_TOKEN_REQUEST_URL;
  delete process.env.ACTIONS_ID_TOKEN_REQUEST_TOKEN;
  try {
    await assert.rejects(getOIDCClaims(), /id-token: write/);
  } finally {
    if (saved.url !== undefined) process.env.ACTIONS_ID_TOKEN_REQUEST_URL = saved.url;
    if (saved.token !== undefined) process.env.ACTIONS_ID_TOKEN_REQUEST_TOKEN = saved.token;
  }
});

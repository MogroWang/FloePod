import assert from "node:assert/strict";
import test from "node:test";
import { resolveRelease } from "./release-identity.mjs";
const sha = "a".repeat(40);
function fixture() {
  return {
    "": { id: 1, default_branch: "main" },
    "/actions/workflows/merge-notification.yml": { id: 9 },
    "/actions/runs/123": {
      id: 123,
      workflow_id: 9,
      path: ".github/workflows/merge-notification.yml",
      display_title: `Merged PR #12 at ${sha}`,
      event: "pull_request_target",
      conclusion: "success",
      repository: { id: 1 },
      head_repository: { id: 1 },
    },
    "/pulls/12": {
      number: 12,
      merged: true,
      merged_at: "2026-09-08T01:00:00Z",
      state: "closed",
      merge_commit_sha: sha,
      base: { ref: "main", repo: { id: 1 } },
    },
    [`/compare/${sha}...main`]: { status: "ahead", merge_base_commit: { sha } },
  };
}
test("same-repo, fork and Dependabot PRs all resolve to their own merge, even after main advances", async () => {
  for (const author of ["owner", "fork-user", "dependabot[bot]"]) {
    const data = fixture();
    data["/pulls/12"].user = { login: author };
    if (author === "fork-user") {
      data["/actions/runs/123"].head_repository.id = 2;
      data["/pulls/12"].head = { repo: { id: 2 }, sha: "b".repeat(40) };
    }
    assert.deepEqual(await resolveRelease(async (path) => data[path], 123), {
      sha,
      pr: "12",
      notification: "123",
    });
  }
});
test("reject unmerged, forged workflow, head code, changed merge identity and non-ancestor", async () => {
  const mutations = [
    (d) => (d["/pulls/12"].merged = false),
    (d) => (d["/actions/runs/123"].workflow_id = 10),
    (d) => (d["/actions/runs/123"].event = "pull_request"),
    (d) => (d["/actions/runs/123"].repository.id = 2),
    (d) => (d["/actions/runs/123"].display_title = "Merged PR #12 at $(evil)"),
    (d) => (d["/pulls/12"].merge_commit_sha = "b".repeat(40)),
    (d) => (d["/pulls/12"].base.ref = "other"),
    (d) => (d[`/compare/${sha}...main`].status = "diverged"),
  ];
  for (const mutate of mutations) {
    const data = fixture();
    mutate(data);
    await assert.rejects(resolveRelease(async (path) => data[path], 123));
  }
});

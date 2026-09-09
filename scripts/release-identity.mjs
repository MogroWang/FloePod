import { resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { githubApi, eventPayload, output } from "./github-api.mjs";

// The notification run name contains only GitHub's numeric PR identity and the
// merge SHA. Neither author, branch name nor PR title is executable input.
export async function resolveRelease(api, runId) {
  if (!/^[1-9]\d*$/.test(String(runId))) throw Error("Invalid notification run ID");
  const [repository, workflow, run] = await Promise.all([
    api(""),
    api("/actions/workflows/merge-notification.yml"),
    api(`/actions/runs/${runId}`),
  ]);
  const identity = /^Merged PR #([1-9]\d*) at ([0-9a-f]{40})$/.exec(run.display_title ?? "");
  if (
    !identity ||
    run.id !== Number(runId) ||
    run.workflow_id !== workflow.id ||
    run.path !== ".github/workflows/merge-notification.yml" ||
    run.event !== "pull_request_target" ||
    run.conclusion !== "success" ||
    run.repository?.id !== repository.id
  ) {
    throw Error("Run is not a successful trusted merge notification");
  }
  const [, number, sha] = identity;
  // head_repository describes PR provenance and may be a fork. Trust only the
  // target workflow identity and the independently verified merged base commit.
  const pr = await api(`/pulls/${number}`);
  if (
    pr.number !== Number(number) ||
    !pr.merged ||
    !pr.merged_at ||
    pr.state !== "closed" ||
    pr.base.repo.id !== repository.id ||
    pr.base.ref !== repository.default_branch ||
    pr.merge_commit_sha !== sha
  ) {
    throw Error("Notification does not identify this PR's immutable merged commit");
  }
  const comparison = await api(
    `/compare/${sha}...${encodeURIComponent(repository.default_branch)}`,
  );
  if (
    !["ahead", "identical"].includes(comparison.status) ||
    comparison.merge_base_commit?.sha !== sha
  ) {
    throw Error("Merged commit is not an ancestor of the current default branch");
  }
  return { sha, pr: number, notification: String(runId) };
}
if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const event = eventPayload();
  if (!["workflow_run", "workflow_dispatch"].includes(process.env.GITHUB_EVENT_NAME))
    throw Error("Unsupported release trigger");
  const identity = await resolveRelease(
    githubApi(),
    event.workflow_run?.id ?? event.inputs?.notification_run_id,
  );
  output(identity);
  console.log(`Verified merged PR #${identity.pr} at ${identity.sha}`);
}

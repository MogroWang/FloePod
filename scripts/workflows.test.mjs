import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import test from "node:test";
import YAML from "yaml";
import Ajv from "ajv";

// SchemaStore github-workflow.json, retrieved 2026-09-08. Kept locally so fork
// PRs validate the same schema without network access or mutable CI tooling.
const schema = JSON.parse(
  readFileSync(new URL("../contracts/github-workflow.schema.json", import.meta.url), "utf8"),
);
const validate = new Ajv({ strict: false, allErrors: true, validateFormats: false }).compile(
  schema,
);
const workflow = (name) =>
  YAML.parse(readFileSync(new URL(`../.github/workflows/${name}.yml`, import.meta.url), "utf8"));
test("all Actions workflows conform to the pinned current schema and immutable action references", () => {
  for (const name of readdirSync(new URL("../.github/workflows", import.meta.url))) {
    if (!name.endsWith(".yml")) continue;
    const value = workflow(name.slice(0, -4));
    assert.ok(validate(value), `${name}: ${JSON.stringify(validate.errors)}`);
    for (const job of Object.values(value.jobs))
      for (const step of job.steps ?? []) {
        if (step.uses && !step.uses.startsWith("./"))
          assert.match(step.uses, /^[\w./-]+@[0-9a-f]{40}$/);
      }
  }
});
test("release has a serial non-dropping queue and no broad push/tag chain or repeated CI", () => {
  const release = workflow("release"),
    notification = workflow("merge-notification"),
    ci = workflow("ci");
  assert.deepEqual(release.concurrency, {
    group: "floepod-release",
    "cancel-in-progress": false,
    queue: "max",
  });
  assert.ok(release.on.workflow_run);
  assert.equal(release.on.push, undefined);
  assert.deepEqual(notification.on.pull_request_target.types, ["closed"]);
  assert.match(notification["run-name"], /merge_commit_sha/);
  assert.match(notification.jobs.merged.if, /merged == true/);
  for (const step of notification.jobs.merged.steps) {
    assert.equal(step.uses, undefined);
    assert.equal(step.env, undefined);
  }
  assert.ok(ci.on.merge_group);
  assert.deepEqual(ci.permissions, { contents: "read" });
  for (const job of Object.values(ci.jobs)) assert.equal(job.permissions, undefined);
  const releaseCommands = Object.values(release.jobs)
    .flatMap((job) => job.steps ?? [])
    .map((step) => step.run ?? "")
    .join("\n");
  assert.doesNotMatch(
    releaseCommands,
    /cargo (test|clippy|fmt|audit)|pnpm (test|audit|format:check|ipc:check)/,
  );
  assert.equal(release.jobs.publish.steps[0].with.ref, "${{ needs.resolve.outputs.sha }}");
  assert.equal(release.jobs.publish.steps[0].with["persist-credentials"], false);
});

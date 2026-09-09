import { githubApi, eventPayload } from "./github-api.mjs";
import { verifyVersions, requireNewVersion } from "./project-version.mjs";

const version = verifyVersions();
const event = eventPayload();
if (!["pull_request", "merge_group"].includes(process.env.GITHUB_EVENT_NAME))
  throw Error("Version gate requires PR or merge_group");
const api = githubApi();
const repository = await api("");
const expected =
  event.pull_request?.base.ref ?? event.merge_group?.base_ref?.replace(/^refs\/heads\//, "");
if (expected !== repository.default_branch)
  throw Error("Version gate only accepts the current default branch");
// Resolve the current base once, then read its immutable snapshot. A queued PR must
// re-run this gate after its predecessor merges; required checks enforce that rule.
const base = await api(`/commits/${encodeURIComponent(repository.default_branch)}`);
const file = await api(`/contents/package.json?ref=${base.sha}`);
const baseVersion = JSON.parse(Buffer.from(file.content, "base64").toString("utf8")).version;
const latest = await api("/releases/latest", { missing: true });
const tags = [];
for (const tag of [`v${version}`, version]) {
  if (await api(`/git/ref/tags/${encodeURIComponent(tag)}`, { missing: true })) tags.push(tag);
}
requireNewVersion(version, baseVersion, latest?.tag_name, tags);
console.log(
  `Version ${version} passes against current base ${base.sha} (${baseVersion}) and latest Release ${latest?.tag_name ?? "none"}`,
);

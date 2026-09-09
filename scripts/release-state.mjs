import { execFileSync } from "node:child_process";
import { readFileSync, readdirSync, statSync } from "node:fs";
import { createHash } from "node:crypto";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { githubApi, output } from "./github-api.mjs";
import { verifyVersions, compareVersions } from "./project-version.mjs";

export function validateRelease(release, tag, sha) {
  if (release.tag_name !== tag || !release.body?.includes(`<!-- floepod-source:${sha} -->`))
    throw Error("Existing Release belongs to another source or publisher");
}
export async function verifyRemote(api, release, expected) {
  const assets = await api(`/releases/${release.id}/assets?per_page=100`);
  if (assets.length !== expected.length) throw Error("Release asset count mismatch");
  for (const item of expected) {
    const asset = assets.find((asset) => asset.name === item.name);
    if (
      !asset ||
      asset.state !== "uploaded" ||
      asset.size !== item.size ||
      asset.digest !== `sha256:${item.sha256}`
    )
      throw Error(`Release asset integrity mismatch: ${item.name}`);
  }
}
async function remoteBytes(asset) {
  const response = await fetch(asset.browser_download_url, { signal: AbortSignal.timeout(60_000) });
  if (!response.ok) throw Error(`Cannot verify published asset: HTTP ${response.status}`);
  return Buffer.from(await response.arrayBuffer());
}
async function verifyPublished(api, release, sha, version) {
  const assets = await api(`/releases/${release.id}/assets?per_page=100`);
  const sumsAsset = assets.find((asset) => asset.name === "SHA256SUMS.txt");
  const manifestAsset = assets.find((asset) => asset.name === "release-manifest.json");
  if (!sumsAsset || !manifestAsset || assets.length !== 7)
    throw Error("Published Release is incomplete");
  const sums = await remoteBytes(sumsAsset),
    manifest = await remoteBytes(manifestAsset);
  const data = JSON.parse(manifest);
  if (data.commit !== sha || data.version !== version)
    throw Error("Published source/version mismatch");
  const expected = sums
    .toString("utf8")
    .trim()
    .split(/\r?\n/)
    .map((line) => {
      const match = /^([0-9a-f]{64})  ([A-Za-z0-9_. -]+)$/.exec(line);
      if (!match) throw Error("Invalid published checksum list");
      const asset = assets.find((asset) => asset.name === match[2]);
      if (!asset) throw Error("Published checksum references missing asset");
      return { name: match[2], size: asset.size, sha256: match[1] };
    });
  if (
    new Set(expected.map((item) => item.name)).size !== 6 ||
    expected.some((item) => item.name === "SHA256SUMS.txt")
  )
    throw Error("Invalid checksum coverage");
  if (
    expected.find((item) => item.name === "release-manifest.json")?.sha256 !==
    createHash("sha256").update(manifest).digest("hex")
  )
    throw Error("Manifest checksum mismatch");
  expected.push({
    name: "SHA256SUMS.txt",
    size: sums.length,
    sha256: createHash("sha256").update(sums).digest("hex"),
  });
  await verifyRemote(api, release, expected);
}
export async function ensureTag(api, tag, sha) {
  const ref = await api(`/git/ref/tags/${tag}`, { missing: true });
  if (ref) {
    let object = ref.object;
    for (let depth = 0; object.type === "tag" && depth < 8; depth++)
      object = (await api(`/git/tags/${object.sha}`)).object;
    if (object.type !== "commit" || object.sha !== sha)
      throw Error(`Tag ${tag} is already owned by another commit`);
    return;
  }
  await api("/git/refs", { method: "POST", body: { ref: `refs/tags/${tag}`, sha } });
}
async function main() {
  const api = githubApi(),
    version = verifyVersions(),
    sha = process.env.RELEASE_SHA;
  if (!/^[0-9a-f]{40}$/.test(sha ?? "")) throw Error("Missing verified source SHA");
  if (execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim() !== sha)
    throw Error("Checked-out HEAD differs from verified merged commit");
  const tag = `v${version}`;
  const legacy = await api(`/git/ref/tags/${version}`, { missing: true });
  if (legacy) throw Error("Version already has an unprefixed historical tag");
  const release = await api(`/releases/tags/${tag}`, { missing: true });
  if (release) validateRelease(release, tag, sha);
  if (process.argv.includes("--prepare")) {
    await ensureTag(api, tag, sha);
    if (release && !release.draft) await verifyPublished(api, release, sha, version);
    output({ version, tag, published: Boolean(release && !release.draft) });
    return;
  }
  if (!process.argv.includes("--publish")) throw Error("Expected --prepare or --publish");
  if (release && !release.draft) {
    await verifyPublished(api, release, sha, version);
    return;
  }
  await ensureTag(api, tag, sha);
  const directory = "dist/release";
  const expected = readdirSync(directory)
    .sort()
    .map((name) => ({
      name,
      size: statSync(join(directory, name)).size,
      sha256: createHash("sha256")
        .update(readFileSync(join(directory, name)))
        .digest("hex"),
    }));
  if (expected.length !== 7) throw Error("Expected five packages, manifest, and checksums");
  const manifest = JSON.parse(readFileSync(join(directory, "release-manifest.json"), "utf8"));
  if (manifest.commit !== sha || manifest.version !== version)
    throw Error("Local asset source/version mismatch");
  const notes = await api("/releases/generate-notes", {
    method: "POST",
    body: { tag_name: tag, target_commitish: sha },
  });
  const body = `${notes.body}\n\n源码提交：\`${sha}\`；PR #${process.env.RELEASE_PR}。\n\nAuthenticode：${manifest.signing === "verified" ? "已签名并验证" : "未签名（未配置签名证书）"}。SHA-256 与构建来源证明对应最终发布文件。\n\n<!-- floepod-source:${sha} -->`;
  const draft =
    release ??
    (await api("/releases", {
      method: "POST",
      body: { tag_name: tag, target_commitish: sha, name: `FloePod ${version}`, body, draft: true },
    }));
  // An interrupted draft can be rebuilt. Published assets are never replaced.
  execFileSync(
    "gh",
    [
      "release",
      "upload",
      tag,
      ...expected.map((item) => join(directory, item.name)),
      "--clobber",
      "--repo",
      process.env.GITHUB_REPOSITORY,
    ],
    { stdio: "inherit" },
  );
  await verifyRemote(api, draft, expected);
  const latest = await api("/releases/latest", { missing: true });
  const makeLatest = !latest || compareVersions(version, latest.tag_name.replace(/^v/, "")) > 0;
  const published = await api(`/releases/${draft.id}`, {
    method: "PATCH",
    body: { draft: false, body, make_latest: String(makeLatest) },
  });
  await verifyRemote(api, published, expected);
  console.log(`Published and verified ${published.html_url}`);
}
if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href)
  await main();

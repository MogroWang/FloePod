import assert from "node:assert/strict";
import test from "node:test";
import { ensureTag, validateRelease, verifyRemote } from "./release-state.mjs";
test("tag retries are idempotent and never move another commit's tag", async () => {
  const sha = "a".repeat(40),
    calls = [];
  await ensureTag(
    async (path, options) => {
      calls.push([path, options]);
      return null;
    },
    "v1.6.1",
    sha,
  );
  assert.equal(calls[1][1].body.sha, sha);
  await ensureTag(async () => ({ object: { type: "commit", sha } }), "v1.6.1", sha);
  await assert.rejects(
    ensureTag(async () => ({ object: { type: "commit", sha: "b".repeat(40) } }), "v1.6.1", sha),
  );
});
test("release retries require original provenance and complete matching assets", async () => {
  assert.throws(() => validateRelease({ tag_name: "v1.0.0", body: "foreign" }, "v1.0.0", "abc"));
  const expected = [{ name: "app.exe", size: 10, sha256: "abc" }];
  const asset = { name: "app.exe", size: 10, state: "uploaded", digest: "sha256:abc" };
  await verifyRemote(async () => [asset], { id: 1 }, expected);
  for (const assets of [
    [],
    [asset, asset],
    [{ ...asset, state: "starter" }],
    [{ ...asset, digest: "sha256:other" }],
  ])
    await assert.rejects(verifyRemote(async () => assets, { id: 1 }, expected));
});

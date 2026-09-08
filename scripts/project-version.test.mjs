import assert from "node:assert/strict";
import test from "node:test";
import {
  parseVersion,
  compareVersions,
  verifyVersions,
  requireNewVersion,
  projectVersions,
} from "./project-version.mjs";

test("strict semantic versions and precedence", () => {
  for (const value of ["01.2.3", "1.2", "v1.2.3", "1.2.3-01", "1.2.3+", " 1.2.3", "1.2.3\n"])
    assert.throws(() => parseVersion(value));
  const ordered = [
    "1.0.0-alpha",
    "1.0.0-alpha.1",
    "1.0.0-alpha.beta",
    "1.0.0-beta",
    "1.0.0-beta.2",
    "1.0.0-beta.11",
    "1.0.0-rc.1",
    "1.0.0",
    "1.0.1",
    "1.10.0",
    "2.0.0",
  ];
  for (let i = 1; i < ordered.length; i++)
    assert.equal(compareVersions(ordered[i - 1], ordered[i]), -1);
  assert.equal(compareVersions("1.0.0+a", "1.0.0+b"), 0);
});
test("source files must all match, including the root lockfile package", () => {
  const versions = projectVersions();
  assert.equal(verifyVersions(versions), versions["package.json"]);
  for (const path of Object.keys(versions))
    assert.throws(() => verifyVersions({ ...versions, [path]: "0.0.0" }));
});
test("latest base, Release and tag collisions reject duplicate PR versions", () => {
  requireNewVersion("1.6.1", "1.6.0", "v1.4.0", []);
  assert.throws(() => requireNewVersion("1.6.1", "1.6.1", "v1.4.0", []));
  assert.throws(() => requireNewVersion("1.6.1", "1.6.0", "v1.6.2", []));
  for (const tag of ["v1.6.1", "1.6.1"])
    assert.throws(() => requireNewVersion("1.6.1", "1.6.0", null, [tag]));
});

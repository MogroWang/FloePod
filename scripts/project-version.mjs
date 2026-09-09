import { readFileSync } from "node:fs";
import { fileURLToPath, pathToFileURL } from "node:url";
import { resolve } from "node:path";

export const root = fileURLToPath(new URL("../", import.meta.url));
const stable = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;
const semver =
  /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-((?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*)(?:\.(?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*))*))?(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$/;
export function parseVersion(value) {
  if (typeof value !== "string") throw Error("Version must be a string");
  const match = semver.exec(value);
  if (!match) throw Error(`Invalid semantic version: ${value}`);
  return { core: match.slice(1, 4).map(BigInt), pre: match[4]?.split(".") ?? [] };
}
export function compareVersions(a, b) {
  const left = parseVersion(a),
    right = parseVersion(b);
  for (let i = 0; i < 3; i++)
    if (left.core[i] !== right.core[i]) return left.core[i] < right.core[i] ? -1 : 1;
  if (!left.pre.length || !right.pre.length)
    return Number(!left.pre.length) - Number(!right.pre.length);
  for (let i = 0; i < Math.max(left.pre.length, right.pre.length); i++) {
    const a = left.pre[i],
      b = right.pre[i];
    if (a === b) continue;
    if (a === undefined || b === undefined) return a === undefined ? -1 : 1;
    const numericA = /^\d+$/.test(a),
      numericB = /^\d+$/.test(b);
    if (numericA && numericB) return BigInt(a) < BigInt(b) ? -1 : 1;
    if (numericA !== numericB) return numericA ? -1 : 1;
    return a < b ? -1 : 1;
  }
  return 0;
}
export function projectVersions(read = (path) => readFileSync(resolve(root, path), "utf8")) {
  const cargo = read("src-tauri/Cargo.toml")
    .split(/^\[package\]\s*$/m)[1]
    ?.split(/^\[/m)[0];
  const lock = read("src-tauri/Cargo.lock")
    .split(/^\[\[package\]\]\s*$/m)
    .find((section) => /^name\s*=\s*"floe-pod"\s*$/m.test(section));
  const value = (section) => section?.match(/^version\s*=\s*"([^"]+)"\s*$/m)?.[1];
  return {
    "package.json": JSON.parse(read("package.json")).version,
    "src-tauri/tauri.conf.json": JSON.parse(read("src-tauri/tauri.conf.json")).version,
    "src-tauri/Cargo.toml": value(cargo),
    "src-tauri/Cargo.lock": value(lock),
    "README.md": read("README.md").match(/当前源码版本 \*\*([^*]+)\*\*/)?.[1],
  };
}
export function verifyVersions(versions = projectVersions()) {
  const version = versions["package.json"];
  parseVersion(version);
  if (
    !stable.test(version) ||
    version.split(".").some((part, index) => BigInt(part) > [255n, 255n, 65535n][index])
  )
    throw Error("Windows MSI/MSIX installers require a stable version within 255.255.65535");
  for (const [path, value] of Object.entries(versions))
    if (value !== version) throw Error(`Version mismatch: ${path}=${value}; expected ${version}`);
  return version;
}
export function requireNewVersion(version, baseVersion, latestRelease, tags) {
  if (compareVersions(version, baseVersion) <= 0)
    throw Error(`Version ${version} must be greater than current default branch ${baseVersion}`);
  if (latestRelease && compareVersions(version, latestRelease.replace(/^v/, "")) <= 0)
    throw Error(`Version ${version} must be greater than latest Release ${latestRelease}`);
  if (tags.some((tag) => tag.replace(/^v/, "") === version))
    throw Error(`Version ${version} already has a tag`);
}
if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href)
  console.log(`Project versions agree: ${verifyVersions()}`);

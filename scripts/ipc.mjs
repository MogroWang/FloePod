import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const write = process.argv.includes("--write");
const cwd = fileURLToPath(new URL("../", import.meta.url));
const env = { ...process.env };
delete env.FLOEPOD_UPDATE_CONTRACT;
if (write) env.FLOEPOD_UPDATE_CONTRACT = "1";
for (const [command, args] of [
  [
    "cargo",
    [
      "test",
      "--manifest-path",
      "src-tauri/Cargo.toml",
      "--locked",
      "--lib",
      "ipc_schema_matches_checked_in_contract",
    ],
  ],
  [process.execPath, ["scripts/generate-ipc.mjs", ...(write ? [] : ["--check"])]],
]) {
  const result = spawnSync(command, args, { cwd, env, stdio: "inherit" });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
}

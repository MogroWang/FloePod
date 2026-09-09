import { appendFileSync, readFileSync } from "node:fs";

export function githubApi(env = process.env, request = fetch) {
  const repository = env.GITHUB_REPOSITORY;
  if (!/^[\w.-]+\/[\w.-]+$/.test(repository ?? "")) throw Error("Invalid repository");
  const base = `${env.GITHUB_API_URL ?? "https://api.github.com"}/repos/${repository}`;
  return async (path, { method = "GET", body, missing = false } = {}) => {
    const response = await request(`${base}${path}`, {
      method,
      headers: {
        Accept: "application/vnd.github+json",
        ...(env.GH_TOKEN ? { Authorization: `Bearer ${env.GH_TOKEN}` } : {}),
        "X-GitHub-Api-Version": "2022-11-28",
        "Content-Type": "application/json",
      },
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
      signal: AbortSignal.timeout(60_000),
    });
    if (missing && response.status === 404) return null;
    if (!response.ok) throw Error(`GitHub ${method} ${path}: HTTP ${response.status}`);
    return response.status === 204 ? null : response.json();
  };
}
export function eventPayload() {
  return JSON.parse(readFileSync(process.env.GITHUB_EVENT_PATH, "utf8"));
}
export function output(values) {
  for (const [name, value] of Object.entries(values)) {
    if (!/^[\w.-]+$/.test(String(value))) throw Error(`Unsafe output ${name}`);
    appendFileSync(process.env.GITHUB_OUTPUT, `${name}=${value}\n`);
  }
}

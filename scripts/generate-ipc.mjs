import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const root = new URL("../", import.meta.url);
const contract = JSON.parse(readFileSync(new URL("contracts/ipc.json", root), "utf8"));
const named = new Map();
for (const schema of [
  ...Object.values(contract.commands).map((c) => c.result),
  ...Object.values(contract.events),
]) {
  for (const [name, definition] of Object.entries(schema.$defs ?? {})) named.set(name, definition);
  if (schema.type === "object" && schema.title) named.set(schema.title, schema);
}
const pascal = (name) =>
  name
    .split(/[-_]/)
    .map((part) => part[0].toUpperCase() + part.slice(1))
    .join("");

/** Fail closed on an unhandled schema construct; never silently emit any. */
export function toType(schema, definitions = {}, references = false) {
  if (schema === true) return "unknown";
  if (schema === false) return "never";
  if (schema.$ref) {
    const name = schema.$ref.replace("#/$defs/", "");
    if (references && named.has(name)) return name;
    if (!definitions[name]) throw new Error(`Unresolved schema reference ${schema.$ref}`);
    return toType(definitions[name], definitions, references);
  }
  if (schema.enum) return schema.enum.map((value) => JSON.stringify(value)).join(" | ");
  if (Object.hasOwn(schema, "const")) return JSON.stringify(schema.const);
  if (schema.anyOf || schema.oneOf)
    return (schema.anyOf ?? schema.oneOf)
      .map((s) => `(${toType(s, definitions, references)})`)
      .join(" | ");
  if (schema.allOf)
    return schema.allOf.map((s) => `(${toType(s, definitions, references)})`).join(" & ");
  if (Array.isArray(schema.type))
    return schema.type
      .map((type) => toType({ ...schema, type }, definitions, references))
      .join(" | ");
  switch (schema.type) {
    case "null":
      return "null";
    case "boolean":
    case "string":
      return schema.type;
    case "integer":
    case "number":
      return "number";
    case "array":
      if (!schema.items) throw new Error("Untyped array in IPC schema");
      return `Array<${toType(schema.items, definitions, references)}>`;
    case "object": {
      const properties = Object.entries(schema.properties ?? {}).map(
        ([key, value]) =>
          `${JSON.stringify(key)}${schema.required?.includes(key) ? "" : "?"}: ${toType(value, definitions, references)};`,
      );
      return properties.length ? `{ ${properties.join(" ")} }` : "Record<string, unknown>";
    }
    case undefined:
      if (
        Object.keys(schema).every((key) =>
          ["$schema", "title", "description", "default"].includes(key),
        )
      )
        return "unknown";
  }
  throw new Error(`Unsupported IPC schema: ${JSON.stringify(schema)}`);
}

const header =
  "// Generated from Rust signatures and Serde schemas. Run pnpm ipc:generate; do not edit.\n";
let types = header;
for (const [name, schema] of [...named].sort(([a], [b]) => a.localeCompare(b))) {
  types += `export type ${name} = ${toType(schema, schema.$defs ?? Object.fromEntries(named), true)};\n`;
}
types += "\nexport interface CommandContract {\n";
for (const [name, command] of Object.entries(contract.commands)) {
  const args = Object.entries(command.args)
    .map(
      ([key, schema]) =>
        `${JSON.stringify(key)}${command.optionalArgs.includes(key) ? "?" : ""}: ${toType(schema, schema.$defs)};`,
    )
    .join(" ");
  const result =
    command.result.type === "null" ? "void" : toType(command.result, command.result.$defs, true);
  types += `  ${JSON.stringify(name)}: { args: { ${args} }; result: ${result} };\n`;
}
types += "}\n\nexport interface EventContract {\n";
for (const [name, schema] of Object.entries(contract.events))
  types += `  ${JSON.stringify(name)}: ${schema.type === "null" ? "void" : toType(schema, schema.$defs, true)};\n`;
types += "}\n";
const commands =
  header +
  "export const Commands = {\n" +
  Object.keys(contract.commands)
    .map((name) => `  ${pascal(name)}: ${JSON.stringify(name)},`)
    .join("\n") +
  "\n} as const;\nexport type CommandName = typeof Commands[keyof typeof Commands];\n";
const events =
  header +
  "export const Events = {\n" +
  Object.keys(contract.events)
    .map((name) => `  ${pascal(name.replace("floepod://", ""))}: ${JSON.stringify(name)},`)
    .join("\n") +
  "\n} as const;\n";
for (const [path, text] of [
  ["src/ipc/generated.ts", types],
  ["src/ipc/commands.ts", commands],
  ["src/ipc/eventNames.ts", events],
]) {
  const file = new URL(path, root);
  if (process.argv.includes("--check")) {
    if (readFileSync(file, "utf8").replaceAll("\r\n", "\n") !== text)
      throw new Error(`Stale generated IPC: ${fileURLToPath(file)}`);
  } else writeFileSync(file, text);
}
console.log(
  `IPC: ${Object.keys(contract.commands).length} commands, ${Object.keys(contract.events).length} events, ${named.size} payload types`,
);

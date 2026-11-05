import { execFileSync } from "node:child_process";
import { mkdir, readFile, readdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
const [output, requestedTarget] = process.argv.slice(2);
if (!output || process.argv.length > 4) {
  throw new Error("Usage: node scripts/collect-licenses.mjs <output> [target]");
}

function command(program, args) {
  return execFileSync(program, args, {
    cwd: root,
    encoding: "utf8",
    maxBuffer: 32 * 1024 * 1024,
  });
}

const target = requestedTarget || process.env.CARGO_BUILD_TARGET || /^host: (.+)$/m.exec(command("rustc", ["-vV"]))?.[1];
if (!target || !/^[a-z0-9_-]+$/i.test(target)) {
  throw new Error("A valid Rust target is required.");
}

const metadata = JSON.parse(command("cargo", [
  "metadata", "--locked", "--format-version", "1",
  "--manifest-path", "src-tauri/Cargo.toml",
  "--filter-platform", target,
]));
const packages = new Map(metadata.packages.map((entry) => [entry.id, entry]));
const nodes = new Map(metadata.resolve.nodes.map((entry) => [entry.id, entry]));
const linked = new Set();

function visit(id) {
  if (linked.has(id)) return;
  linked.add(id);
  for (const dep of nodes.get(id).deps) {
    if (dep.dep_kinds.some(({ kind }) => kind === null)) visit(dep.pkg);
  }
}
visit(metadata.resolve.root);
linked.delete(metadata.resolve.root);

const documents = new Map();
const declared = new Map();
function add(name, text) {
  const content = text.replace(/\r\n/g, "\n").trim();
  if (!content) throw new Error(`Empty license text for ${name}.`);
  if (!documents.has(content)) documents.set(content, new Set());
  documents.get(content).add(name);
}

async function notices(folder, declaredFile) {
  const files = new Set();
  if (declaredFile) files.add(path.resolve(folder, declaredFile));
  for (const entry of await readdir(folder, { withFileTypes: true })) {
    if (entry.isFile() && /^(?:(?:licen[cs]es?|unlicense|copying|notice)(?:$|[._-]))/i.test(entry.name)) {
      files.add(path.join(folder, entry.name));
    }
    if (entry.isDirectory() && entry.name === "LICENSES") {
      const reuse = path.join(folder, entry.name);
      for (const file of await readdir(reuse, { withFileTypes: true })) {
        if (file.isFile()) files.add(path.join(reuse, file.name));
      }
    }
  }
  return Promise.all([...files].sort().map((file) => readFile(file, "utf8")));
}

add("ChatGPT Prism", await readFile(path.join(root, "LICENSE"), "utf8"));
for (const name of ["@tauri-apps/api", "react", "react-dom", "scheduler", "vite"]) {
  const files = await notices(path.join(root, "node_modules", name));
  if (files.length === 0) throw new Error(`No license files found for ${name}.`);
  for (const text of files) add(name, text);
}
const dependencies = [...linked].map((id) => packages.get(id)).sort((a, b) => a.name.localeCompare(b.name, "en"));
for (const dependency of dependencies) {
  const files = await notices(path.dirname(dependency.manifest_path), dependency.license_file);
  if (files.length) {
    for (const text of files) add(dependency.name, text);
  } else if (dependency.license) {
    declared.set(dependency.name, dependency.license);
  } else {
    throw new Error(`No license information found for ${dependency.name}.`);
  }
}

const sections = [...documents].map(([text, names]) => `${[...names].sort().join(", ")}\n\n${text}`);
if (declared.size) {
  sections.push(`Declared licenses\n\n${[...declared].map(([name, license]) => `${name} (${license})`).join("\n")}`);
}
const destination = path.resolve(output);
await mkdir(path.dirname(destination), { recursive: true });
await writeFile(destination, sections.join("\n\n" + "—".repeat(24) + "\n\n") + "\n");

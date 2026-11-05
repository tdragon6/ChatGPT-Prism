import { copyFile, cp, lstat, mkdir, mkdtemp, open, readFile, readdir, rename, rm, rmdir, unlink } from "node:fs/promises";
import { execFileSync, spawn } from "node:child_process";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
const [target, requestedArch, extra] = process.argv.slice(2);
const macTargets = [["arm64", "aarch64-apple-darwin"], ["x64", "x86_64-apple-darwin"]];
if (extra !== undefined || (requestedArch !== undefined
  && (target !== "mac" || !macTargets.some(([arch]) => arch === requestedArch)))) {
  throw new Error("Usage: node scripts/package.mjs mac [arm64|x64] | win");
}
const crossWindows = target === "win" && process.env.PRISM_WINDOWS_CROSS === "1";
if (!((target === "mac" && process.platform === "darwin")
  || (target === "win" && (process.platform === "win32" || crossWindows)))) {
  throw new Error("Build Mac packages on macOS and Windows packages on Windows.");
}

const output = path.resolve(process.env.PRISM_RELEASE_DIR || path.join(root, "..", "ChatGPT-Prism-Releases"));
const build = path.resolve(process.env.CARGO_TARGET_DIR || path.join(root, "src-tauri", "target"));
const env = { ...process.env, CARGO_TARGET_DIR: build };
const tauri = path.join(root, "node_modules", "@tauri-apps", "cli", "tauri.js");
const vite = path.join(root, "node_modules", "vite", "bin", "vite.js");
const cargo = process.platform === "win32" ? "cargo.exe" : "cargo";

function run(command, args, options = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, { cwd: root, env, stdio: "inherit", ...options });
    child.once("error", reject);
    child.once("exit", code => code === 0 ? resolve() : reject(new Error(`${command} exited with ${code}`)));
  });
}

function isSystemMetadata(name) {
  const lower = name.toLowerCase();
  return [".ds_store", "__macosx", "_macosx", "__maxos", "_maxos", ".spotlight-v100",
    ".trashes", ".fseventsd", ".temporaryitems", ".apdisk", "icon\r", "thumbs.db", "desktop.ini"].includes(lower)
    || lower === ".volumeicon" || lower.startsWith(".volumeicon.") || name.startsWith("._");
}

async function removeMetadataEntry(source) {
  const info = await lstat(source);
  if (info.isDirectory()) {
    for (const name of await readdir(source)) await removeMetadataEntry(path.join(source, name));
    await rmdir(source);
    return;
  }
  if (info.isFile() && info.size) {
    const file = await open(source, "r+");
    try {
      // Zero metadata blocks before recompressing the disk image.
      const zeros = Buffer.alloc(Math.min(info.size, 64 * 1024));
      for (let position = 0; position < info.size;) {
        const { bytesWritten } = await file.write(zeros, 0, Math.min(zeros.length, info.size - position), position);
        if (!bytesWritten) throw new Error("Could not clear disk image metadata.");
        position += bytesWritten;
      }
    } finally {
      await file.close();
    }
  }
  await unlink(source);
}

async function removeSystemMetadata(directory) {
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const source = path.join(directory, entry.name);
    if (isSystemMetadata(entry.name)) await removeMetadataEntry(source);
    else if (entry.isDirectory()) await removeSystemMetadata(source);
  }
}

async function cleanMacDiskImage(image, destination) {
  const hdiutil = "/usr/bin/hdiutil";
  const mount = await mkdtemp(path.join(os.tmpdir(), "prism-dmg-mount-"));
  const shadow = destination + ".shadow";
  let attached = false;
  try {
    await run(hdiutil, ["attach", image, "-shadow", shadow, "-mountpoint", mount, "-readwrite", "-nobrowse", "-noautoopen", "-quiet"]);
    attached = true;
    await run("/usr/bin/SetFile", ["-a", "c", mount]);
    await removeSystemMetadata(mount);
    await run("/usr/bin/codesign", ["--verify", "--deep", "--strict", path.join(mount, "ChatGPT Prism.app")]);
  } finally {
    if (attached) await run(hdiutil, ["detach", mount, "-quiet"]);
    await rmdir(mount);
  }
  await run(hdiutil, ["convert", image, "-shadow", shadow, "-format", "UDZO", "-imagekey", "zlib-level=9", "-o", destination, "-quiet"]);
  await rm(shadow, { force: true });
}

function releasePathFlags() {
  return [
    `--remap-path-prefix=${path.resolve(root)}=/prism`,
    `--remap-path-prefix=${path.resolve(env.CARGO_HOME || path.join(os.homedir(), ".cargo"))}=/cargo`,
    `--remap-path-prefix=${build}=/build`,
  ];
}

function encodedReleaseEnv(flags) {
  const current = env.CARGO_ENCODED_RUSTFLAGS ?? (env.RUSTFLAGS || "").trim().replace(/\s+/g, "\x1f");
  return { ...env, CARGO_ENCODED_RUSTFLAGS: [current, ...flags].filter(Boolean).join("\x1f") };
}

function macReleaseEnv() {
  const sysroot = execFileSync(env.RUSTC || "rustc", ["--print", "sysroot"], {
    cwd: root, env, encoding: "utf8",
  }).trim();
  return encodedReleaseEnv([
    ...releasePathFlags(),
    `--remap-path-prefix=${path.resolve(sysroot)}=/rust`,
  ]);
}

function windowsReleaseEnv() {
  const flags = [...releasePathFlags(), "-Clink-arg=/DEBUG:NONE"];
  if (crossWindows && env.CARGO_ENCODED_RUSTFLAGS === undefined) {
    return { ...env, RUSTFLAGS: [env.RUSTFLAGS, ...flags].filter(Boolean).join(" ") };
  }
  return encodedReleaseEnv(flags);
}

function includeStartupEntry(source) {
  return !isSystemMetadata(path.basename(source));
}

async function macStartupResources() {
  const startup = path.join(root, "startup");
  const resources = {};
  async function visit(directory) {
    const entries = (await readdir(directory, { withFileTypes: true }))
      .sort((a, b) => a.name.localeCompare(b.name, "en"));
    for (const entry of entries) {
      const source = path.join(directory, entry.name);
      if (!includeStartupEntry(source)) continue;
      if (entry.isDirectory()) {
        await visit(source);
      } else if (entry.isFile()) {
        const relative = path.relative(startup, source).split(path.sep).join("/");
        resources[source] = `startup/${relative}`;
      } else {
        throw new Error(`Unsupported startup resource: ${source}`);
      }
    }
  }
  await visit(startup);
  if (Object.keys(resources).length === 0) throw new Error("No startup animations to package.");
  return resources;
}

await mkdir(output, { recursive: true });
const temporary = await mkdtemp(path.join(os.tmpdir(), "prism-package-"));
const notices = path.join(temporary, "Licenses.txt");
const collectLicenses = triple => run(process.execPath, [path.join(root, "scripts", "collect-licenses.mjs"), notices, triple]);
try {
  if (target === "mac") {
    const resources = JSON.stringify({ bundle: { resources: {
      [notices]: "Licenses.txt", ...await macStartupResources(),
    } } });
    for (const [arch, triple] of macTargets.filter(([arch]) => !requestedArch || arch === requestedArch)) {
      await collectLicenses(triple);
      await run(process.execPath, [tauri, "build", "--target", triple, "--bundles", "dmg", "--config", resources, "--ci"], { env: macReleaseEnv() });
      const folder = path.join(build, triple, "release", "bundle", "dmg");
      const files = (await readdir(folder)).filter(name => name.startsWith("ChatGPT Prism_") && name.endsWith(".dmg"));
      if (files.length === 0) throw new Error("Mac disk image was not produced.");
      const manifest = await readFile(path.join(root, "src-tauri", "Cargo.toml"), "utf8");
      const version = /^version = "([^"]+)"/m.exec(manifest)?.[1];
      const image = files.find(name => name.startsWith(`ChatGPT Prism_${version}_`));
      if (!image) throw new Error("Mac disk image version does not match the application.");
      const name = `ChatGPT-Prism-macos-${arch}.dmg`;
      const destination = path.join(output, name);
      const finalImage = path.join(temporary, name);
      await cleanMacDiskImage(path.join(folder, image), finalImage);
      await copyFile(finalImage, destination + ".part");
      await rename(destination + ".part", destination);
    }
  } else {
    const triple = "x86_64-pc-windows-msvc";
    await run(process.execPath, [vite, "build"]);
    await collectLicenses(triple);
    await run(cargo, [...(crossWindows ? ["xwin"] : []), "build", "--locked", "--release", "--features", "custom-protocol", "--target", triple],
      { cwd: path.join(root, "src-tauri"), env: windowsReleaseEnv() });
    const bundle = path.join(temporary, "ChatGPT Prism");
    const zip = path.join(temporary, "ChatGPT-Prism-windows-x64-portable.zip");
    await mkdir(bundle);
    await copyFile(path.join(build, triple, "release", "chatgpt-prism.exe"), path.join(bundle, "ChatGPT Prism.exe"));
    await copyFile(notices, path.join(bundle, "Licenses.txt"));
    await mkdir(path.join(bundle, "data"));
    await cp(path.join(root, "startup"), path.join(bundle, "data", "startup"), {
      recursive: true,
      filter: async (source) => {
        if (!includeStartupEntry(source)) return false;
        const info = await lstat(source);
        if (!info.isDirectory() && !info.isFile()) {
          throw new Error(`Unsupported startup resource: ${source}`);
        }
        return true;
      },
    });
    if (process.platform === "win32") {
      const powershell = path.join(process.env.SystemRoot || "C:\\Windows", "System32", "WindowsPowerShell", "v1.0", "powershell.exe");
      await run(powershell, ["-NoProfile", "-NonInteractive", "-Command",
        "Add-Type -AssemblyName System.IO.Compression.FileSystem; [System.IO.Compression.ZipFile]::CreateFromDirectory($env:PRISM_BUNDLE, $env:PRISM_ARCHIVE, [System.IO.Compression.CompressionLevel]::Optimal, $true)"],
        { env: { ...env, PRISM_BUNDLE: bundle, PRISM_ARCHIVE: zip } });
    } else {
      await run("/usr/bin/zip", ["-X", "-q", "-r", zip, path.basename(bundle)], {
        cwd: temporary, env: { ...env, COPYFILE_DISABLE: "1" },
      });
    }
    const destination = path.join(output, path.basename(zip));
    await copyFile(zip, destination + ".part");
    await rename(destination + ".part", destination);
  }
} finally {
  await rm(temporary, { recursive: true, force: true });
}
console.log(output);

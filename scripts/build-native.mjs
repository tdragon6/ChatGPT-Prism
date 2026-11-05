import { execFile } from "node:child_process";
import { access, mkdir } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";

const run = promisify(execFile);
const root = fileURLToPath(new URL("../", import.meta.url));
const [target, destination, extra] = process.argv.slice(2);
const macTargets = new Set(["aarch64-apple-darwin", "x86_64-apple-darwin"]);
const windows = target === "x86_64-pc-windows-msvc";
if ((!windows && !macTargets.has(target)) || extra !== undefined) {
  throw new Error("Usage: node scripts/build-native.mjs <target-triple> [output]");
}

const name = `prism-window-host-${target}${windows ? ".exe" : ""}`;
const output = destination
  ? path.resolve(destination)
  : path.join(root, "native", "bin", name);
await mkdir(path.dirname(output), { recursive: true });

async function windowsCompiler() {
  if (process.env.PRISM_WINDOWS_CSC) return process.env.PRISM_WINDOWS_CSC;
  const system = process.env.SystemRoot || "C:\\Windows";
  for (const directory of ["Framework64", "Framework"]) {
    const compiler = path.join(system, "Microsoft.NET", directory, "v4.0.30319", "csc.exe");
    try {
      await access(compiler);
      return compiler;
    } catch (error) {
      if (error.code !== "ENOENT") throw error;
    }
  }
  throw new Error("Set PRISM_WINDOWS_CSC to a C# compiler for Windows builds.");
}

async function buildWindows() {
  const compiler = await windowsCompiler();
  const netfx = process.env.PRISM_WINDOWS_NETFX;
  const managed = path.extname(compiler).toLowerCase() === ".dll";
  if (managed && !netfx)
    throw new Error("Set PRISM_WINDOWS_NETFX to the .NET Framework reference assembly directory.");
  const references = netfx
    ? ["/nostdlib", ...["mscorlib", "System", "System.Core", "System.Web.Extensions"].map(
        name => `/reference:${path.join(netfx, name + ".dll")}`,
      )]
    : ["/reference:System.dll", "/reference:System.Core.dll", "/reference:System.Web.Extensions.dll"];
  await run(managed ? process.env.PRISM_WINDOWS_DOTNET || "dotnet" : compiler, [
    ...(managed ? [compiler] : []),
    "/nologo", "/noconfig", "/langversion:5", "/target:exe", "/platform:x64", "/optimize+",
    ...references,
    `/out:${output}`, path.join(root, "native", "WinEventHost.cs"),
  ], { windowsHide: true, timeout: 60_000 });
}

async function buildMac() {
  const arch = target.startsWith("aarch64") ? "arm64" : "x86_64";
  await run("/usr/bin/xcrun", [
    "swiftc", "-O", "-swift-version", "5", "-target", `${arch}-apple-macos13.0`,
    ...[[path.resolve(root), "/prism"], [path.dirname(output), "/build"]].flatMap(([prefix, replacement]) => [
      "-file-prefix-map", `${prefix}=${replacement}`,
      "-debug-prefix-map", `${prefix}=${replacement}`,
    ]),
    "-framework", "AppKit", "-framework", "CoreGraphics",
    path.join(root, "native", "MacWindowHost.swift"), "-o", output,
  ], { timeout: 120_000 });
}

try {
  await (windows ? buildWindows() : buildMac());
  console.log(output);
} catch (error) {
  console.error(error.stderr?.trim() || error.stdout?.trim() || error.message);
  process.exitCode = 1;
}

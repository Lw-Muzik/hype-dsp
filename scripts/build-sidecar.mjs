// Build the MilkDrop visualizer sidecar (release, `milkdrop` feature) and stage
// it where Tauri's bundler expects: `src-tauri/binaries/hm-visualizer-<triple>`.
// On Windows, projectM links as a shared lib, so its `projectM-4.dll` is staged
// alongside (and next to the dev binary) since the sidecar loads it at runtime.
//
// Run automatically by `beforeBuildCommand` (see tauri.conf.json) and usable on
// its own: `node scripts/build-sidecar.mjs`. Requires the native toolchain
// (CMake + OpenGL/GLEW; on Windows, vcpkg) — see docs/system-eq.md siblings.

import { execFileSync } from "node:child_process";
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readdirSync,
  statSync,
} from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const isWin = process.platform === "win32";
const exeName = isWin ? "hm-visualizer.exe" : "hm-visualizer";

/** Target triple: Tauri sets TAURI_ENV_TARGET_TRIPLE during a bundle; else host. */
function targetTriple() {
  if (process.env.TAURI_ENV_TARGET_TRIPLE) {
    return process.env.TAURI_ENV_TARGET_TRIPLE;
  }
  const out = execFileSync("rustc", ["-vV"], { encoding: "utf8" });
  const m = out.match(/^host:\s*(.+)$/m);
  if (!m) throw new Error("could not determine the rustc host triple");
  return m[1].trim();
}

/** First file named `name` found anywhere under `dir` (recursive), or null. */
function findFile(dir, name) {
  if (!existsSync(dir)) return null;
  for (const entry of readdirSync(dir)) {
    const p = join(dir, entry);
    const s = statSync(p);
    if (s.isDirectory()) {
      const hit = findFile(p, name);
      if (hit) return hit;
    } else if (entry === name) {
      return p;
    }
  }
  return null;
}

console.log("[sidecar] building hm-visualizer (release, milkdrop)…");
execFileSync(
  "cargo",
  ["build", "--release", "-p", "hm-visualizer", "--features", "milkdrop"],
  {
    cwd: root,
    stdio: "inherit",
    // CMake 4 dropped compat with SDL2's old `cmake_minimum_required`.
    env: { ...process.env, CMAKE_POLICY_VERSION_MINIMUM: "3.5" },
  },
);

const built = join(root, "target", "release", exeName);
if (!existsSync(built)) {
  throw new Error(`sidecar binary not found at ${built}`);
}

const binDir = join(root, "src-tauri", "binaries");
mkdirSync(binDir, { recursive: true });

const triple = targetTriple();
const dest = join(binDir, isWin ? `hm-visualizer-${triple}.exe` : `hm-visualizer-${triple}`);
copyFileSync(built, dest);
console.log(`[sidecar] staged ${dest}`);

if (isWin) {
  // projectM is a shared lib on Windows (its static feature is broken in
  // projectm-sys) — ship the DLL next to the sidecar in the bundle, and beside
  // the dev binary so `tauri dev` / manual runs resolve it too.
  const dll = findFile(join(root, "target", "release", "build"), "projectM-4.dll");
  if (dll) {
    copyFileSync(dll, join(binDir, "projectM-4.dll"));
    copyFileSync(dll, join(root, "target", "release", "projectM-4.dll"));
    console.log(`[sidecar] staged projectM-4.dll (from ${dll})`);
  } else {
    console.warn(
      "[sidecar] WARNING: projectM-4.dll not found — the visualizer won't run on Windows.",
    );
  }
}

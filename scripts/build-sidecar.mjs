// Build the MilkDrop visualizer sidecar (release, `milkdrop` feature) and stage
// it where Tauri's bundler expects: `src-tauri/binaries/hm-visualizer-<triple>`.
// On Windows, projectM links as a shared lib, so its `projectM-4.dll` is staged
// alongside (and next to the dev binary) since the sidecar loads it at runtime.
//
// Run automatically by `beforeBuildCommand` (see tauri.conf.json) and usable on
// its own: `node scripts/build-sidecar.mjs`. Requires the native toolchain
// (CMake + OpenGL/GLEW; on Windows, vcpkg) — see docs/system-eq.md siblings.
//
// Universal macOS: `tauri build --target universal-apple-darwin` runs this hook
// once with TAURI_ENV_TARGET_TRIPLE=universal-apple-darwin, but the per-arch
// bundler then needs BOTH `hm-visualizer-aarch64-apple-darwin` and
// `hm-visualizer-x86_64-apple-darwin`. So a universal target is expanded into
// its two arches: the host arch builds natively, the other is cross-built.

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

/** The rustc host triple (what a plain `cargo build` targets). */
function hostTriple() {
  const out = execFileSync("rustc", ["-vV"], { encoding: "utf8" });
  const m = out.match(/^host:\s*(.+)$/m);
  if (!m) throw new Error("could not determine the rustc host triple");
  return m[1].trim();
}

/** The arch triples to build. Tauri sets TAURI_ENV_TARGET_TRIPLE during a
 *  bundle; a universal macOS target fans out to its two concrete arches. */
function targetsToBuild() {
  const requested = process.env.TAURI_ENV_TARGET_TRIPLE || hostTriple();
  if (requested === "universal-apple-darwin") {
    return ["aarch64-apple-darwin", "x86_64-apple-darwin"];
  }
  return [requested];
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

/** Best-effort `rustup target add` so a cross-build doesn't fail on a missing
 *  std (CI installs targets up front; this covers local universal builds). */
function ensureTarget(triple) {
  try {
    execFileSync("rustup", ["target", "add", triple], { stdio: "inherit" });
  } catch {
    // rustup may be absent (e.g. distro-packaged Rust) — let the build surface
    // the real error if the target genuinely isn't available.
  }
}

/** Build hm-visualizer for `triple` and return the path to the binary. When
 *  `triple` is the host, build without `--target` (output `target/release`, no
 *  redundant per-target dir); otherwise cross-build into `target/<triple>`. */
function buildFor(triple, host) {
  const cross = triple !== host;
  console.log(
    `[sidecar] building hm-visualizer (release, milkdrop) for ${triple}` +
      (cross ? " [cross]" : "") +
      "…",
  );
  if (cross) ensureTarget(triple);

  const args = ["build", "--release", "-p", "hm-visualizer", "--features", "milkdrop"];
  if (cross) args.push("--target", triple);
  execFileSync("cargo", args, {
    cwd: root,
    stdio: "inherit",
    // CMake 4 dropped compat with SDL2's old `cmake_minimum_required`.
    env: { ...process.env, CMAKE_POLICY_VERSION_MINIMUM: "3.5" },
  });

  const outDir = cross
    ? join(root, "target", triple, "release")
    : join(root, "target", "release");
  const built = join(outDir, exeName);
  if (!existsSync(built)) {
    throw new Error(`sidecar binary not found at ${built}`);
  }
  return built;
}

const host = hostTriple();
const requested = process.env.TAURI_ENV_TARGET_TRIPLE || host;
const binDir = join(root, "src-tauri", "binaries");
mkdirSync(binDir, { recursive: true });

const staged = [];
for (const triple of targetsToBuild()) {
  const built = buildFor(triple, host);
  const dest = join(
    binDir,
    isWin ? `hm-visualizer-${triple}.exe` : `hm-visualizer-${triple}`,
  );
  copyFileSync(built, dest);
  staged.push(dest);
  console.log(`[sidecar] staged ${dest}`);
}

// A universal-apple-darwin build needs THREE files: the two per-arch sidecars
// (which satisfy Tauri's per-arch resource check during each sub-build, staged
// above) AND a single fat `hm-visualizer-universal-apple-darwin` that the final
// `.app` bundle step copies in — `lipo` the two arches into it.
if (requested === "universal-apple-darwin") {
  const universal = join(binDir, "hm-visualizer-universal-apple-darwin");
  execFileSync("lipo", ["-create", ...staged, "-output", universal], {
    stdio: "inherit",
  });
  console.log(`[sidecar] lipo'd universal ${universal}`);
}

if (isWin) {
  // projectM is a shared lib on Windows (its static feature is broken in
  // projectm-sys). The sidecar loads TWO DLLs at startup: projectM-4.dll and —
  // because projectm-sys's default `playlist` feature links it too —
  // projectM-4-playlist.dll. Both must sit next to hm-visualizer.exe or Windows
  // aborts it in the loader (STATUS_DLL_NOT_FOUND, 0xC0000135) before main()
  // runs. Stage both into the bundler's binaries dir (so tauri.windows.conf.json
  // can bundle them) and beside the dev binary so `tauri dev` / manual runs
  // resolve them too.
  const buildDir = join(root, "target", "release", "build");
  const dlls = ["projectM-4.dll", "projectM-4-playlist.dll"];
  const missing = [];
  for (const name of dlls) {
    const src = findFile(buildDir, name);
    if (!src) {
      missing.push(name);
      continue;
    }
    copyFileSync(src, join(binDir, name));
    copyFileSync(src, join(root, "target", "release", name));
    console.log(`[sidecar] staged ${name} (from ${src})`);
  }
  if (missing.length) {
    // Fail the build rather than ship a sidecar that crashes on every launch.
    throw new Error(
      `[sidecar] required projectM DLL(s) not found under ${buildDir}: ` +
        `${missing.join(", ")} — the Windows visualizer can't run without them.`,
    );
  }
}

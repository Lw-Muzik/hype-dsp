#!/usr/bin/env node
// Cut a HypeMuzik release from one command.
//
//   pnpm release 0.1.11        # explicit version
//   pnpm release patch         # 0.1.10 -> 0.1.11
//   pnpm release minor         # 0.1.10 -> 0.2.0
//   pnpm release major         # 0.1.10 -> 1.0.0
//   pnpm release patch --dry-run   # show every change, touch nothing
//
// It bumps the version in the three places that must agree (package.json,
// src-tauri/tauri.conf.json, and the Cargo workspace), refreshes Cargo.lock,
// commits, tags `v<version>`, and pushes the commit + tag. The tag push is what
// the Release workflow (.github/workflows/release.yml) waits for: it builds and
// signs macOS/Windows/Linux, publishes the GitHub Release with `latest.json`,
// and the running app auto-updates from it on its next check.
//
// The app's version is authoritative in tauri.conf.json — that is the number
// the updater compares and the bundler stamps into `latest.json` and the
// installer filenames. The other two are kept in lockstep so nothing drifts.

import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

const PKG_JSON = join(root, "package.json");
const TAURI_CONF = join(root, "src-tauri", "tauri.conf.json");
const CARGO_TOML = join(root, "Cargo.toml");

function die(msg) {
  console.error(`\n✗ ${msg}\n`);
  process.exit(1);
}

// `git` that throws its stderr on failure, trimmed stdout on success.
function git(args, { capture = true } = {}) {
  try {
    const out = execFileSync("git", args, {
      cwd: root,
      encoding: "utf8",
      stdio: capture ? ["ignore", "pipe", "pipe"] : "inherit",
    });
    return capture ? out.trim() : "";
  } catch (err) {
    const detail = err.stderr?.toString().trim() || err.message;
    die(`git ${args.join(" ")} failed:\n${detail}`);
  }
}

function parseArgs(argv) {
  const flags = new Set(argv.filter((a) => a.startsWith("--")));
  const positional = argv.filter((a) => !a.startsWith("--"));
  const unknown = [...flags].filter((f) => f !== "--dry-run");
  if (unknown.length) die(`unknown flag(s): ${unknown.join(", ")}`);
  if (positional.length !== 1) {
    die(
      "usage: pnpm release <version|patch|minor|major> [--dry-run]\n" +
        "  e.g. pnpm release 0.1.11   |   pnpm release patch",
    );
  }
  return { spec: positional[0], dryRun: flags.has("--dry-run") };
}

const SEMVER = /^(\d+)\.(\d+)\.(\d+)$/;

// A release version is a bare X.Y.Z: no pre-release/build suffix. The updater
// compares them as semver and the bundler puts the raw string in filenames, so
// anything fancier is a foot-gun we don't need.
function nextVersion(spec, current) {
  const m = current.match(SEMVER);
  if (!m) die(`current version "${current}" is not a plain X.Y.Z`);
  const [major, minor, patch] = m.slice(1).map(Number);

  if (spec === "major") return `${major + 1}.0.0`;
  if (spec === "minor") return `${major}.${minor + 1}.0`;
  if (spec === "patch") return `${major}.${minor}.${patch + 1}`;

  if (!SEMVER.test(spec)) {
    die(`"${spec}" is not a version (X.Y.Z) or a bump (patch|minor|major)`);
  }
  return spec;
}

// Strictly-greater guard so a typo can't ship a lower "release" that every
// client then refuses as older than what they already run.
function isGreater(a, b) {
  const pa = a.split(".").map(Number);
  const pb = b.split(".").map(Number);
  for (let i = 0; i < 3; i++) {
    if (pa[i] !== pb[i]) return pa[i] > pb[i];
  }
  return false;
}

// Rewrite exactly one line so the diff is one line and the file's formatting is
// left untouched. Each caller passes a regex with a single capture group around
// the version literal.
function bumpLine(file, re, version, label) {
  const src = readFileSync(file, "utf8");
  if (!re.test(src)) die(`could not find the version field in ${label}`);
  let hits = 0;
  const out = src.replace(re, (_, pre, _old, post) => {
    hits++;
    return `${pre}${version}${post}`;
  });
  if (hits !== 1) die(`expected exactly one version field in ${label}, found ${hits}`);
  return { file, out, label };
}

function currentVersion() {
  const pkg = JSON.parse(readFileSync(PKG_JSON, "utf8"));
  const tauri = JSON.parse(readFileSync(TAURI_CONF, "utf8"));
  const cargo = readFileSync(CARGO_TOML, "utf8");
  const cargoVer = cargo.match(/\[workspace\.package\][^[]*?version\s*=\s*"([^"]+)"/)?.[1];

  // If the three ever drift, refuse rather than guess which is right.
  const versions = { "package.json": pkg.version, "tauri.conf.json": tauri.version, "Cargo.toml": cargoVer };
  const distinct = [...new Set(Object.values(versions))];
  if (distinct.length !== 1) {
    die(
      "version files are out of sync — fix by hand before releasing:\n" +
        Object.entries(versions)
          .map(([k, v]) => `  ${k}: ${v ?? "(not found)"}`)
          .join("\n"),
    );
  }
  return distinct[0];
}

function main() {
  const { spec, dryRun } = parseArgs(process.argv.slice(2));

  const current = currentVersion();
  const version = nextVersion(spec, current);
  const tag = `v${version}`;

  if (version === current) die(`already at ${version}; nothing to release`);
  if (!isGreater(version, current)) {
    die(`${version} is not greater than the current ${current}`);
  }

  // A tag we'd overwrite means the release already exists (or a partial one) —
  // stop before clobbering history. Check both local and remote.
  if (git(["tag", "--list", tag])) die(`tag ${tag} already exists locally`);
  if (git(["ls-remote", "--tags", "origin", tag])) {
    die(`tag ${tag} already exists on origin`);
  }

  // A dirty tree would fold stray edits into the release commit and make the
  // tag point at a state you never reviewed. A --dry-run writes nothing, so it
  // only warns — you can preview a release from a work-in-progress tree.
  if (git(["status", "--porcelain"])) {
    if (dryRun) console.warn("⚠ working tree is not clean (fine for --dry-run; a real release needs it clean)");
    else die("working tree is not clean — commit or stash your changes first");
  }

  const branch = git(["rev-parse", "--abbrev-ref", "HEAD"]);
  if (branch !== "main") {
    console.warn(`⚠ not on main (on ${branch}); the tag still triggers a release from here.`);
  }

  const edits = [
    bumpLine(PKG_JSON, /("version"\s*:\s*")([^"]+)(")/, version, "package.json"),
    bumpLine(TAURI_CONF, /("version"\s*:\s*")([^"]+)(")/, version, "tauri.conf.json"),
    // Anchor to the [workspace.package] table so we don't touch a dependency's
    // `version = ` line: [^[]* stays inside the table (stops at the next `[`).
    bumpLine(
      CARGO_TOML,
      /(\[workspace\.package\][^[]*?version\s*=\s*")([^"]+)(")/,
      version,
      "Cargo.toml",
    ),
  ];

  console.log(`\nRelease ${current} → ${version}  (tag ${tag}, branch ${branch})`);
  for (const e of edits) console.log(`  • ${e.label}: version = ${version}`);

  if (dryRun) {
    console.log("\n--dry-run: no files written, nothing committed or pushed.");
    console.log("Steps that would run: write the 3 files → cargo update --workspace →");
    console.log(`git commit -m \"release: ${tag}\" → git tag -a ${tag} → git push origin ${branch} → git push origin refs/tags/${tag}`);
    return;
  }

  for (const e of edits) writeFileSync(e.file, e.out);
  console.log("\n✓ version files bumped");

  // Keep Cargo.lock's local-crate versions in step. `--workspace` touches only
  // our own crates, not the dependency graph. If cargo isn't installed, CI
  // regenerates the lock on build — warn and carry on rather than block.
  try {
    execFileSync("cargo", ["update", "--workspace"], { cwd: root, stdio: "inherit" });
    console.log("✓ Cargo.lock synced");
  } catch {
    console.warn("⚠ `cargo update --workspace` failed or cargo is missing — CI will regenerate Cargo.lock");
  }

  const changed = ["package.json", "src-tauri/tauri.conf.json", "Cargo.toml"];
  if (existsSync(join(root, "Cargo.lock"))) changed.push("Cargo.lock");
  git(["add", ...changed], { capture: false });
  git(["commit", "-m", `release: ${tag}`], { capture: false });
  // Annotated (`-a`), not lightweight: an annotated tag carries the tagger/date
  // and is what release tooling expects. It also matters for the push below.
  git(["tag", "-a", tag, "-m", `HypeMuzik ${tag}`], { capture: false });
  console.log(`✓ committed and tagged ${tag}`);

  // Push the branch and the tag as two explicit refs. Do NOT use
  // `--follow-tags`: it pushes ONLY annotated tags, so a lightweight tag
  // silently stayed local — the commit reached main (firing ci.yml) while the
  // tag never reached origin, so release.yml never triggered. Naming the tag
  // ref pushes it regardless of kind, and only this one tag (unlike `--tags`).
  git(["push", "origin", branch], { capture: false });
  git(["push", "origin", `refs/tags/${tag}`], { capture: false });

  const remote = git(["remote", "get-url", "origin"]);
  const slug = remote.replace(/^git@github\.com:/, "").replace(/^https:\/\/github\.com\//, "").replace(/\.git$/, "");
  console.log(`\n✓ pushed ${tag}. The Release workflow is building now:`);
  console.log(`  https://github.com/${slug}/actions`);
  console.log(`Once it publishes, running apps auto-update to ${version} on their next check.`);
}

main();

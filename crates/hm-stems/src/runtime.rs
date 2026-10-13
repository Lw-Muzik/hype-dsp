//! The ONNX Runtime shared library. `ort` is built with `load-dynamic`, so the
//! app fetches Microsoft's official 1.22 release on first use (hash-pinned),
//! pulls the one library out of the archive, and points `ort` at it. 1.22 is
//! the floor: `ort` rc.10 asks for API version 22, and an older runtime makes
//! the load fail.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use crate::download::{self, Manifest, RemoteFile};
use crate::StemError;

pub const ORT_VERSION: &str = "1.22.0";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[cfg_attr(not(windows), allow(dead_code))]
enum Archive {
    TarGz,
    Zip,
}

/// One platform's release archive.
struct Spec {
    asset: &'static str,
    archive: Archive,
    size: u64,
    sha256: &'static str,
    /// The library inside the archive, relative to its single top directory.
    member: &'static str,
    /// What we call it on disk.
    lib_name: &'static str,
}

fn spec() -> Option<Spec> {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    return Some(Spec {
        asset: "onnxruntime-osx-arm64-1.22.0.tgz",
        archive: Archive::TarGz,
        size: 25_943_843,
        sha256: "cab6dcbd77e7ec775390e7b73a8939d45fec3379b017c7cb74f5b204c1a1cc07",
        member: "lib/libonnxruntime.1.22.0.dylib",
        lib_name: "libonnxruntime.dylib",
    });
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    return Some(Spec {
        asset: "onnxruntime-osx-x86_64-1.22.0.tgz",
        archive: Archive::TarGz,
        size: 27_889_590,
        sha256: "e4ec94a7696de74fb1b12846569aa94e499958af6ffa186022cfde16c9d617f0",
        member: "lib/libonnxruntime.1.22.0.dylib",
        lib_name: "libonnxruntime.dylib",
    });
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    return Some(Spec {
        asset: "onnxruntime-linux-x64-1.22.0.tgz",
        archive: Archive::TarGz,
        size: 7_798_730,
        sha256: "8344d55f93d5bc5021ce342db50f62079daf39aaafb5d311a451846228be49b3",
        member: "lib/libonnxruntime.so.1.22.0",
        lib_name: "libonnxruntime.so",
    });
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    return Some(Spec {
        asset: "onnxruntime-win-x64-1.22.0.zip",
        archive: Archive::Zip,
        size: 72_368_545,
        sha256: "174c616efc0271194488642a72f1a514e01487da4dfe84c49296d66e40ebe0da",
        member: "lib/onnxruntime.dll",
        lib_name: "onnxruntime.dll",
    });
    #[allow(unreachable_code)]
    None
}

/// Where the runtime library lives once installed.
pub fn lib_path(root: &Path) -> Option<PathBuf> {
    spec().map(|s| root.join("runtime").join(s.lib_name))
}

/// Is a runtime available — either one the developer pointed `ORT_DYLIB_PATH`
/// at, or the one we installed?
pub fn is_ready(root: &Path) -> bool {
    std::env::var_os("ORT_DYLIB_PATH").is_some() || lib_path(root).is_some_and(|p| p.is_file())
}

/// Download size of the runtime for this platform (0 if unsupported/ready).
pub fn download_bytes(root: &Path) -> u64 {
    if is_ready(root) {
        return 0;
    }
    spec().map_or(0, |s| s.size)
}

/// Point `ort` at the installed library. A no-op when `ORT_DYLIB_PATH` is set
/// (it wins) or nothing is installed; harmless to call repeatedly.
pub fn activate(root: &Path) {
    if std::env::var_os("ORT_DYLIB_PATH").is_some() {
        return;
    }
    if let Some(p) = lib_path(root).filter(|p| p.is_file()) {
        let _ = ort::init_from(p.to_string_lossy().to_string()).commit();
    }
}

/// Fetch, verify and unpack the runtime. `progress` gets `(done, total)` bytes.
pub fn install(root: &Path, progress: &dyn Fn(u64, u64), cancel: &AtomicBool) -> Result<(), StemError> {
    let Some(spec) = spec() else {
        return Err(StemError::Unavailable);
    };
    if is_ready(root) {
        return Ok(());
    }
    let manifest = Manifest {
        dir_name: format!("runtime-dl-{ORT_VERSION}"),
        files: vec![RemoteFile {
            name: spec.asset.to_string(),
            size: spec.size,
            sha256: spec.sha256.to_string(),
            urls: vec![format!(
                "https://github.com/microsoft/onnxruntime/releases/download/v{ORT_VERSION}/{}",
                spec.asset
            )],
        }],
    };
    download::download(root, &manifest, progress, cancel)?;
    let dl_dir = download::model_dir(root, &manifest);
    let archive = dl_dir.join(spec.asset);
    let dest_dir = root.join("runtime");
    std::fs::create_dir_all(&dest_dir).map_err(io)?;
    let part = dest_dir.join(format!("{}.part", spec.lib_name));
    extract(&archive, spec.archive, spec.member, &part)?;
    std::fs::rename(&part, dest_dir.join(spec.lib_name)).map_err(io)?;
    let _ = std::fs::remove_dir_all(dl_dir);
    Ok(())
}

/// Copy the single `member` (path below the archive's top directory) to `out`.
fn extract(archive: &Path, kind: Archive, member: &str, out: &Path) -> Result<(), StemError> {
    let file = std::fs::File::open(archive).map_err(io)?;
    match kind {
        Archive::TarGz => {
            let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
            for entry in tar.entries().map_err(io)? {
                let mut entry = entry.map_err(io)?;
                let path = entry.path().map_err(io)?.to_path_buf();
                if below_top_dir(&path) == Some(member) && entry.header().entry_type().is_file() {
                    let mut dst = std::fs::File::create(out).map_err(io)?;
                    std::io::copy(&mut entry, &mut dst).map_err(io)?;
                    return Ok(());
                }
            }
        }
        Archive::Zip => {
            let mut zip = zip::ZipArchive::new(file).map_err(io)?;
            for i in 0..zip.len() {
                let mut entry = zip.by_index(i).map_err(io)?;
                let name = entry.name().to_string();
                if below_top_dir(Path::new(&name)) == Some(member) && entry.is_file() {
                    let mut dst = std::fs::File::create(out).map_err(io)?;
                    let mut buf = Vec::new();
                    entry.read_to_end(&mut buf).map_err(io)?;
                    std::io::Write::write_all(&mut dst, &buf).map_err(io)?;
                    return Ok(());
                }
            }
        }
    }
    Err(StemError::Io(format!("{member} not found in the ONNX Runtime archive")))
}

/// `onnxruntime-x/lib/foo` -> `lib/foo`.
fn below_top_dir(path: &Path) -> Option<&str> {
    let s = path.to_str()?;
    s.split_once('/').map(|(_, rest)| rest)
}

fn io(e: impl std::fmt::Display) -> StemError {
    StemError::Io(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_the_single_top_directory() {
        assert_eq!(below_top_dir(Path::new("onnxruntime-osx-arm64-1.22.0/lib/libonnxruntime.1.22.0.dylib")), Some("lib/libonnxruntime.1.22.0.dylib"));
        assert_eq!(below_top_dir(Path::new("lonely")), None);
    }

    #[test]
    fn extracts_one_member_from_a_tar_gz() {
        let dir = std::env::temp_dir().join(format!("hm_stems_rt_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let archive = dir.join("a.tgz");
        {
            let gz = flate2::write::GzEncoder::new(std::fs::File::create(&archive).unwrap(), flate2::Compression::fast());
            let mut b = tar::Builder::new(gz);
            for (name, body) in [("top/lib/other.txt", &b"no"[..]), ("top/lib/libx.1.dylib", &b"the library"[..])] {
                let mut h = tar::Header::new_gnu();
                h.set_size(body.len() as u64);
                h.set_mode(0o644);
                h.set_cksum();
                b.append_data(&mut h, name, body).unwrap();
            }
            b.into_inner().unwrap().finish().unwrap();
        }
        let out = dir.join("out");
        extract(&archive, Archive::TarGz, "lib/libx.1.dylib", &out).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), b"the library");
        assert!(extract(&archive, Archive::TarGz, "lib/missing", &dir.join("o2")).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn supported_platform_has_a_pinned_spec() {
        if let Some(s) = spec() {
            assert_eq!(s.sha256.len(), 64);
            assert!(s.asset.contains(ORT_VERSION));
        }
    }
}

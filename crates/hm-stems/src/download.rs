//! Hash-verified downloads: the Spleeter nets (and the ONNX Runtime library)
//! are fetched on first use. Mirrors the mobile model store: each file streams
//! to `<name>.part` while its SHA-256 is computed, the digest is compared with
//! the one embedded in the app, and only then is the file renamed into place —
//! so the loader can never open bytes nobody reviewed. A `ready.json` marker is
//! written last; its absence means "download again".

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::StemError;

/// One file to fetch. `urls` are tried in order (our host first, then a mirror).
#[derive(Clone, Debug)]
pub struct RemoteFile {
    pub name: String,
    pub size: u64,
    pub sha256: String,
    pub urls: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Manifest {
    /// Directory name under the models root, versioned so two model versions
    /// never share files.
    pub dir_name: String,
    pub files: Vec<RemoteFile>,
}

impl Manifest {
    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }
}

/// Deezer's Spleeter 4-stem nets (weights MIT per the authors' JOSS paper,
/// doi:10.21105/joss.02154), exported to ONNX fp16 by
/// Best-Practice/spleeter-4stems-onnx (Apache-2.0). These are the same files,
/// sizes and hashes the mobile app pins (`stems_manifest.dart`), served from
/// hypemuzik.com with the Hugging Face repo as the fallback.
pub fn spleeter_manifest() -> Manifest {
    let file = |name: &str, size: u64, sha256: &str| RemoteFile {
        name: format!("{name}.onnx"),
        size,
        sha256: sha256.to_string(),
        urls: vec![
            format!("https://hypemuzik.com/models/stems/spleeter-4stems-fp16-v1/{name}.onnx"),
            format!("https://huggingface.co/Best-Practice/spleeter-4stems-onnx/resolve/main/{name}.fp16.onnx"),
        ],
    };
    Manifest {
        dir_name: "spleeter-4stems-fp16-v1".to_string(),
        files: vec![
            file("vocals", 19_714_141, "db47148ab1c52709ce694893f532c91abfe3edc4d46238939570e036a22878ca"),
            file("drums", 19_714_140, "7ae4002e5633634674f74dc3356d5875b0da894d59ce0f60e844bb8f9cb8aa92"),
            file("bass", 19_714_139, "ba4c4949a27222492cca49859901a873b4b71461dc48c7c5a51f93d31eb11f55"),
            file("other", 19_714_140, "3cc59116cb7195946ab9596d8ca25984d09c0f8a70db8cf85d063132f97bc61d"),
        ],
    }
}

const MARKER: &str = "ready.json";

/// Whether every file of `manifest` is installed under `root/<dir_name>`: the
/// marker is present and each file has its exact size (hashes were checked when
/// the marker was written).
pub fn is_ready(root: &Path, manifest: &Manifest) -> bool {
    let dir = root.join(&manifest.dir_name);
    let Ok(text) = std::fs::read_to_string(dir.join(MARKER)) else { return false };
    let Ok(marker) = serde_json::from_str::<serde_json::Value>(&text) else { return false };
    manifest.files.iter().all(|f| {
        marker["files"][&f.name] == f.sha256.as_str()
            && std::fs::metadata(dir.join(&f.name)).is_ok_and(|m| m.len() == f.size)
    })
}

/// Where the installed files live.
pub fn model_dir(root: &Path, manifest: &Manifest) -> PathBuf {
    root.join(&manifest.dir_name)
}

/// Download and verify every missing file. `progress` gets `(bytes_done,
/// bytes_total)` across the whole set. Files that already verify are kept.
pub fn download(
    root: &Path,
    manifest: &Manifest,
    progress: &dyn Fn(u64, u64),
    cancel: &AtomicBool,
) -> Result<(), StemError> {
    let dir = model_dir(root, manifest);
    std::fs::create_dir_all(&dir).map_err(io)?;
    let total = manifest.total_bytes();
    let mut done = 0u64;
    // A marker from an older/partial install must not outlive a re-download.
    let _ = std::fs::remove_file(dir.join(MARKER));
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(900))
        .build()
        .map_err(|e| StemError::Network(e.to_string()))?;
    for f in &manifest.files {
        let target = dir.join(&f.name);
        if verified_on_disk(&target, f) {
            done += f.size;
            progress(done, total);
            continue;
        }
        let mut last_err = StemError::Network("no download URL".into());
        let mut ok = false;
        for url in &f.urls {
            match fetch_verified(&client, url, &target, f, &mut |n| progress(done + n, total), cancel) {
                Ok(()) => {
                    ok = true;
                    break;
                }
                Err(StemError::Aborted) => return Err(StemError::Aborted),
                Err(e) => last_err = e,
            }
        }
        if !ok {
            return Err(last_err);
        }
        done += f.size;
        progress(done, total);
    }
    let files: serde_json::Map<String, serde_json::Value> = manifest
        .files
        .iter()
        .map(|f| (f.name.clone(), serde_json::Value::String(f.sha256.clone())))
        .collect();
    std::fs::write(dir.join(MARKER), serde_json::json!({ "files": files }).to_string()).map_err(io)
}

fn verified_on_disk(path: &Path, f: &RemoteFile) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.len() == f.size) && hash_file(path).is_ok_and(|h| h == f.sha256)
}

pub(crate) fn hash_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub(crate) fn fetch_verified(
    client: &reqwest::blocking::Client,
    url: &str,
    target: &Path,
    f: &RemoteFile,
    on_bytes: &mut dyn FnMut(u64),
    cancel: &AtomicBool,
) -> Result<(), StemError> {
    let part = PathBuf::from(format!("{}.part", target.display()));
    let result = (|| {
        let mut resp = client
            .get(url)
            .send()
            .and_then(|r| r.error_for_status())
            .map_err(|e| StemError::Network(e.to_string()))?;
        let mut out = std::fs::File::create(&part).map_err(io)?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 1 << 16];
        let mut got = 0u64;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(StemError::Aborted);
            }
            let n = resp.read(&mut buf).map_err(|e| StemError::Network(e.to_string()))?;
            if n == 0 {
                break;
            }
            got += n as u64;
            if got > f.size {
                return Err(StemError::Network(format!("{url} sent more than the expected {} bytes", f.size)));
            }
            hasher.update(&buf[..n]);
            out.write_all(&buf[..n]).map_err(io)?;
            on_bytes(got);
        }
        out.flush().map_err(io)?;
        drop(out);
        let digest = hex(&hasher.finalize());
        if got != f.size || digest != f.sha256 {
            return Err(StemError::Network(format!(
                "{} failed its integrity check (got {got} bytes, sha256 {digest})",
                f.name
            )));
        }
        std::fs::rename(&part, target).map_err(io)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    result
}

fn io(e: impl std::fmt::Display) -> StemError {
    StemError::Io(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;
    use std::sync::Arc;

    /// A one-file HTTP server answering every request with `body`.
    fn serve(body: Vec<u8>) -> (String, Arc<std::sync::atomic::AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let h = hits.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                h.fetch_add(1, Ordering::Relaxed);
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                while reader.read_line(&mut line).is_ok() && line.trim() != "" {
                    line.clear();
                }
                let mut s = stream;
                let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = s.write_all(head.as_bytes());
                let _ = s.write_all(&body);
            }
        });
        (format!("http://{addr}/f.onnx"), hits)
    }

    fn manifest_for(body: &[u8], urls: Vec<String>, sha: Option<String>) -> Manifest {
        Manifest {
            dir_name: "m-v1".into(),
            files: vec![RemoteFile {
                name: "a.onnx".into(),
                size: body.len() as u64,
                sha256: sha.unwrap_or_else(|| hex(&Sha256::digest(body))),
                urls,
            }],
        }
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("hm_stems_dl_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn shipped_manifest_matches_the_site_hash_list() {
        let m = spleeter_manifest();
        assert_eq!(m.files.len(), 4);
        assert_eq!(m.total_bytes(), 78_856_560, "the mobile manifest's totalBytes");
        assert!(m.files.iter().all(|f| f.sha256.len() == 64 && f.urls.len() == 2));
    }

    #[test]
    fn downloads_verifies_and_marks_ready() {
        let body: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        let (url, _) = serve(body.clone());
        let root = tmp("ok");
        let m = manifest_for(&body, vec![url], None);
        assert!(!is_ready(&root, &m));
        let last = std::sync::atomic::AtomicU64::new(0);
        download(&root, &m, &|d, _| last.store(d, Ordering::Relaxed), &AtomicBool::new(false)).unwrap();
        assert!(is_ready(&root, &m));
        assert_eq!(last.load(Ordering::Relaxed), body.len() as u64);
        assert_eq!(std::fs::read(model_dir(&root, &m).join("a.onnx")).unwrap(), body);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_corrupt_bytes_and_leaves_nothing_behind() {
        let body = vec![7u8; 50_000];
        let (url, _) = serve(body.clone());
        let root = tmp("bad");
        let m = manifest_for(&body, vec![url], Some("0".repeat(64)));
        let err = download(&root, &m, &|_, _| {}, &AtomicBool::new(false)).unwrap_err();
        assert!(err.to_string().contains("integrity"), "{err}");
        let dir = model_dir(&root, &m);
        assert!(!dir.join("a.onnx").exists() && !dir.join("a.onnx.part").exists());
        assert!(!is_ready(&root, &m));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn falls_back_to_the_next_url() {
        let body = vec![9u8; 10_000];
        let (good, _) = serve(body.clone());
        let root = tmp("fallback");
        let m = manifest_for(&body, vec!["http://127.0.0.1:1/none".into(), good], None);
        download(&root, &m, &|_, _| {}, &AtomicBool::new(false)).unwrap();
        assert!(is_ready(&root, &m));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_verified_file_is_not_fetched_again() {
        let body = vec![3u8; 10_000];
        let (url, hits) = serve(body.clone());
        let root = tmp("skip");
        let m = manifest_for(&body, vec![url], None);
        download(&root, &m, &|_, _| {}, &AtomicBool::new(false)).unwrap();
        std::fs::remove_file(model_dir(&root, &m).join(MARKER)).unwrap();
        let before = hits.load(Ordering::Relaxed);
        download(&root, &m, &|_, _| {}, &AtomicBool::new(false)).unwrap();
        assert_eq!(hits.load(Ordering::Relaxed), before);
        assert!(is_ready(&root, &m));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn cancel_aborts_and_removes_the_partial_file() {
        let body = vec![1u8; 1_000_000];
        let (url, _) = serve(body.clone());
        let root = tmp("cancel");
        let m = manifest_for(&body, vec![url], None);
        let err = download(&root, &m, &|_, _| {}, &AtomicBool::new(true)).unwrap_err();
        assert!(matches!(err, StemError::Aborted));
        assert!(!model_dir(&root, &m).join("a.onnx.part").exists());
        let _ = std::fs::remove_dir_all(root);
    }
}

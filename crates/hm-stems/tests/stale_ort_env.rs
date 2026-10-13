//! An `ORT_DYLIB_PATH` that names a missing file must not count as a runtime.
//!
//! The Demucs-era app set it on itself at startup; the updater's restart hands
//! that environment to the new version, whose legacy clean-up moves the file
//! into `runtime/`. Trusting the stale value made every stems load fail with
//! "the ONNX Runtime library could not be loaded". Its own test binary: it
//! sets the process environment.

use hm_stems::runtime;

#[test]
fn a_stale_ort_dylib_path_does_not_count_as_a_runtime() {
    let root = std::env::temp_dir().join(format!("hm_stems_stale_env_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::env::set_var("ORT_DYLIB_PATH", root.join("libonnxruntime.dylib"));

    if runtime::lib_path(&root).is_some() {
        // Nothing installed: the app must offer the download, not try to load.
        assert!(!runtime::is_ready(&root));
        assert!(runtime::download_bytes(&root) > 0);

        // Installed under runtime/: that one is used.
        let lib = runtime::lib_path(&root).unwrap();
        std::fs::create_dir_all(lib.parent().unwrap()).unwrap();
        std::fs::write(&lib, b"lib").unwrap();
        assert!(runtime::is_ready(&root));
        assert_eq!(runtime::download_bytes(&root), 0);
    }

    // A developer override that exists still wins.
    let dev = root.join("dev-libonnxruntime");
    std::fs::write(&dev, b"lib").unwrap();
    std::env::set_var("ORT_DYLIB_PATH", &dev);
    let empty = root.join("empty-root");
    assert!(runtime::is_ready(&empty));

    let _ = std::fs::remove_dir_all(root);
}

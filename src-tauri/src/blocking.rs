//! Blocking work inside commands.
//!
//! `#[tauri::command(async)]` on a plain `fn` does not run it on a blocking
//! thread: Tauri wraps the body in `async move { … }` on the async runtime. So
//! every such command that does blocking I/O (`reqwest::blocking`, decoding a
//! file, enumerating devices, waiting on an installer) was parking a Tokio
//! worker, which can stall the async tasks sharing it: the Hype Connect socket,
//! the updater, event delivery.
//!
//! In debug builds it was worse: `reqwest::blocking` asserts it is not inside
//! an async context by creating and dropping a runtime there, which panics
//! ("Cannot drop a runtime in a context where blocking is not allowed"). The
//! command's future died, its `invoke()` never settled, and `pnpm tauri dev`
//! hung on the splash waiting for `account_status`.
//!
//! [`io`] tells Tokio the current worker is about to block, so its queued tasks
//! move to another worker first. Outside the runtime (a std thread, a
//! `spawn_blocking` thread) it just runs the closure. Every async command body
//! goes through it; `every_async_command_runs_its_body_in_blocking_io` below
//! keeps it that way.

/// Run blocking work from a command. Same thread, same result; Tokio is told.
pub fn io<T>(f: impl FnOnce() -> T) -> T {
    tokio::task::block_in_place(f)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The failure this fixes: a `reqwest::blocking` call made directly on an
    /// async worker panics in debug builds (and blocks the worker in release).
    /// Through `io` it runs. Port 9 (discard) on loopback refuses at once, so
    /// no network is needed: the point is that it returns an error rather than
    /// panicking.
    #[test]
    fn blocking_http_from_an_async_worker_does_not_panic() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let result = rt.block_on(async {
            tokio::spawn(async {
                io(|| {
                    reqwest::blocking::Client::new()
                        .get("http://127.0.0.1:9/")
                        .timeout(std::time::Duration::from_secs(2))
                        .send()
                        .map(|_| ())
                        .map_err(|e| e.to_string())
                })
            })
            .await
        });
        // The task finished (no panic); the request itself failed to connect.
        let inner = result.expect("the command task must not panic");
        assert!(inner.is_err());
    }

    #[test]
    fn outside_a_runtime_it_just_runs() {
        assert_eq!(io(|| 41 + 1), 42);
    }

    /// Every `#[tauri::command(async)]` sync fn must run its body in `io`.
    #[test]
    fn every_async_command_runs_its_body_in_blocking_io() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut offenders = Vec::new();
        let mut checked = 0;
        visit(&root, &mut |path, text| {
            let lines: Vec<&str> = text.lines().collect();
            for (i, line) in lines.iter().enumerate() {
                if line.trim() != "#[tauri::command(async)]" {
                    continue;
                }
                // Find the signature line, skipping other attributes.
                let Some(sig) = lines[i + 1..].iter().position(|l| l.trim_start().starts_with("pub ")) else {
                    continue;
                };
                let sig_line = lines[i + 1 + sig];
                if sig_line.contains("async fn") {
                    continue; // genuinely async: awaits, doesn't block
                }
                checked += 1;
                // The body's first statement follows the line ending the signature with `{`.
                let open = lines[i + 1 + sig..].iter().position(|l| l.trim_end().ends_with('{')).unwrap();
                let first = lines[i + 2 + sig + open].trim();
                if !first.starts_with("crate::blocking::io(") {
                    offenders.push(format!("{}: {}", path.display(), sig_line.trim()));
                }
            }
        });
        assert!(checked > 50, "scanned too few commands ({checked}); is the source layout different?");
        assert!(offenders.is_empty(), "commands blocking an async worker:\n{}", offenders.join("\n"));
    }

    fn visit(dir: &std::path::Path, f: &mut dyn FnMut(&std::path::Path, &str)) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                visit(&path, f);
            } else if path.extension().is_some_and(|e| e == "rs") {
                f(&path, &std::fs::read_to_string(&path).unwrap());
            }
        }
    }
}

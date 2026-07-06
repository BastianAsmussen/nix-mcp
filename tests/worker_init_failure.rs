//! Readiness-gate failure test, isolated in its own binary on purpose.
//!
//! Nix init is process-global and one-shot: a failed init poisons every later
//! Nix use in the same process, and `#[serial]` cannot help across that
//! boundary. File-level isolation does (each integration-test file is its own
//! process), so no other Nix-touching test may be added to this file.
//!
//! There is a second trap here: `NIX_REMOTE` cannot be injected with
//! `std::env::set_var` at runtime. libnixstore's global `Settings` object is
//! a C++ static that captures `getenv("NIX_REMOTE")` when the library is
//! loaded (before `main`), so a runtime `set_var` is invisible to store
//! resolution (verified empirically: with an in-process `set_var`, the worker
//! happily opened the real local store). The variable must be in the
//! process's environment from the start, so this test re-execs its own
//! binary: the parent spawns `current_exe()` filtered to the `child_*` test
//! with `NIX_REMOTE` pointing at a nonexistent daemon socket, and the child
//! is what actually calls `serve()`.

use std::process::{Command, Stdio};

/// Marker distinguishing the re-exec'd child from a normal suite run.
const CHILD_MARKER: &str = "NIX_MCP_INIT_FAILURE_CHILD";

#[test]
fn serve_refuses_to_start_when_the_store_is_unreachable() {
    let exe = std::env::current_exe().expect("test binary path");
    let output = Command::new(exe)
        .args([
            "child_serve_fails_with_unreachable_store",
            "--exact",
            "--nocapture",
        ])
        .env(CHILD_MARKER, "1")
        .env("NIX_REMOTE", "unix:///nix-mcp-tests/no-such-daemon.socket")
        // Null stdin: if the readiness gate ever regresses and `serve()`
        // reaches the MCP handshake, the stdio read hits EOF and errors
        // instead of hanging the suite on the harness's stdin.
        .stdin(Stdio::null())
        .output()
        .expect("spawn child test process");

    assert!(
        output.status.success(),
        "child test failed (status: {:?})\n--- child stdout ---\n{}\n--- child stderr ---\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

/// Body of the test above, run in a child process whose `NIX_REMOTE` was set
/// at spawn time. A no-op unless [`CHILD_MARKER`] is present, so a normal
/// suite run passes it without touching Nix (see the module docs).
#[tokio::test(flavor = "current_thread")]
async fn child_serve_fails_with_unreachable_store() {
    if std::env::var_os(CHILD_MARKER).is_none() {
        return;
    }

    let err = nix_mcp::serve()
        .await
        .expect_err("a server without a reachable store must refuse to start");
    let msg = err.to_string();
    assert!(msg.contains("failed to initialize"), "got: {msg}");
    assert!(msg.contains("refusing to start"), "got: {msg}");
}

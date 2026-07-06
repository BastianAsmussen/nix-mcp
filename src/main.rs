//! Binary entrypoint for the `nix_mcp` MCP server.
//!
//! All logic lives in the [`nix_mcp`] library crate; this just initializes
//! tracing and serves the protocol over stdio.

// See the twin expect in `lib.rs`: transitive skew from the reqwest tree.
#![expect(
    clippy::multiple_crate_versions,
    reason = "unfixable transitive skew (getrandom majors, r-efi, windows-sys)"
)]

#[tokio::main(flavor = "current_thread")]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    nix_mcp::init_tracing();
    nix_mcp::serve().await
}

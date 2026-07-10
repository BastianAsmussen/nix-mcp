//! The `nix_mcp` library: the MCP tool router, worker plumbing, and the
//! [`serve`] entrypoint the binary drives.
//!
//! Each `#[tool]` method turns its arguments into a [`worker::NixOp`] and hands
//! it to the single Nix worker thread via `NixMcp::send_op`, then returns the
//! worker's JSON reply. All Nix work happens on the worker; the one exception
//! is `nix_search`'s web backend ([`search::WebSearch`]), plain HTTP that the
//! async handler serves itself without touching Nix state. Splitting this out
//! as a library lets the benches and integration tests exercise the internals
//! directly.

// The `Arc<Context>` pattern is documented by nix-bindings: the underlying
// types are `Send` but `!Sync` (see the crate's thread-safety docs).
#![expect(
    clippy::arc_with_non_send_sync,
    reason = "documented Arc<Context> pattern; nix-bindings types are Send but !Sync"
)]
// Transitive version skew from the reqwest tree: getrandom 0.2/0.3/0.4 are
// deliberately coexisting majors (proptest's rand vs rustls), and r-efi +
// windows-sys are UEFI-/Windows-only deps that follow them. Not resolvable
// from this crate; the expect flags itself for removal if the tree dedupes.
#![expect(
    clippy::multiple_crate_versions,
    reason = "unfixable transitive skew (getrandom majors, r-efi, windows-sys)"
)]

pub mod config;
pub mod error;
pub mod output;
pub mod search;
pub mod tools;
pub mod worker;

use std::sync::Arc;

use rmcp::{
    ServerHandler,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock},
    tool, tool_handler, tool_router,
};
use tracing_subscriber::EnvFilter;

use crate::error::NixMcpError;
use crate::worker::{NixOp, NixWorker};

/// Capacity of the op channel into the Nix worker. Bounded so a pipelining
/// client queueing behind a long `realize` hits backpressure (`send_async`
/// waits) instead of growing an unbounded queue.
const OP_CHANNEL_CAPACITY: usize = 32;

/// Default page size for the `closure` query when the caller omits `limit`.
const DEFAULT_CLOSURE_LIMIT: u32 = 100;

/// Server-side cap on the `closure` page size, so a caller cannot request an
/// unbounded result that overruns the client's token budget.
const MAX_CLOSURE_LIMIT: u32 = 500;

/// Default cap, in characters, on a rendered `nix_eval` result.
const DEFAULT_EVAL_MAX_LEN: u32 = 10_000;

/// Server-side ceiling on `nix_eval`'s `max_len`, so a single eval cannot
/// flood the client's context no matter what the caller asks for.
const MAX_EVAL_MAX_LEN: u32 = 100_000;

/// Default page size for a `nix_search` result page.
const DEFAULT_SEARCH_LIMIT: u32 = 20;

/// Server-side cap on the `nix_search` page size.
const MAX_SEARCH_LIMIT: u32 = 100;

/// The MCP server handle: a clonable sender into the Nix worker thread.
#[derive(Clone)]
struct NixMcp {
    /// Channel to the worker that owns all Nix state.
    worker_tx: flume::Sender<NixOp>,
    /// The `nix_search` web backend: HTTP only, no Nix state, so it runs on
    /// the async handler instead of queueing behind the worker.
    web: Arc<search::WebSearch>,
}

impl NixMcp {
    /// Send an op to the worker, await its reply, and classify any failure.
    ///
    /// `op` is handed a fresh reply channel and returns the [`NixOp`] to send.
    ///
    /// A successful reply becomes a structured result: the worker's JSON
    /// (built from the [`crate::output`] structs that also generate each
    /// tool's `outputSchema`) rides in `structuredContent`, with the
    /// serialized text block alongside for clients that predate structured
    /// output. Failures the agent can act on (Nix evaluation/store errors,
    /// invalid arguments) come back in-band as a tool result with
    /// `is_error: true`, so the client feeds the diagnostic to the model for
    /// self-correction. Per MCP, protocol-level errors may never reach the
    /// model at all, so they are reserved for failures the server cannot
    /// recover from.
    ///
    /// # Errors
    ///
    /// Returns an [`rmcp::ErrorData`] only for a worker-channel failure (the
    /// worker thread is gone or dropped the reply without answering) or a
    /// reply that is not valid JSON, which the output structs make
    /// impossible short of a server bug.
    async fn send_op(
        &self,
        op: impl FnOnce(flume::Sender<crate::error::Result<String>>) -> NixOp,
    ) -> std::result::Result<CallToolResult, rmcp::ErrorData> {
        let (tx, rx) = flume::bounded(1);
        self.worker_tx
            .send_async(op(tx))
            .await
            .map_err(|err| channel_error(format!("worker disconnected: {err}")))?;

        let reply = rx
            .into_recv_async()
            .await
            .map_err(|err| channel_error(format!("worker dropped the reply: {err}")))?;

        Self::reply_to_result(reply)
    }

    /// Wrap a backend reply as a tool result: the JSON rides in
    /// `structuredContent` (with the serialized text block alongside), and
    /// failures the agent can act on come back in-band with
    /// `is_error: true`. Shared by the worker path (`send_op`) and the
    /// `nix_search` web path.
    ///
    /// # Errors
    ///
    /// Returns an [`rmcp::ErrorData`] only for a reply that is not valid
    /// JSON, which the output structs make impossible short of a server bug.
    fn reply_to_result(
        reply: crate::error::Result<String>,
    ) -> std::result::Result<CallToolResult, rmcp::ErrorData> {
        match reply {
            Ok(json) => {
                let value: serde_json::Value = serde_json::from_str(&json).map_err(|err| {
                    rmcp::ErrorData::internal_error(
                        format!("the backend produced an unparseable reply: {err}"),
                        None,
                    )
                })?;
                Ok(CallToolResult::structured(value))
            }
            Err(err) => Ok(CallToolResult::error(vec![ContentBlock::text(
                err.to_string(),
            )])),
        }
    }
}

/// Map a worker-channel failure to the JSON-RPC internal error it is: the
/// server can no longer do any work, which no agent can retry around in-band.
fn channel_error(message: String) -> rmcp::ErrorData {
    rmcp::ErrorData::internal_error(NixMcpError::Channel(message).to_string(), None)
}

/// Resolve a requested `closure` page size to its effective value: the default
/// when omitted, clamped to `[1, MAX_CLOSURE_LIMIT]`. The floor of 1 stops a
/// paging caller from looping forever on a zero-size page.
fn effective_limit(requested: Option<u32>) -> usize {
    requested
        .unwrap_or(DEFAULT_CLOSURE_LIMIT)
        .clamp(1, MAX_CLOSURE_LIMIT) as usize
}

/// Resolve a requested eval `max_len` to its effective value: the default
/// when omitted, clamped to `[1, MAX_EVAL_MAX_LEN]`.
fn effective_eval_max_len(requested: Option<u32>) -> usize {
    requested
        .unwrap_or(DEFAULT_EVAL_MAX_LEN)
        .clamp(1, MAX_EVAL_MAX_LEN) as usize
}

/// Resolve a requested `nix_search` page size to its effective value: the
/// default when omitted, clamped to `[1, MAX_SEARCH_LIMIT]`.
fn effective_search_limit(requested: Option<u32>) -> usize {
    requested
        .unwrap_or(DEFAULT_SEARCH_LIMIT)
        .clamp(1, MAX_SEARCH_LIMIT) as usize
}

#[tool_router]
impl NixMcp {
    /// Report information about the Nix store.
    ///
    /// # Errors
    ///
    /// Returns an error only if the worker is unavailable; query failures are
    /// reported in-band in the tool result.
    #[tool(
        description = "Get information about the Nix store: URI, store directory, daemon and library versions, and the evaluator's current system (e.g. x86_64-linux, useful for picking flake attrs). Returns JSON.",
        output_schema = output::schema::<output::StoreInfo>(),
        annotations(
            title = "Nix store info",
            read_only_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn nix_store_info(&self) -> std::result::Result<CallToolResult, rmcp::ErrorData> {
        self.send_op(NixOp::StoreInfo).await
    }

    /// Evaluate a Nix expression.
    ///
    /// # Errors
    ///
    /// Returns an error only if the worker is unavailable; evaluation failures
    /// are reported in-band in the tool result.
    #[tool(
        description = "Evaluate a Nix expression in an impure, flake-enabled evaluator (builtins.getFlake and fetch builtins work; they may access the network and write to the store). Returns JSON {\"result\":\"...\"} where result is the value rendered as Nix syntax, not JSON (strings include their quotes). `attr` selects one top-level attribute of the result. Results longer than max_len characters (default 10000) are cut and carry {truncated, total_len, hint}. Example expression: builtins.nixVersion.",
        output_schema = output::schema::<output::EvalResult>(),
        annotations(
            title = "Nix eval",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = true
        )
    )]
    async fn nix_eval(
        &self,
        Parameters(tools::EvalParams {
            expression,
            attr,
            max_len,
        }): Parameters<tools::EvalParams>,
    ) -> std::result::Result<CallToolResult, rmcp::ErrorData> {
        let max_len = effective_eval_max_len(max_len);

        self.send_op(|reply| NixOp::Eval {
            expression,
            attr,
            max_len,
            reply,
        })
        .await
    }

    /// Query a store path.
    ///
    /// # Errors
    ///
    /// Returns an error only if the worker is unavailable; invalid arguments
    /// and store failures are reported in-band in the tool result.
    #[tool(
        description = "Query a store path. operation = validate | real_path | closure | resolve_hash; 'path' is required for real_path and closure, 'hash' for resolve_hash, either one for validate. For closure: closure_type = runtime (default) | build | referrers, paged via offset/limit; the result carries {count, paths:[\"/nix/store/...\"], has_more, next_offset} ({path,name} objects when include_names=true). To check whether a closure contains something, pass filter (case-insensitive substring on the name) instead of paging everything. Returns JSON.",
        output_schema = output::schema::<output::StorePathQueryResult>(),
        annotations(
            title = "Nix store path query",
            read_only_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn nix_store_path_query(
        &self,
        Parameters(p): Parameters<tools::StorePathQueryParams>,
    ) -> std::result::Result<CallToolResult, rmcp::ErrorData> {
        let closure_type = p.closure_type.unwrap_or_default();
        let offset = p.offset.unwrap_or(0) as usize;
        let limit = effective_limit(p.limit);
        let include_names = p.include_names.unwrap_or(false);

        self.send_op(|reply| NixOp::StorePathQuery {
            path: p.path,
            hash: p.hash,
            operation: p.operation,
            closure_type,
            offset,
            limit,
            filter: p.filter,
            include_names,
            reply,
        })
        .await
    }

    /// Realize (build or substitute) a store path and its dependencies.
    ///
    /// # Errors
    ///
    /// Returns an error only if the worker is unavailable; build failures are
    /// reported in-band in the tool result.
    #[tool(
        description = "Build (or substitute) the outputs of a derivation. path must be a .drv store path; the Nix C API cannot fetch plain output paths, so check those with nix_store_path_query operation=validate instead. Returns JSON {\"outputs\":[{output_name, path, name}], \"count\"}. May take minutes for large builds, blocks every other nix_mcp tool until it finishes, and cannot be cancelled; do not call it speculatively.",
        output_schema = output::schema::<output::RealizeResult>(),
        annotations(
            title = "Nix realize",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = true
        )
    )]
    async fn nix_realize(
        &self,
        Parameters(p): Parameters<tools::RealizeParams>,
    ) -> std::result::Result<CallToolResult, rmcp::ErrorData> {
        self.send_op(|reply| NixOp::Realize {
            path: p.path,
            reply,
        })
        .await
    }

    /// Resolve and lock a flake reference.
    ///
    /// # Errors
    ///
    /// Returns an error only if the worker is unavailable; resolution failures
    /// are reported in-band in the tool result.
    #[tool(
        description = "Resolve and lock a flake reference, returning its top-level output attribute names as JSON. Fetches the flake into the store (remote refs access the network). Pass a bare ref without a #fragment; fragments are rejected until attribute selection is supported. Example url: github:NixOS/nixpkgs.",
        output_schema = output::schema::<output::FlakeResolveResult>(),
        annotations(
            title = "Nix flake resolve",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn nix_flake_resolve(
        &self,
        Parameters(p): Parameters<tools::FlakeResolveParams>,
    ) -> std::result::Result<CallToolResult, rmcp::ErrorData> {
        self.send_op(|reply| NixOp::FlakeResolve { url: p.url, reply })
            .await
    }

    /// Read derivation metadata from a `.drv` store path.
    ///
    /// # Errors
    ///
    /// Returns an error only if the worker is unavailable; read failures are
    /// reported in-band in the tool result.
    #[tool(
        description = "Read derivation metadata from a .drv store path (the path must end in .drv). Returns the derivation as JSON; by default a summary where values over 500 chars (build scripts and the like) are truncated inline, with {truncated, hint} set when anything was cut. Pass verbose=true for the complete JSON, which can be tens of thousands of tokens. Example path: /nix/store/<hash>-hello.drv. Find .drv paths via nix_store_path_query with operation=closure, closure_type=build.",
        output_schema = output::schema::<output::DerivationJson>(),
        annotations(
            title = "Nix derivation info",
            read_only_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn nix_derivation_info(
        &self,
        Parameters(p): Parameters<tools::DerivationInfoParams>,
    ) -> std::result::Result<CallToolResult, rmcp::ErrorData> {
        let verbose = p.verbose.unwrap_or(false);

        self.send_op(|reply| NixOp::DerivationInfo {
            path: p.path,
            verbose,
            reply,
        })
        .await
    }

    /// Search nixpkgs packages and NixOS options.
    ///
    /// # Errors
    ///
    /// Returns an error only if the worker is unavailable; web and
    /// evaluation failures are reported in-band in the tool result.
    #[tool(
        description = "Search nixpkgs packages and NixOS options; use it even when you think you already know the name (nixpkgs moves faster than any training data). operation = package | option. source = web (default: search.nixos.org, ranked, needs network; channel = unstable (default) or a release like 25.11) | pinned (offline: evaluates a nixpkgs flake in-process; flake = nixpkgs (default, via the flake registry); matches names only; option search evaluates the whole NixOS module system and can take tens of seconds, and submodule options like <name>.* are not enumerated). Paged via offset/limit; returns JSON {count, results:[{name, version?, description?, type?, default?, example?, programs?}], has_more, next_offset}.",
        output_schema = output::schema::<output::SearchResult>(),
        annotations(
            title = "Nix search",
            read_only_hint = true,
            idempotent_hint = true,
            open_world_hint = true
        )
    )]
    async fn nix_search(
        &self,
        Parameters(p): Parameters<tools::SearchParams>,
    ) -> std::result::Result<CallToolResult, rmcp::ErrorData> {
        let source = p.source.unwrap_or_default();
        let offset = p.offset.unwrap_or(0) as usize;
        let limit = effective_search_limit(p.limit);

        match source {
            tools::SearchSource::Web => {
                // Fail loud on a pinned-only argument instead of silently
                // answering from a different dataset than the caller named.
                if p.flake.is_some() {
                    return Self::reply_to_result(Err(NixMcpError::InvalidArgument(
                        "'flake' applies to source=pinned; for web searches pick a 'channel' \
                         (unstable, or a release like 25.11)"
                            .to_owned(),
                    )));
                }
                let channel = p.channel.unwrap_or_else(|| "unstable".to_owned());
                let reply = self
                    .web
                    .search(p.operation, &p.query, &channel, offset, limit)
                    .await
                    .and_then(|result| Ok(serde_json::to_string(&result)?));
                Self::reply_to_result(reply)
            }
            tools::SearchSource::Pinned => {
                if p.channel.is_some() {
                    return Self::reply_to_result(Err(NixMcpError::InvalidArgument(
                        "'channel' applies to source=web; for pinned searches pass 'flake' \
                         (default nixpkgs, via the flake registry)"
                            .to_owned(),
                    )));
                }
                let flake = p.flake.unwrap_or_else(|| "nixpkgs".to_owned());
                self.send_op(|reply| NixOp::Search {
                    operation: p.operation,
                    query: p.query,
                    flake,
                    offset,
                    limit,
                    reply,
                })
                .await
            }
        }
    }

    /// Query Nix configuration settings (nix.conf).
    ///
    /// # Errors
    ///
    /// Returns an error only if the worker is unavailable; invalid arguments
    /// and setting-read failures are reported in-band in the tool result.
    #[tool(
        description = "Query nix.conf settings (connect-timeout, stalled-download-timeout, substituters, etc.): the keys in nix.conf / nix.settings that are NOT NixOS module options and do not appear in nix_search. operation = get (read one setting by key) | show (list settings with descriptions and current values, optionally filtered by a substring query) | list (list setting keys only, optionally filtered). For get: query is the setting key (e.g. connect-timeout). For show/list: query is an optional case-insensitive substring filter (e.g. timeout). Returns JSON.",
        output_schema = output::schema::<output::ConfigResult>(),
        annotations(
            title = "Nix config",
            read_only_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn nix_config(
        &self,
        Parameters(p): Parameters<tools::ConfigParams>,
    ) -> std::result::Result<CallToolResult, rmcp::ErrorData> {
        self.send_op(|reply| NixOp::Config {
            operation: p.operation,
            query: p.query,
            reply,
        })
        .await
    }
}

#[tool_handler]
impl ServerHandler for NixMcp {}

/// Initialize the `tracing` subscriber from the `RUST_LOG` environment.
pub fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();
}

/// Spawn the Nix worker and serve the MCP protocol over stdio until the client
/// disconnects.
///
/// # Errors
///
/// Returns any error from establishing or running the stdio MCP service.
pub async fn serve() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // A panic on the worker thread would otherwise leave a zombie: the thread
    // unwinds, the op channel closes, and every later call gets a "worker
    // disconnected" protocol error while the process keeps answering the
    // handshake. Fail loud instead: log the panic, then abort. `abort` rather
    // than `exit` because nothing may unwind through live Nix FFI frames and
    // the C++ static destructors are abort-prone at teardown; the MCP client
    // sees the pipe close and can relaunch.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default_hook(info);
        std::process::abort();
    }));

    // Bounded so a pipelining client cannot queue ops without limit behind a
    // long realize: once the channel is full, `send_op`'s `send_async` waits,
    // turning a flood into natural async backpressure instead of unbounded
    // memory growth. 32 is far more in flight than a single agent produces.
    let (tx, rx) = flume::bounded(OP_CHANNEL_CAPACITY);
    let (ready_tx, ready_rx) = flume::bounded(1);
    let worker_handle = NixWorker::spawn(rx, ready_tx);

    tracing::info!("nix_mcp server starting; waiting for the Nix worker");

    // Refuse to complete the MCP handshake unless the worker has a live Nix
    // store and evaluator. A server that answers `initialize` with no store
    // behind it would fail every subsequent call on a dead channel, making a
    // client waste requests on a server that can never do work. Fail loud and
    // specific instead, so the failure is legible at connect time.
    match ready_rx.recv_async().await {
        Ok(Ok(())) => {}
        Ok(Err(reason)) => {
            return Err(format!(
                "Nix worker failed to initialize: {reason}. \
                 nix_mcp requires access to a Nix store; refusing to start."
            )
            .into());
        }
        Err(_) => {
            return Err("Nix worker exited before signaling readiness; refusing to start.".into());
        }
    }

    let server = Arc::new(NixMcp {
        worker_tx: tx,
        web: Arc::new(search::WebSearch::new()?),
    });

    tracing::info!("nix_mcp server ready");

    let service = rmcp::serve_server(server, rmcp::transport::io::stdio()).await?;
    service.waiting().await?;

    // `waiting` consumed the service, which held the last clone of the op
    // sender; the closed channel ends the worker's run loop. Join the thread
    // so Nix C++ teardown completes before `main` returns instead of racing
    // process exit (the abort-prone path the worker module docs warn about).
    // Documented tradeoff: a wedged eval blocks exit here, and the client's
    // SIGKILL remains the escape hatch. The error arm is unreachable in
    // practice, since a worker panic aborts the process via the hook above.
    tracing::info!("client disconnected; joining the Nix worker");
    worker_handle
        .join()
        .map_err(|_| "Nix worker panicked during shutdown")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use rmcp::model::{ContentBlock, ErrorCode};

    use super::{
        DEFAULT_CLOSURE_LIMIT, DEFAULT_EVAL_MAX_LEN, DEFAULT_SEARCH_LIMIT, MAX_CLOSURE_LIMIT,
        MAX_EVAL_MAX_LEN, MAX_SEARCH_LIMIT, NixMcp, effective_eval_max_len, effective_limit,
        effective_search_limit,
    };
    use crate::error::NixMcpError;
    use crate::worker::NixOp;

    #[test]
    fn effective_limit_uses_the_default_when_omitted() {
        assert_eq!(effective_limit(None), DEFAULT_CLOSURE_LIMIT as usize);
    }

    #[test]
    fn effective_limit_floors_zero_to_one() {
        assert_eq!(effective_limit(Some(0)), 1);
    }

    #[test]
    fn effective_limit_passes_values_in_range_through() {
        assert_eq!(effective_limit(Some(1)), 1);
        assert_eq!(effective_limit(Some(50)), 50);
        assert_eq!(effective_limit(Some(500)), 500);
    }

    #[test]
    fn effective_limit_caps_at_the_maximum() {
        assert_eq!(effective_limit(Some(501)), MAX_CLOSURE_LIMIT as usize);
        assert_eq!(effective_limit(Some(10_000)), MAX_CLOSURE_LIMIT as usize);
    }

    #[test]
    fn effective_eval_max_len_defaults_floors_and_caps() {
        assert_eq!(effective_eval_max_len(None), DEFAULT_EVAL_MAX_LEN as usize);
        assert_eq!(effective_eval_max_len(Some(0)), 1);
        assert_eq!(effective_eval_max_len(Some(42)), 42);
        assert_eq!(
            effective_eval_max_len(Some(MAX_EVAL_MAX_LEN + 1)),
            MAX_EVAL_MAX_LEN as usize
        );
    }

    #[test]
    fn effective_search_limit_defaults_floors_and_caps() {
        assert_eq!(effective_search_limit(None), DEFAULT_SEARCH_LIMIT as usize);
        assert_eq!(effective_search_limit(Some(0)), 1);
        assert_eq!(effective_search_limit(Some(42)), 42);
        assert_eq!(
            effective_search_limit(Some(MAX_SEARCH_LIMIT + 1)),
            MAX_SEARCH_LIMIT as usize
        );
    }

    /// A `NixMcp` whose "worker" is a plain thread running `handler` over the
    /// op channel; lets the `send_op` contract be tested without any Nix.
    fn with_fake_worker(handler: impl Fn(NixOp) + Send + 'static) -> NixMcp {
        let (tx, rx) = flume::unbounded();
        std::thread::spawn(move || {
            for op in rx {
                handler(op);
            }
        });
        NixMcp {
            worker_tx: tx,
            web: std::sync::Arc::new(crate::search::WebSearch::new().expect("http client")),
        }
    }

    /// The text of the first content block, for asserting on tool results.
    fn first_text(result: &rmcp::model::CallToolResult) -> &str {
        match &result.content[0] {
            ContentBlock::Text(text) => &text.text,
            other => panic!("expected a text block, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn send_op_wraps_a_worker_reply_as_a_structured_tool_result() {
        let server = with_fake_worker(|op| {
            if let NixOp::Ping(reply) = op {
                drop(reply.send(Ok(r#"{"pong":true}"#.to_owned())));
            }
        });

        let result = server.send_op(NixOp::Ping).await.expect("tool result");
        assert_eq!(result.is_error, Some(false));
        // The reply rides in structuredContent, with the serialized text
        // block alongside for clients that predate structured output.
        assert_eq!(
            result.structured_content,
            Some(serde_json::json!({ "pong": true }))
        );
        assert_eq!(first_text(&result), r#"{"pong":true}"#);
    }

    #[tokio::test]
    async fn send_op_rejects_an_unparseable_reply_as_a_protocol_error() {
        let server = with_fake_worker(|op| {
            if let NixOp::Ping(reply) = op {
                drop(reply.send(Ok("not json".to_owned())));
            }
        });

        let err = server
            .send_op(NixOp::Ping)
            .await
            .expect_err("protocol error");
        assert_eq!(err.code, ErrorCode::INTERNAL_ERROR);
        assert!(err.message.contains("unparseable"));
    }

    #[tokio::test]
    async fn send_op_reports_handler_failures_in_band() {
        let server = with_fake_worker(|op| {
            if let NixOp::Ping(reply) = op {
                drop(reply.send(Err(NixMcpError::InvalidArgument("bad input".to_owned()))));
            }
        });

        let result = server.send_op(NixOp::Ping).await.expect("in-band error");
        assert_eq!(result.is_error, Some(true));
        assert_eq!(first_text(&result), "Invalid argument: bad input");
    }

    #[tokio::test]
    async fn send_op_maps_a_dead_worker_to_a_protocol_error() {
        let (tx, rx) = flume::unbounded();
        drop(rx);
        let server = NixMcp {
            worker_tx: tx,
            web: std::sync::Arc::new(crate::search::WebSearch::new().expect("http client")),
        };

        let err = server
            .send_op(NixOp::Ping)
            .await
            .expect_err("protocol error");
        assert_eq!(err.code, ErrorCode::INTERNAL_ERROR);
        assert!(err.message.contains("Worker channel error"));
        assert!(err.message.contains("worker disconnected"));
    }

    #[test]
    fn tool_router_pins_the_tool_surface() {
        let router = NixMcp::tool_router();
        let tools = router.list_all();

        let mut names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
        names.sort_unstable();
        assert_eq!(
            names,
            [
                "nix_config",
                "nix_derivation_info",
                "nix_eval",
                "nix_flake_resolve",
                "nix_realize",
                "nix_search",
                "nix_store_info",
                "nix_store_path_query",
            ]
        );

        let tool = |name: &str| {
            tools
                .iter()
                .find(|t| t.name == name)
                .unwrap_or_else(|| panic!("tool {name} exists"))
        };

        // The operation enum must reach the input schema; a dropped or
        // renamed variant silently invalidates the tool's documentation.
        let query_schema = serde_json::to_string(&tool("nix_store_path_query").input_schema)
            .expect("schema serializes");
        for variant in ["validate", "real_path", "closure", "resolve_hash"] {
            assert!(
                query_schema.contains(variant),
                "operation variant {variant} missing from the input schema"
            );
        }

        // Annotation spot checks: honest hints are a shipped guarantee
        // (ROADMAP rubric #3); every hint is set explicitly, never defaulted.
        let annotations = |name: &str| {
            tool(name)
                .annotations
                .as_ref()
                .unwrap_or_else(|| panic!("tool {name} declares annotations"))
        };
        assert_eq!(annotations("nix_store_info").read_only_hint, Some(true));
        assert_eq!(
            annotations("nix_store_path_query").read_only_hint,
            Some(true)
        );
        assert_eq!(
            annotations("nix_derivation_info").read_only_hint,
            Some(true)
        );
        assert_eq!(annotations("nix_config").read_only_hint, Some(true));
        assert_eq!(annotations("nix_config").open_world_hint, Some(false));
        // eval and flake_resolve write to the store and reach the network.
        assert_eq!(annotations("nix_eval").read_only_hint, Some(false));
        assert_eq!(annotations("nix_eval").open_world_hint, Some(true));
        assert_eq!(annotations("nix_flake_resolve").read_only_hint, Some(false));
        assert_eq!(annotations("nix_flake_resolve").open_world_hint, Some(true));
        assert_eq!(annotations("nix_realize").read_only_hint, Some(false));
        assert_eq!(annotations("nix_realize").open_world_hint, Some(true));
        // search is read-only but reaches the network by default.
        assert_eq!(annotations("nix_search").read_only_hint, Some(true));
        assert_eq!(annotations("nix_search").open_world_hint, Some(true));

        // Both search enums must reach the input schema, like the store
        // query's operation enum above.
        let search_schema =
            serde_json::to_string(&tool("nix_search").input_schema).expect("schema serializes");
        for token in ["package", "option", "web", "pinned"] {
            assert!(
                search_schema.contains(token),
                "search variant {token} missing from the input schema"
            );
        }

        let config_schema =
            serde_json::to_string(&tool("nix_config").input_schema).expect("schema serializes");
        for variant in ["get", "show", "list"] {
            assert!(
                config_schema.contains(variant),
                "config operation variant {variant} missing from the input schema"
            );
        }

        // Structured output is declared on every tool.
        for t in &tools {
            assert!(
                t.output_schema.is_some(),
                "{} lacks an outputSchema",
                t.name
            );
        }
    }

    #[tokio::test]
    async fn send_op_maps_a_dropped_reply_to_a_protocol_error() {
        let server = with_fake_worker(drop);

        let err = server
            .send_op(NixOp::Ping)
            .await
            .expect_err("protocol error");
        assert_eq!(err.code, ErrorCode::INTERNAL_ERROR);
        assert!(err.message.contains("worker dropped the reply"));
    }
}

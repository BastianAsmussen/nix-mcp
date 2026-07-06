//! The Nix worker thread and its operation dispatch.
//!
//! `nix-bindings` types are `Send` but `!Sync` (the Nix C evaluator is
//! single-threaded), so a single [`NixWorker`] owns all Nix state and processes
//! [`NixOp`] messages sequentially. The async MCP handler never touches Nix
//! state directly; it sends ops over a `flume` channel and awaits the reply.
//! A dedicated `std::thread` is the pattern Tokio recommends for persistent
//! blocking work (as opposed to `spawn_blocking`, which is for short bounded
//! tasks) - <https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html>.
//!
//! ## FFI exception safety
//!
//! The Nix evaluator is C++. A thrown C++ exception unwinding into Rust across a
//! plain `extern "C"` boundary is undefined behavior, and `catch_unwind` cannot
//! reliably intercept a foreign exception, so `nix_mcp` never tries to catch
//! one. Instead every Nix call here goes through a `nix-bindings` method that
//! carries a `Context`; the Nix C API catches C++ exceptions internally and
//! reports them through that context, so they arrive as [`NixMcpError::Nix`] (a
//! normal `Err`) rather than aborting the process. Context-less free helpers
//! that pass a null context (e.g. `nix_bindings::is_pure_eval`) are avoided
//! precisely because a throw there aborts. A genuine `panic!` on this thread is
//! treated as a fatal, unrecoverable bug: a panic mid-call could leave the Nix
//! C++ state inconsistent, so the worker does not catch-and-continue.

use std::sync::Arc;

use nix_bindings::{Context, EvalState, EvalStateBuilder, Store, flake::FlakeSettings};

use crate::error::{NixMcpError, Result};
use crate::output;
use crate::search;
use crate::tools::{ClosureType, Page, SearchOperation, StorePathOperation, truncate_chars};

/// Serialize a reply payload built from one of the [`crate::output`] structs.
///
/// # Errors
///
/// Returns [`NixMcpError::Internal`] if serialization fails, which cannot
/// happen for the output structs in practice; the mapping exists so a future
/// bug surfaces as a legible error instead of a worker panic (which aborts
/// the process by policy).
fn render<T: serde::Serialize>(payload: &T) -> Result<String> {
    Ok(serde_json::to_string(payload)?)
}

/// Reply channel a handler uses to return its JSON result to the caller.
pub type Reply = flume::Sender<Result<String>>;

/// A unit of work for the [`NixWorker`], each carrying its own reply channel.
pub enum NixOp {
    /// Health check; replies with `{"pong": true}`.
    Ping(Reply),
    /// Report store URI, directory, and version information.
    StoreInfo(Reply),
    /// Evaluate a Nix expression, optionally accessing one attribute.
    Eval {
        /// The expression to evaluate.
        expression: String,
        /// Optional attribute to access on the result.
        attr: Option<String>,
        /// Cap, in characters, on the rendered result (resolved by the
        /// caller; always at least 1).
        max_len: usize,
        /// Where to send the JSON reply.
        reply: Reply,
    },
    /// Query a store path (see [`StorePathOperation`]).
    StorePathQuery {
        /// Store-path argument, if provided.
        path: Option<String>,
        /// Hash-part argument, if provided.
        hash: Option<String>,
        /// Which query to run.
        operation: StorePathOperation,
        /// `closure` only: which dependency graph to walk.
        closure_type: ClosureType,
        /// `closure` only: page offset into the sorted result.
        offset: usize,
        /// `closure` only: page size.
        limit: usize,
        /// `closure` only: case-insensitive substring filter on entry names.
        filter: Option<String>,
        /// `closure` only: emit `{path, name}` objects instead of plain
        /// path strings.
        include_names: bool,
        /// Where to send the JSON reply.
        reply: Reply,
    },
    /// Realize (build or substitute) a store path and its dependencies.
    Realize {
        /// The store path to realize.
        path: String,
        /// Where to send the JSON reply.
        reply: Reply,
    },
    /// Resolve and lock a flake reference, returning its output attr names.
    FlakeResolve {
        /// The flake reference to resolve.
        url: String,
        /// Where to send the JSON reply.
        reply: Reply,
    },
    /// Read derivation metadata from a `.drv` store path.
    DerivationInfo {
        /// The `.drv` store path.
        path: String,
        /// Return the full derivation JSON instead of the capped summary.
        verbose: bool,
        /// Where to send the JSON reply.
        reply: Reply,
    },
    /// `nix_search` with `source=pinned`: search a nixpkgs flake in-process.
    Search {
        /// What to search for.
        operation: SearchOperation,
        /// The search text.
        query: String,
        /// The flake reference to evaluate.
        flake: String,
        /// Page offset into the name-sorted matches.
        offset: usize,
        /// Page size.
        limit: usize,
        /// Where to send the JSON reply.
        reply: Reply,
    },
}

/// Per-value cap, in characters, applied to derivation JSON in summary mode.
/// Long enough to keep every real metadata field intact; short enough that
/// inlined build scripts and `passAsFile` blobs stop dominating the result.
const DERIVATION_VALUE_MAX_CHARS: usize = 500;

/// Recursively cap every string in `value` longer than `max_chars`,
/// appending an inline marker naming the cut and the way to the full text.
/// Returns whether anything was capped.
fn cap_long_strings(value: &mut serde_json::Value, max_chars: usize) -> bool {
    match value {
        serde_json::Value::String(s) => {
            let (kept, truncated) = truncate_chars(s, max_chars);
            if !truncated {
                return false;
            }
            let total = s.chars().count();
            let capped =
                format!("{kept} <truncated, {total} chars total; pass verbose=true for the rest>");
            *s = capped;
            true
        }
        serde_json::Value::Array(items) => {
            let mut any = false;
            for item in items {
                any |= cap_long_strings(item, max_chars);
            }
            any
        }
        serde_json::Value::Object(map) => {
            let mut any = false;
            for item in map.values_mut() {
                any |= cap_long_strings(item, max_chars);
            }
            any
        }
        _ => false,
    }
}

/// Owns all Nix state and runs [`NixOp`]s one at a time on its own thread.
pub struct NixWorker {
    /// The Nix C API context, which owns the process-global init.
    ctx: Arc<Context>,
    /// The opened store.
    store: Arc<Store>,
    /// Flake settings registered into the evaluator (so `builtins.getFlake`
    /// works) and reused by `handle_flake_resolve`.
    flake_settings: Arc<FlakeSettings>,
    /// The evaluator state.
    eval_state: EvalState,
}

impl NixWorker {
    /// Initialize a fresh Nix context, store, and flake-aware evaluator.
    ///
    /// Passing `None` to [`Store::open`] uses Nix's own default-store
    /// resolution (honoring `NIX_REMOTE` and the system config), so the server
    /// talks to whatever store the system does. `Store::open` connects lazily,
    /// though, so this also *probes* the store with a tiny content-addressed
    /// write to force real access: a missing store or unreachable daemon fails
    /// here, at startup, rather than silently deferring to the client's first
    /// call.
    ///
    /// Enables the `flakes` experimental feature and registers flake settings
    /// into the [`EvalState`], so `builtins.getFlake` and flake-aware evaluation
    /// work in [`Self::handle_eval`].
    ///
    /// # Errors
    ///
    /// Returns [`NixMcpError::Nix`] if the context or store cannot be created,
    /// the store cannot be reached, or the settings/evaluator cannot be built.
    pub fn new() -> Result<Self> {
        let ctx = Arc::new(Context::new()?);
        ctx.set_setting("experimental-features", "nix-command flakes")?;

        let store = Arc::new(Store::open(&ctx, None)?);
        // Force a real store round-trip now; `Store::open` alone connects
        // lazily. A tiny content-addressed write is the probe of choice:
        // `Store::version` reports connection failure as an opaque "Null
        // pointer error" and `query_path_from_hash_part` swallows it entirely
        // (both verified empirically against a dead daemon socket), while a
        // write surfaces the daemon's actual error *and* proves the store is
        // writable, which realize and .drv-producing evaluation need anyway.
        // The path is content-addressed, so repeated startups dedup to the
        // same store path instead of accumulating garbage.
        store.add_bytes_to_store("nix-mcp-readiness-probe", b"nix_mcp readiness probe\n")?;

        let flake_settings = Arc::new(FlakeSettings::new(&ctx)?);
        let eval_state = EvalStateBuilder::new(&store)?
            .with_flake_settings(&flake_settings)?
            .build()?;

        Ok(Self {
            ctx,
            store,
            flake_settings,
            eval_state,
        })
    }

    /// Spawn the worker thread, returning its join handle.
    ///
    /// The thread initializes Nix state and reports the outcome over `ready`:
    /// `Ok(())` once the store and evaluator are live, or `Err(reason)` if
    /// initialization failed. It then blocks in [`Self::run`], draining `rx`
    /// until the channel closes. Callers must wait on `ready` before serving,
    /// so the server never accepts a client it cannot answer (a storeless
    /// server that completes the handshake and then fails every call is worse
    /// than one that refuses to start).
    pub(crate) fn spawn(
        rx: flume::Receiver<NixOp>,
        ready: flume::Sender<std::result::Result<(), String>>,
    ) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            let worker = match Self::new() {
                Ok(worker) => worker,
                Err(err) => {
                    tracing::error!(%err, "failed to initialize Nix worker");
                    drop(ready.send(Err(err.to_string())));
                    return;
                }
            };

            if ready.send(Ok(())).is_err() {
                // The parent stopped waiting before we came up; nothing to serve.
                return;
            }

            worker.run(rx);
        })
    }

    /// Process each incoming [`NixOp`] sequentially until `rx` closes.
    fn run(&self, rx: flume::Receiver<NixOp>) {
        for op in rx {
            match op {
                NixOp::Ping(reply) => {
                    drop(reply.send(Ok(serde_json::json!({ "pong": true }).to_string())));
                }
                NixOp::StoreInfo(reply) => drop(reply.send(self.handle_store_info())),
                NixOp::Eval {
                    expression,
                    attr,
                    max_len,
                    reply,
                } => drop(reply.send(self.handle_eval(&expression, attr.as_deref(), max_len))),
                NixOp::StorePathQuery {
                    path,
                    hash,
                    operation,
                    closure_type,
                    offset,
                    limit,
                    filter,
                    include_names,
                    reply,
                } => drop(reply.send(self.handle_store_path_query(
                    path.as_deref(),
                    hash.as_deref(),
                    operation,
                    closure_type,
                    offset,
                    limit,
                    filter.as_deref(),
                    include_names,
                ))),
                NixOp::Realize { path, reply } => drop(reply.send(self.handle_realize(&path))),
                NixOp::FlakeResolve { url, reply } => {
                    drop(reply.send(self.handle_flake_resolve(&url)));
                }
                NixOp::DerivationInfo {
                    path,
                    verbose,
                    reply,
                } => {
                    drop(reply.send(self.handle_derivation_info(&path, verbose)));
                }
                NixOp::Search {
                    operation,
                    query,
                    flake,
                    offset,
                    limit,
                    reply,
                } => {
                    drop(
                        reply.send(
                            self.handle_search_pinned(operation, &query, &flake, offset, limit),
                        ),
                    );
                }
            }
        }
    }

    /// Report store URI and directory, the daemon version, the linked Nix
    /// library version, and the evaluator's current system.
    ///
    /// # Errors
    ///
    /// Returns [`NixMcpError::Nix`] if a store query or the system eval fails.
    pub fn handle_store_info(&self) -> Result<String> {
        // Agents need the platform string to pick flake attrs like
        // `packages.x86_64-linux.hello`; the evaluator is the source of truth.
        let system = self
            .eval_state
            .eval_from_string("builtins.currentSystem", "<mcp>")?
            .as_string()?;

        render(&output::StoreInfo {
            uri: self.store.uri()?,
            store_dir: self.store.store_dir()?,
            version: self.store.version()?,
            // `nix_version()` is context-less but safe: it returns a static
            // string with no throw path, unlike the banned null-context
            // helpers (see the module docs).
            nix_version: nix_bindings::nix_version().to_owned(),
            system,
        })
    }

    /// Evaluate `expression`, optionally accessing `attr`, and render the
    /// value, truncating it to `max_len` characters.
    ///
    /// # Errors
    ///
    /// Returns [`NixMcpError::InvalidArgument`] if the expression is empty or
    /// the requested attribute cannot be accessed, or [`NixMcpError::Nix`] if
    /// evaluation or rendering fails.
    pub fn handle_eval(
        &self,
        expression: &str,
        attr: Option<&str>,
        max_len: usize,
    ) -> Result<String> {
        // Catch this before the parser turns it into a cryptic
        // "unexpected end of file".
        if expression.trim().is_empty() {
            return Err(NixMcpError::InvalidArgument(
                "expression is empty; provide a Nix expression, e.g. builtins.nixVersion"
                    .to_owned(),
            ));
        }

        let value = self.eval_state.eval_from_string(expression, "<mcp>")?;
        let value = match attr {
            Some(name) => value.get_attr(name).map_err(|err| {
                NixMcpError::InvalidArgument(format!(
                    "cannot access attribute '{name}': {err}; evaluate without `attr` or list \
                     the keys with builtins.attrNames"
                ))
            })?,
            None => value,
        };
        let result = value.to_nix_string()?;

        // The untruncated path pays zero extra tokens: no metadata fields.
        let (shown, truncated) = truncate_chars(&result, max_len);
        if !truncated {
            return render(&output::EvalResult {
                result,
                truncated: None,
                total_len: None,
                hint: None,
            });
        }

        render(&output::EvalResult {
            result: shown.to_owned(),
            truncated: Some(true),
            total_len: Some(result.chars().count()),
            hint: Some(
                "result truncated; narrow with `attr` or builtins.substring, or raise \
                 max_len (cap 100000)"
                    .to_owned(),
            ),
        })
    }

    /// Run one store-path query.
    ///
    /// # Errors
    ///
    /// Returns [`NixMcpError::InvalidArgument`] if a required argument is
    /// missing, or [`NixMcpError::Nix`] if an underlying store call fails.
    #[expect(
        clippy::too_many_arguments,
        reason = "mirrors the flat NixOp::StorePathQuery wire shape; the per-operation \
                  tagged-enum schema (ROADMAP P2) will collapse this"
    )]
    pub fn handle_store_path_query(
        &self,
        path: Option<&str>,
        hash: Option<&str>,
        operation: StorePathOperation,
        closure_type: ClosureType,
        offset: usize,
        limit: usize,
        filter: Option<&str>,
        include_names: bool,
    ) -> Result<String> {
        match operation {
            StorePathOperation::Validate => self.query_validate(path, hash),
            StorePathOperation::RealPath => self.query_real_path(path),
            StorePathOperation::Closure => {
                self.query_closure(path, closure_type, offset, limit, filter, include_names)
            }
            StorePathOperation::ResolveHash => self.query_resolve_hash(hash),
        }
    }

    /// `validate`: report whether a path (or hash part) names a valid store path.
    ///
    /// # Errors
    ///
    /// Returns [`NixMcpError::InvalidArgument`] if neither `path` nor `hash`
    /// is given, or [`NixMcpError::Nix`] if a store call fails.
    fn query_validate(&self, path: Option<&str>, hash: Option<&str>) -> Result<String> {
        let store_path = match (path, hash) {
            (Some(p), _) => self.store.store_path(p)?,
            // A hash miss is an answer, not a caller mistake: report it in
            // the same soft shape `resolve_hash` uses.
            (_, Some(h)) => match self.store.query_path_from_hash_part(h)? {
                Some(sp) => sp,
                None => {
                    return render(&output::StorePathQueryResult {
                        valid: Some(false),
                        found: Some(false),
                        ..output::StorePathQueryResult::default()
                    });
                }
            },
            (None, None) => {
                return Err(NixMcpError::InvalidArgument(
                    "operation=validate requires either 'path' (a full store path, \
                     /nix/store/<hash>-<name>) or 'hash' (the 32-character base-32 prefix of \
                     the basename)"
                        .to_owned(),
                ));
            }
        };

        render(&output::StorePathQueryResult {
            valid: Some(self.store.is_valid_path(&store_path)),
            path: Some(self.store.print_path(&store_path)?),
            name: Some(store_path.name()?),
            ..output::StorePathQueryResult::default()
        })
    }

    /// `real_path`: resolve a store path to its real filesystem location.
    ///
    /// # Errors
    ///
    /// Returns [`NixMcpError::InvalidArgument`] if `path` is missing, or
    /// [`NixMcpError::Nix`] if a store call fails.
    fn query_real_path(&self, path: Option<&str>) -> Result<String> {
        let p = path.ok_or_else(|| {
            NixMcpError::InvalidArgument(
                "operation=real_path requires 'path' (a full store path, e.g. \
                 /nix/store/<hash>-<name>)"
                    .to_owned(),
            )
        })?;

        let store_path = self.store.store_path(p)?;
        render(&output::StorePathQueryResult {
            path: Some(self.store.print_path(&store_path)?),
            real_path: Some(self.store.real_path(&store_path)?),
            ..output::StorePathQueryResult::default()
        })
    }

    /// `closure`: walk a dependency graph of `path` and return one page of
    /// it, optionally narrowed to entries whose name matches `filter`.
    ///
    /// `count` and the pages reflect only matching entries. Pages are flat
    /// arrays of store-path strings unless `include_names` asks for
    /// `{path, name}` objects (the name is derivable from the path, so the
    /// default spares the caller the duplicate tokens).
    ///
    /// # Errors
    ///
    /// Returns [`NixMcpError::InvalidArgument`] if `path` is missing, or
    /// [`NixMcpError::Nix`] if the closure cannot be collected or rendered.
    fn query_closure(
        &self,
        path: Option<&str>,
        closure_type: ClosureType,
        offset: usize,
        limit: usize,
        filter: Option<&str>,
        include_names: bool,
    ) -> Result<String> {
        let p = path.ok_or_else(|| {
            NixMcpError::InvalidArgument(
                "operation=closure requires 'path' (a full store path, e.g. \
                 /nix/store/<hash>-<name>)"
                    .to_owned(),
            )
        })?;

        let store_path = self.store.store_path(p)?;
        let (flip_direction, include_outputs, include_derivers) = closure_type.flags();
        let closure = self.store.collect_fs_closure(
            &store_path,
            flip_direction,
            include_outputs,
            include_derivers,
        )?;

        // Materialize (name, printed path) up front: the filter matches on
        // names, the sort needs the full path as a tiebreak (names collide
        // across versions, and a name-only key would leave page stability
        // resting on the traversal order), and a key error must fail the
        // query instead of silently sorting as an empty string.
        let filter_lowered = filter.map(str::to_lowercase);
        let mut entries = Vec::with_capacity(closure.len());
        for sp in &closure {
            let name = sp.name()?;
            if let Some(f) = &filter_lowered
                && !name.to_lowercase().contains(f.as_str())
            {
                continue;
            }
            entries.push((name, self.store.print_path(sp)?));
        }
        entries.sort_unstable();

        let count = entries.len();
        let page = Page::new(count, offset, limit);
        let window = &entries[page.start..page.end];

        let paths: Vec<output::ClosureEntry> = window
            .iter()
            .map(|(name, path)| {
                if include_names {
                    output::ClosureEntry::Named {
                        path: path.clone(),
                        name: name.clone(),
                    }
                } else {
                    output::ClosureEntry::Path(path.clone())
                }
            })
            .collect();

        render(&output::StorePathQueryResult {
            root: Some(self.store.print_path(&store_path)?),
            closure_type: Some(closure_type.as_str().to_owned()),
            count: Some(count),
            offset: Some(offset),
            limit: Some(limit),
            has_more: Some(page.has_more),
            next_offset: Some(page.next_offset),
            filter: filter.map(str::to_owned),
            paths: Some(paths),
            ..output::StorePathQueryResult::default()
        })
    }

    /// `resolve_hash`: resolve a hash part to its full store path.
    ///
    /// # Errors
    ///
    /// Returns [`NixMcpError::InvalidArgument`] if `hash` is missing, or
    /// [`NixMcpError::Nix`] if the lookup fails.
    fn query_resolve_hash(&self, hash: Option<&str>) -> Result<String> {
        let h = hash.ok_or_else(|| {
            NixMcpError::InvalidArgument(
                "operation=resolve_hash requires 'hash' (the 32-character base-32 prefix of \
                 the store path basename)"
                    .to_owned(),
            )
        })?;

        match self.store.query_path_from_hash_part(h)? {
            Some(sp) => render(&output::StorePathQueryResult {
                found: Some(true),
                path: Some(self.store.print_path(&sp)?),
                name: Some(sp.name()?),
                ..output::StorePathQueryResult::default()
            }),
            None => render(&output::StorePathQueryResult {
                found: Some(false),
                ..output::StorePathQueryResult::default()
            }),
        }
    }

    /// Realize a `.drv` store path and return its outputs with full store
    /// paths.
    ///
    /// # Errors
    ///
    /// Returns [`NixMcpError::InvalidArgument`] if the path is not a `.drv`
    /// path, or [`NixMcpError::Nix`] if it is invalid or the build fails.
    pub fn handle_realize(&self, path: &str) -> Result<String> {
        // The C API's realise parses its argument as a derivation, so a plain
        // output path can never be realized here; probe-confirmed on a valid
        // store path ("is not a valid derivation path"). Catch it up front
        // with a usable error instead of Nix's parse failure.
        let is_drv = std::path::Path::new(path)
            .extension()
            .is_some_and(|ext| ext == "drv");
        if !is_drv {
            return Err(NixMcpError::InvalidArgument(format!(
                "'{path}' is not a .drv store path; nix_realize builds derivations only and \
                 cannot fetch plain output paths at the current Nix C API. To check whether an \
                 output path already exists, use nix_store_path_query with operation=validate"
            )));
        }

        let store_path = self.store.store_path(path)?;
        let outputs = self.store.realize(&store_path)?;

        let mut result = Vec::with_capacity(outputs.len());
        for (output_name, sp) in &outputs {
            result.push(output::RealizeEntry {
                output_name: output_name.clone(),
                path: self.store.print_path(sp)?,
                name: sp.name()?,
            });
        }

        let count = result.len();
        render(&output::RealizeResult {
            outputs: result,
            count,
        })
    }

    /// Resolve and lock a flake reference, returning its output attribute names.
    ///
    /// # Errors
    ///
    /// Returns [`NixMcpError::InvalidArgument`] if the reference carries a
    /// `#fragment`, or [`NixMcpError::Nix`] if it cannot be parsed, locked, or
    /// its outputs enumerated.
    pub fn handle_flake_resolve(&self, url: &str) -> Result<String> {
        use nix_bindings::flake;

        let flake_settings = Arc::clone(&self.flake_settings);
        let fetch_settings = flake::FetchersSettings::new(&self.ctx)?;
        let parse_flags = flake::FlakeReferenceParseFlags::new(&self.ctx, &flake_settings)?;

        let (flake_ref, fragment) = flake::FlakeReference::parse(
            &self.ctx,
            &fetch_settings,
            &flake_settings,
            &parse_flags,
            url,
        )?;

        // Silently ignoring the selector would return top-level outputs as if
        // the caller never asked for an attribute: a quiet wrong answer. Fail
        // loud until fragment descent is implemented.
        if !fragment.is_empty() {
            return Err(NixMcpError::InvalidArgument(format!(
                "flake fragment '#{fragment}' is not supported yet; pass the bare flake ref \
                 (e.g. github:NixOS/nixpkgs) and pick an attribute from the returned outputs"
            )));
        }

        let lock_flags = flake::LockFlags::new(&self.ctx, &flake_settings)?
            .set_mode(flake::LockMode::Virtual)?;

        let locked = flake::LockedFlake::lock(
            &self.ctx,
            &fetch_settings,
            &flake_settings,
            &self.eval_state,
            &lock_flags,
            &flake_ref,
        )?;

        let output_attrs = locked.output_attrs(&flake_settings, &self.eval_state)?;
        let keys: Vec<String> = output_attrs.attr_keys()?.into_iter().collect();

        render(&output::FlakeResolveResult {
            url: url.to_owned(),
            outputs: keys,
        })
    }

    /// Read a derivation from the store and return its JSON metadata: a
    /// summary with long values capped by default, or the full JSON when
    /// `verbose` is set.
    ///
    /// # Errors
    ///
    /// Returns [`NixMcpError::InvalidArgument`] if the path is not a `.drv`
    /// path, or [`NixMcpError::Nix`] if it is invalid or the derivation cannot
    /// be read or serialized.
    pub fn handle_derivation_info(&self, path: &str, verbose: bool) -> Result<String> {
        // Catch the obvious misuse (an output path) before Nix produces a
        // store-path parse error about it. Case-sensitive on purpose: the
        // store only ever writes lowercase `.drv`.
        let is_drv = std::path::Path::new(path)
            .extension()
            .is_some_and(|ext| ext == "drv");
        if !is_drv {
            return Err(NixMcpError::InvalidArgument(format!(
                "'{path}' is not a .drv store path; pass a derivation path like \
                 /nix/store/<hash>-<name>.drv (find one via nix_store_path_query with \
                 operation=closure, closure_type=build)"
            )));
        }

        let store_path = self.store.store_path(path)?;
        let derivation = self.store.read_derivation(&store_path)?;
        let json = derivation.to_json()?;

        if verbose {
            return Ok(json);
        }

        // Summary mode: the drv JSON's env (and sometimes args or
        // structuredAttrs) carries whole build scripts and `passAsFile`
        // blobs, tens of thousands of tokens an agent rarely wants. Cap long
        // strings anywhere in the document; real metadata fields are far
        // shorter than the cap. If Nix ever hands back something unparseable,
        // fall through to the raw JSON rather than failing the call.
        let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&json) else {
            return Ok(json);
        };
        if cap_long_strings(&mut value, DERIVATION_VALUE_MAX_CHARS) && value.is_object() {
            value["truncated"] = true.into();
            value["hint"] =
                "long values were truncated; pass verbose=true for the full derivation JSON".into();
        }

        Ok(value.to_string())
    }

    /// `nix_search` with `source=pinned`: evaluate a generated expression
    /// (see [`search::pinned_expression`]) against a nixpkgs flake, matching
    /// attribute/option names case-insensitively and forcing details only
    /// for the page of hits.
    ///
    /// # Errors
    ///
    /// Returns [`NixMcpError::InvalidArgument`] if `query` or `flake` is
    /// empty, [`NixMcpError::Nix`] if evaluation fails (an unknown flake
    /// ref, a registry miss, an offline first fetch), or
    /// [`NixMcpError::Internal`] if the expression's reply does not parse,
    /// which would be a template bug.
    pub fn handle_search_pinned(
        &self,
        operation: SearchOperation,
        query: &str,
        flake: &str,
        offset: usize,
        limit: usize,
    ) -> Result<String> {
        if query.trim().is_empty() {
            return Err(NixMcpError::InvalidArgument(
                "query is empty; pass a name fragment like 'ripgrep' or 'services.nginx'"
                    .to_owned(),
            ));
        }
        if flake.trim().is_empty() {
            return Err(NixMcpError::InvalidArgument(
                "flake is empty; pass a flake ref like 'nixpkgs' (the registry default) or \
                 github:NixOS/nixpkgs/<rev>"
                    .to_owned(),
            ));
        }

        let expression = search::pinned_expression(operation, query, flake, offset, limit);
        let json = self
            .eval_state
            .eval_from_string(&expression, "<mcp-search>")?
            .as_string()?;

        let reply: search::PinnedReply = serde_json::from_str(&json).map_err(|err| {
            NixMcpError::Internal(format!(
                "pinned search returned unexpected JSON ({err}); this is a template bug"
            ))
        })?;

        let page = Page::new(reply.count, offset, limit);
        render(&output::SearchResult {
            query: query.to_owned(),
            operation: operation.as_str().to_owned(),
            source: "pinned".to_owned(),
            channel: None,
            flake: Some(flake.to_owned()),
            count: reply.count,
            offset,
            limit,
            has_more: page.has_more,
            next_offset: page.next_offset,
            results: reply
                .results
                .into_iter()
                .map(search::PinnedItem::into_output)
                .collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use serial_test::serial;

    use super::{NixOp, NixWorker};

    // These exercise the real Nix evaluator and store. That state is
    // process-global and single-threaded, so the tests are `#[serial]`; they
    // run under `cargo test` in the dev shell, where a store is available (the
    // sandboxed `nix build` sets `doCheck = false` and does not run them).

    #[test]
    #[serial]
    fn eval_renders_an_arithmetic_result() {
        let worker = NixWorker::new().expect("worker init");
        assert_eq!(
            worker.handle_eval("1 + 41", None, 10_000).expect("eval"),
            r#"{"result":"42"}"#
        );
    }

    #[test]
    #[serial]
    fn eval_can_access_an_attribute() {
        let worker = NixWorker::new().expect("worker init");
        assert_eq!(
            worker
                .handle_eval("{ answer = 42; }", Some("answer"), 10_000)
                .expect("eval attr"),
            r#"{"result":"42"}"#
        );
    }

    #[test]
    #[serial]
    fn eval_surfaces_errors() {
        let worker = NixWorker::new().expect("worker init");
        worker
            .handle_eval("this-is-undefined", None, 10_000)
            .expect_err("undefined variable is an error");
    }

    #[test]
    #[serial]
    fn flake_builtins_are_available() {
        let worker = NixWorker::new().expect("worker init");
        assert_eq!(
            worker
                .handle_eval("builtins ? getFlake", None, 10_000)
                .expect("eval"),
            r#"{"result":"true"}"#
        );
    }

    #[test]
    #[serial]
    fn store_info_reports_store_dir_versions_and_system() {
        let worker = NixWorker::new().expect("worker init");
        let out = worker.handle_store_info().expect("store info");
        let info: serde_json::Value = serde_json::from_str(&out).expect("json");
        assert_eq!(info["store_dir"], "/nix/store");
        assert!(info["version"].is_string());
        assert!(info["nix_version"].is_string());
        // A bare platform string like "x86_64-linux", not a quoted rendering.
        let system = info["system"].as_str().expect("system is a string");
        assert!(system.contains('-'), "unexpected system string: {system}");
        assert!(!system.starts_with('"'));
    }

    #[test]
    #[serial]
    fn eval_rejects_an_empty_expression() {
        let worker = NixWorker::new().expect("worker init");
        let err = worker
            .handle_eval("   ", None, 10_000)
            .expect_err("empty expression is rejected");
        assert!(err.to_string().contains("expression is empty"));
    }

    #[test]
    #[serial]
    fn eval_attr_misses_are_prescriptive() {
        let worker = NixWorker::new().expect("worker init");
        let err = worker
            .handle_eval("1 + 1", Some("missing"), 10_000)
            .expect_err("attr on a non-attrset is an error");
        let msg = err.to_string();
        assert!(msg.contains("attribute 'missing'"), "got: {msg}");
        assert!(msg.contains("builtins.attrNames"), "got: {msg}");
    }

    #[test]
    #[serial]
    fn validate_by_hash_miss_is_a_soft_result() {
        use crate::tools::{ClosureType, StorePathOperation};

        let worker = NixWorker::new().expect("worker init");
        let out = worker
            .handle_store_path_query(
                None,
                // 32 valid base-32 chars; a content-addressed collision with a
                // real path is effectively impossible.
                Some("00000000000000000000000000000000"),
                StorePathOperation::Validate,
                ClosureType::Runtime,
                0,
                100,
                None,
                false,
            )
            .expect("a miss is an answer, not an error");
        let info: serde_json::Value = serde_json::from_str(&out).expect("json");
        assert_eq!(info["valid"], false);
        assert_eq!(info["found"], false);
    }

    #[test]
    #[serial]
    fn flake_resolve_rejects_fragments() {
        let worker = NixWorker::new().expect("worker init");
        // Parsing is offline; the rejection fires before any locking/fetching.
        let err = worker
            .handle_flake_resolve("github:NixOS/nixpkgs#hello")
            .expect_err("fragments are rejected until supported");
        let msg = err.to_string();
        assert!(msg.contains("fragment"), "got: {msg}");
        assert!(msg.contains("#hello"), "got: {msg}");
    }

    #[test]
    #[serial]
    fn derivation_info_requires_a_drv_path() {
        let worker = NixWorker::new().expect("worker init");
        let err = worker
            .handle_derivation_info("/nix/store/w816gk8hp9sbghl81rb11qvz3v563zn2-hello", false)
            .expect_err("non-drv paths are rejected");
        let msg = err.to_string();
        assert!(msg.contains(".drv"), "got: {msg}");
        assert!(msg.contains("closure_type=build"), "got: {msg}");
    }

    #[test]
    #[serial]
    fn error_messages_contain_no_ansi_escapes() {
        let worker = NixWorker::new().expect("worker init");
        // A malformed store path is the error class observed leaking raw SGR
        // color codes over MCP before stripping landed in `NixMcpError`.
        let err = worker
            .handle_store_path_query(
                Some("not-a-store-path"),
                None,
                crate::tools::StorePathOperation::Validate,
                crate::tools::ClosureType::Runtime,
                0,
                100,
                None,
                false,
            )
            .expect_err("malformed path is an error");
        let msg = err.to_string();
        assert!(!msg.contains('\u{1b}'), "ANSI escape leaked: {msg:?}");
    }

    /// Instantiate (never build) a tiny fixture derivation and return its
    /// `.drv` store path. Content-addressed inputs keep the path stable
    /// across runs; evaluating `.drvPath` writes the `.drv` to the store
    /// without building anything. The fs-closure of the `.drv` is exactly
    /// itself plus its three `toFile` sources: four known entries.
    fn instantiate_fixture_drv(worker: &NixWorker) -> String {
        const EXPR: &str = r#"(derivation {
            name = "nix-mcp-test-fixture";
            system = builtins.currentSystem;
            builder = "/bin/sh";
            longEnv = builtins.concatStringsSep "" (builtins.genList (_: "y") 1000);
            srcAlpha = builtins.toFile "nix-mcp-fixture-alpha" "aaa";
            srcBeta = builtins.toFile "nix-mcp-fixture-beta" "bbb";
            srcGamma = builtins.toFile "nix-mcp-fixture-gamma" "ccc";
        }).drvPath"#;

        let out = worker
            .handle_eval(EXPR, None, 100_000)
            .expect("instantiate the fixture drv");
        let info: serde_json::Value = serde_json::from_str(&out).expect("json");
        let rendered = info["result"].as_str().expect("rendered drv path");
        rendered.trim_matches('"').to_owned()
    }

    #[test]
    #[serial]
    fn eval_truncates_long_results_with_metadata() {
        let worker = NixWorker::new().expect("worker init");
        let out = worker
            .handle_eval(
                r#"builtins.concatStringsSep "" (builtins.genList (_: "x") 100)"#,
                None,
                10,
            )
            .expect("eval");
        let info: serde_json::Value = serde_json::from_str(&out).expect("json");
        // The rendered string is 100 x's plus the two quotes: 102 chars.
        assert_eq!(info["truncated"], true);
        assert_eq!(info["total_len"], 102);
        assert_eq!(info["result"].as_str().expect("result").chars().count(), 10);
        assert!(
            info["hint"].as_str().expect("hint").contains("max_len"),
            "hint should say how to raise the cap"
        );
    }

    #[test]
    #[serial]
    fn eval_untruncated_results_carry_no_metadata() {
        let worker = NixWorker::new().expect("worker init");
        let out = worker.handle_eval("1 + 1", None, 10_000).expect("eval");
        let info: serde_json::Value = serde_json::from_str(&out).expect("json");
        assert_eq!(info["result"], "2");
        assert!(info.get("truncated").is_none());
        assert!(info.get("hint").is_none());
    }

    #[test]
    #[serial]
    fn derivation_summary_caps_long_values_and_verbose_does_not() {
        let worker = NixWorker::new().expect("worker init");
        let drv = instantiate_fixture_drv(&worker);

        let summary = worker
            .handle_derivation_info(&drv, false)
            .expect("summary derivation info");
        let s: serde_json::Value = serde_json::from_str(&summary).expect("summary json");
        let capped = s["env"]["longEnv"].as_str().expect("longEnv");
        assert!(capped.chars().count() < 600, "value not capped: {capped}");
        assert!(capped.contains("truncated"), "no inline marker: {capped}");
        assert_eq!(s["truncated"], true);
        assert!(
            s["hint"].as_str().expect("hint").contains("verbose"),
            "hint should point at verbose=true"
        );

        let full = worker
            .handle_derivation_info(&drv, true)
            .expect("verbose derivation info");
        let f: serde_json::Value = serde_json::from_str(&full).expect("full json");
        assert_eq!(f["env"]["longEnv"].as_str().expect("longEnv").len(), 1000);
        assert!(f.get("hint").is_none());
    }

    #[test]
    #[serial]
    fn closure_pages_are_flat_sorted_and_gap_free() {
        use crate::tools::{ClosureType, StorePathOperation};

        let worker = NixWorker::new().expect("worker init");
        let drv = instantiate_fixture_drv(&worker);

        let query = |offset| {
            let out = worker
                .handle_store_path_query(
                    Some(&drv),
                    None,
                    StorePathOperation::Closure,
                    ClosureType::Runtime,
                    offset,
                    3,
                    None,
                    false,
                )
                .expect("closure page");
            serde_json::from_str::<serde_json::Value>(&out).expect("json")
        };

        let p1 = query(0);
        assert_eq!(p1["count"], 4);
        assert_eq!(p1["has_more"], true);
        assert_eq!(p1["next_offset"], 3);
        let paths1: Vec<&str> = p1["paths"]
            .as_array()
            .expect("flat array")
            .iter()
            .map(|v| v.as_str().expect("plain path string"))
            .collect();
        assert_eq!(paths1.len(), 3);
        assert!(paths1.iter().all(|p| p.starts_with("/nix/store/")));

        let p2 = query(3);
        assert_eq!(p2["has_more"], false);
        assert_eq!(p2["next_offset"], serde_json::Value::Null);
        let paths2: Vec<&str> = p2["paths"]
            .as_array()
            .expect("flat array")
            .iter()
            .map(|v| v.as_str().expect("plain path string"))
            .collect();
        assert_eq!(paths2.len(), 1);
        // Name-major sort puts the three fixture sources first and the
        // `.drv` itself last.
        assert!(
            paths2[0].ends_with("nix-mcp-test-fixture.drv"),
            "got: {}",
            paths2[0]
        );

        let mut all: Vec<&str> = paths1.iter().chain(paths2.iter()).copied().collect();
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), 4, "pages must be gap-free and non-overlapping");
    }

    #[test]
    #[serial]
    fn closure_filter_narrows_case_insensitively() {
        use crate::tools::{ClosureType, StorePathOperation};

        let worker = NixWorker::new().expect("worker init");
        let drv = instantiate_fixture_drv(&worker);

        let query = |filter: Option<&str>| {
            let out = worker
                .handle_store_path_query(
                    Some(&drv),
                    None,
                    StorePathOperation::Closure,
                    ClosureType::Runtime,
                    0,
                    100,
                    filter,
                    false,
                )
                .expect("filtered closure");
            serde_json::from_str::<serde_json::Value>(&out).expect("json")
        };

        // The three sources are nix-mcp-fixture-*; the drv itself is
        // nix-mcp-test-fixture.drv and does not contain "mcp-fixture".
        let sources = query(Some("mcp-fixture"));
        assert_eq!(sources["count"], 3);
        assert_eq!(sources["filter"], "mcp-fixture");

        let beta = query(Some("FIXTURE-BETA"));
        assert_eq!(beta["count"], 1, "filter must be case-insensitive");
        assert!(
            beta["paths"][0].as_str().expect("path").contains("beta"),
            "got: {}",
            beta["paths"][0]
        );

        let none = query(Some("no-such-entry-anywhere"));
        assert_eq!(none["count"], 0);
        assert_eq!(none["has_more"], false);
        assert_eq!(none["paths"].as_array().expect("array").len(), 0);
    }

    #[test]
    #[serial]
    fn closure_include_names_restores_objects() {
        use crate::tools::{ClosureType, StorePathOperation};

        let worker = NixWorker::new().expect("worker init");
        let drv = instantiate_fixture_drv(&worker);

        let out = worker
            .handle_store_path_query(
                Some(&drv),
                None,
                StorePathOperation::Closure,
                ClosureType::Runtime,
                0,
                1,
                None,
                true,
            )
            .expect("closure with names");
        let info: serde_json::Value = serde_json::from_str(&out).expect("json");
        let entry = &info["paths"][0];
        assert!(
            entry["path"]
                .as_str()
                .expect("path")
                .starts_with("/nix/store/")
        );
        let name = entry["name"].as_str().expect("name");
        assert!(entry["path"].as_str().expect("path").ends_with(name));
    }

    /// Add a small, reference-free byte fixture to the store and return its
    /// full printed path. Content-addressed, so the path is stable across
    /// runs; no network, no build, no references.
    fn add_bytes_fixture(worker: &NixWorker) -> String {
        let sp = worker
            .store
            .add_bytes_to_store("nix-mcp-bytes-fixture", b"nix-mcp byte fixture v1")
            .expect("add bytes to the store");
        worker
            .store
            .print_path(&sp)
            .expect("print the fixture path")
    }

    #[test]
    #[serial]
    fn store_path_query_missing_arguments_are_prescriptive() {
        use crate::tools::{ClosureType, StorePathOperation};

        let worker = NixWorker::new().expect("worker init");
        let query = |path: Option<&str>, hash: Option<&str>, operation| {
            worker
                .handle_store_path_query(
                    path,
                    hash,
                    operation,
                    ClosureType::Runtime,
                    0,
                    100,
                    None,
                    false,
                )
                .expect_err("a missing required argument must be an error")
                .to_string()
        };

        let validate = query(None, None, StorePathOperation::Validate);
        assert!(validate.contains("Invalid argument"), "got: {validate}");
        assert!(validate.contains("operation=validate"), "got: {validate}");
        assert!(
            validate.contains("'path'") && validate.contains("'hash'"),
            "validate must name both accepted arguments, got: {validate}"
        );

        let real_path = query(None, None, StorePathOperation::RealPath);
        assert!(
            real_path.contains("operation=real_path requires 'path'"),
            "got: {real_path}"
        );
        assert!(real_path.contains("/nix/store/"), "got: {real_path}");

        // A hash does not satisfy closure; the coupling is per operation.
        let closure = query(
            None,
            Some("00000000000000000000000000000000"),
            StorePathOperation::Closure,
        );
        assert!(
            closure.contains("operation=closure requires 'path'"),
            "got: {closure}"
        );

        // And a path does not satisfy resolve_hash.
        let resolve = query(
            Some("/nix/store/w816gk8hp9sbghl81rb11qvz3v563zn2-hello"),
            None,
            StorePathOperation::ResolveHash,
        );
        assert!(
            resolve.contains("operation=resolve_hash requires 'hash'"),
            "got: {resolve}"
        );
        assert!(resolve.contains("32-character"), "got: {resolve}");
    }

    #[test]
    #[serial]
    fn store_path_round_trips_across_validate_real_path_and_resolve_hash() {
        use crate::tools::{ClosureType, StorePathOperation};

        let worker = NixWorker::new().expect("worker init");
        let path = add_bytes_fixture(&worker);

        let query = |path: Option<&str>, hash: Option<&str>, operation| {
            let out = worker
                .handle_store_path_query(
                    path,
                    hash,
                    operation,
                    ClosureType::Runtime,
                    0,
                    100,
                    None,
                    false,
                )
                .expect("query succeeds");
            serde_json::from_str::<serde_json::Value>(&out).expect("json")
        };

        let validated = query(Some(&path), None, StorePathOperation::Validate);
        assert_eq!(validated["valid"], true);
        assert_eq!(validated["path"], path.as_str());
        assert_eq!(validated["name"], "nix-mcp-bytes-fixture");

        let real = query(Some(&path), None, StorePathOperation::RealPath);
        assert_eq!(real["path"], path.as_str());
        let real_path = real["real_path"].as_str().expect("real_path");
        assert!(
            std::path::Path::new(real_path).exists(),
            "real path must exist on disk: {real_path}"
        );

        // The hash part is the 32 base-32 chars the basename starts with.
        let basename = path.rsplit('/').next().expect("basename");
        let hash = &basename[..32];

        let resolved = query(None, Some(hash), StorePathOperation::ResolveHash);
        assert_eq!(resolved["found"], true);
        assert_eq!(resolved["path"], path.as_str());
        assert_eq!(resolved["name"], "nix-mcp-bytes-fixture");

        // The validate-via-hash arm resolves to the same path.
        let validated_by_hash = query(None, Some(hash), StorePathOperation::Validate);
        assert_eq!(validated_by_hash["valid"], true);
        assert_eq!(validated_by_hash["path"], path.as_str());
    }

    #[test]
    #[serial]
    fn resolve_hash_miss_is_a_soft_result() {
        use crate::tools::{ClosureType, StorePathOperation};

        let worker = NixWorker::new().expect("worker init");
        let out = worker
            .handle_store_path_query(
                None,
                Some("00000000000000000000000000000000"),
                StorePathOperation::ResolveHash,
                ClosureType::Runtime,
                0,
                100,
                None,
                false,
            )
            .expect("a miss is an answer, not an error");
        let miss: serde_json::Value = serde_json::from_str(&out).expect("json");
        assert_eq!(miss["found"], false);
        assert!(miss.get("path").is_none(), "a miss carries no path");
    }

    #[test]
    #[serial]
    fn closure_types_diverge_on_the_fixture() {
        use crate::tools::{ClosureType, StorePathOperation};

        let worker = NixWorker::new().expect("worker init");
        let drv = instantiate_fixture_drv(&worker);

        let query = |path: &str, closure_type| {
            let out = worker
                .handle_store_path_query(
                    Some(path),
                    None,
                    StorePathOperation::Closure,
                    closure_type,
                    0,
                    500,
                    None,
                    false,
                )
                .expect("closure query");
            serde_json::from_str::<serde_json::Value>(&out).expect("json")
        };

        let runtime_drv = query(&drv, ClosureType::Runtime);
        let source = runtime_drv["paths"]
            .as_array()
            .expect("paths")
            .iter()
            .map(|v| v.as_str().expect("path"))
            .find(|p| p.contains("nix-mcp-fixture-alpha"))
            .expect("fixture source in the drv closure")
            .to_owned();

        // A toFile source has no references: its runtime closure is itself.
        let runtime_src = query(&source, ClosureType::Runtime);
        assert_eq!(runtime_src["count"], 1);
        assert_eq!(runtime_src["paths"][0], source.as_str());

        // The reverse closure of the source contains the drv referring to it.
        let referrers = query(&source, ClosureType::Referrers);
        let referrer_paths: Vec<&str> = referrers["paths"]
            .as_array()
            .expect("paths")
            .iter()
            .map(|v| v.as_str().expect("path"))
            .collect();
        assert!(
            referrer_paths
                .iter()
                .any(|p| p.ends_with("nix-mcp-test-fixture.drv")),
            "drv missing from referrers: {referrer_paths:?}"
        );

        // The build closure of the .drv covers at least its runtime closure.
        // Loose on purpose: unbuilt-output behavior is a Nix-version detail.
        let build = query(&drv, ClosureType::Build);
        let build_count = build["count"].as_u64().expect("count");
        let runtime_count = runtime_drv["count"].as_u64().expect("count");
        assert!(
            build_count >= runtime_count,
            "build closure ({build_count}) smaller than runtime ({runtime_count})"
        );
    }

    #[test]
    #[serial]
    fn derivation_info_rejects_a_garbage_drv_path() {
        let worker = NixWorker::new().expect("worker init");
        // Passes the cheap .drv suffix pre-check but names no store entry.
        worker
            .handle_derivation_info(
                "/nix/store/00000000000000000000000000000000-nope.drv",
                false,
            )
            .expect_err("a nonexistent .drv must surface a Nix error");
    }

    #[test]
    #[serial]
    fn flake_resolve_resolves_a_local_fixture_offline() {
        let worker = NixWorker::new().expect("worker init");
        let fixture = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/trivial-flake");
        let url = format!("path:{fixture}");

        let out = worker
            .handle_flake_resolve(&url)
            .expect("resolve the fixture flake");
        let info: serde_json::Value = serde_json::from_str(&out).expect("json");
        assert_eq!(info["url"], url.as_str());
        let outputs: Vec<&str> = info["outputs"]
            .as_array()
            .expect("outputs")
            .iter()
            .map(|v| v.as_str().expect("output name"))
            .collect();
        assert!(outputs.contains(&"greeting"), "got: {outputs:?}");
        assert!(outputs.contains(&"fixtureVersion"), "got: {outputs:?}");

        // LockMode::Virtual keeps the lock in memory: resolving must never
        // write a flake.lock into the fixture directory.
        assert!(
            !std::path::Path::new(fixture).join("flake.lock").exists(),
            "flake.lock leaked to disk"
        );
    }

    #[test]
    #[serial]
    fn flake_resolve_rejects_a_nonexistent_path_and_a_malformed_ref() {
        let worker = NixWorker::new().expect("worker init");
        worker
            .handle_flake_resolve("path:/nix-mcp-tests/no/such/flake/anywhere")
            .expect_err("a nonexistent flake path must error");
        worker
            .handle_flake_resolve("::::")
            .expect_err("a malformed flake ref must error");
    }

    #[test]
    #[serial]
    fn realize_rejects_a_plain_output_path_prescriptively() {
        let worker = NixWorker::new().expect("worker init");
        let path = add_bytes_fixture(&worker);

        // Even a *valid* non-drv path cannot be realized: the C API's realise
        // parses its argument as a derivation. The pre-check turns Nix's
        // parse failure into a course-correcting error. If this ever starts
        // succeeding, the binding learned to fetch output paths: update the
        // tool description and drop the pre-check.
        let err = worker
            .handle_realize(&path)
            .expect_err("a plain output path must be rejected")
            .to_string();
        assert!(err.contains("Invalid argument"), "got: {err}");
        assert!(err.contains(".drv"), "got: {err}");
        assert!(err.contains("operation=validate"), "got: {err}");
    }

    #[test]
    #[serial]
    fn realize_errors_on_a_garbage_drv_path() {
        let worker = NixWorker::new().expect("worker init");
        // Passes the .drv pre-check but names no store entry.
        worker
            .handle_realize("/nix/store/00000000000000000000000000000000-nix-mcp-garbage.drv")
            .expect_err("a nonexistent, unsubstitutable .drv cannot be realized");
    }

    #[test]
    #[serial]
    fn spawn_signals_ready_answers_ping_and_exits_on_channel_close() {
        let (tx, rx) = flume::unbounded();
        let (ready_tx, ready_rx) = flume::bounded(1);
        let handle = NixWorker::spawn(rx, ready_tx);

        ready_rx
            .recv_timeout(std::time::Duration::from_secs(60))
            .expect("ready signal arrives")
            .expect("worker init succeeds");

        let (reply_tx, reply_rx) = flume::bounded(1);
        tx.send(NixOp::Ping(reply_tx)).expect("send ping");
        let pong = reply_rx
            .recv_timeout(std::time::Duration::from_secs(60))
            .expect("ping reply arrives")
            .expect("ping succeeds");
        assert_eq!(pong, r#"{"pong":true}"#);

        // Closing the op channel must end the run loop and the thread.
        drop(tx);
        handle.join().expect("worker exits cleanly");
    }

    /// The `path:` flake ref of the search fixture: fake `legacyPackages`
    /// and a fake `lib.nixosSystem`, so pinned search runs offline.
    fn search_fixture_ref() -> String {
        format!(
            "path:{}",
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/search-flake")
        )
    }

    #[test]
    #[serial]
    fn pinned_package_search_matches_case_insensitively_and_pages() {
        use crate::tools::SearchOperation;

        let worker = NixWorker::new().expect("worker init");
        let flake = search_fixture_ref();

        // Case differs from the fixture's attr names on purpose.
        let out = worker
            .handle_search_pinned(SearchOperation::Package, "RipGrep", &flake, 0, 1)
            .expect("pinned package search");
        let p1: serde_json::Value = serde_json::from_str(&out).expect("json");
        assert_eq!(p1["source"], "pinned");
        assert_eq!(p1["flake"], flake.as_str());
        assert_eq!(p1["count"], 2);
        assert_eq!(p1["has_more"], true);
        assert_eq!(p1["next_offset"], 1);
        assert_eq!(p1["results"][0]["name"], "ripgrep");
        assert_eq!(p1["results"][0]["version"], "99.0");
        assert_eq!(p1["results"][0]["description"], "fixture ripgrep");

        let out = worker
            .handle_search_pinned(SearchOperation::Package, "RipGrep", &flake, 1, 50)
            .expect("second page");
        let p2: serde_json::Value = serde_json::from_str(&out).expect("json");
        assert_eq!(p2["results"][0]["name"], "ripgrep-all");
        assert_eq!(p2["has_more"], false);
        assert_eq!(p2["next_offset"], serde_json::Value::Null);
    }

    #[test]
    #[serial]
    fn pinned_package_search_degrades_a_throwing_package_to_a_name_hit() {
        use crate::tools::SearchOperation;

        let worker = NixWorker::new().expect("worker init");
        let out = worker
            .handle_search_pinned(
                SearchOperation::Package,
                "broken",
                &search_fixture_ref(),
                0,
                10,
            )
            .expect("a throwing package must not fail the query");
        let info: serde_json::Value = serde_json::from_str(&out).expect("json");
        assert_eq!(info["count"], 1);
        assert_eq!(info["results"][0]["name"], "broken");
        assert!(
            info["results"][0].get("version").is_none(),
            "a failed detail eval must degrade to a name-only hit"
        );
    }

    #[test]
    #[serial]
    fn pinned_option_search_walks_the_tree_and_renders_details() {
        use crate::tools::SearchOperation;

        let worker = NixWorker::new().expect("worker init");
        let flake = search_fixture_ref();

        // The fixture's `_module` throws if touched; finding both foo
        // options proves the walker skipped it and recursed the rest.
        let out = worker
            .handle_search_pinned(SearchOperation::Option, "foo", &flake, 0, 10)
            .expect("pinned option search");
        let info: serde_json::Value = serde_json::from_str(&out).expect("json");
        assert_eq!(info["count"], 2);
        let enable = &info["results"][0];
        assert_eq!(enable["name"], "services.foo.enable");
        assert_eq!(enable["type"], "boolean");
        assert_eq!(enable["description"], "Whether to enable foo.");
        assert_eq!(enable["default"], "false", "defaultText wins");
        assert_eq!(enable["example"], "true");
        let port = &info["results"][1];
        assert_eq!(port["name"], "services.foo.port");
        assert_eq!(port["default"], "8080", "scalar default is rendered");

        // Case-insensitive on the dotted path; string default keeps quotes.
        let out = worker
            .handle_search_pinned(SearchOperation::Option, "HOSTNAME", &flake, 0, 10)
            .expect("case-insensitive option search");
        let info: serde_json::Value = serde_json::from_str(&out).expect("json");
        assert_eq!(info["count"], 1);
        assert_eq!(info["results"][0]["name"], "networking.hostName");
        assert_eq!(info["results"][0]["default"], "\"nixos\"");
    }

    #[test]
    #[serial]
    fn pinned_search_rejects_empty_arguments_prescriptively() {
        use crate::tools::SearchOperation;

        let worker = NixWorker::new().expect("worker init");
        let err = worker
            .handle_search_pinned(SearchOperation::Package, "  ", "nixpkgs", 0, 10)
            .expect_err("empty query is rejected")
            .to_string();
        assert!(err.contains("query is empty"), "got: {err}");
        assert!(err.contains("ripgrep"), "must show an example, got: {err}");

        let err = worker
            .handle_search_pinned(SearchOperation::Package, "ripgrep", "", 0, 10)
            .expect_err("empty flake is rejected")
            .to_string();
        assert!(err.contains("flake is empty"), "got: {err}");
        assert!(err.contains("nixpkgs"), "must show an example, got: {err}");
    }

    #[test]
    #[serial]
    fn pinned_search_surfaces_a_missing_flake_as_a_nix_error() {
        use crate::tools::SearchOperation;

        let worker = NixWorker::new().expect("worker init");
        worker
            .handle_search_pinned(
                SearchOperation::Package,
                "ripgrep",
                "path:/nix-mcp-tests/no/such/flake",
                0,
                10,
            )
            .expect_err("a missing flake must surface a Nix error");
    }
}

//! Serde/`schemars` parameter structs backing each MCP tool's input schema.
//!
//! Every struct here maps to one `#[tool]` method in [`crate::main`] and
//! generates that tool's JSON input schema via `schemars`.

use schemars::JsonSchema;
use serde::Deserialize;

/// Which dependency graph `nix_store_path_query`'s `closure` operation walks.
#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClosureType {
    /// Runtime dependencies only: what the path needs to run.
    #[default]
    Runtime,
    /// Build closure: also includes derivers (`.drv`) and their outputs.
    Build,
    /// Reverse closure: paths that depend on this one ("what pulls this in").
    Referrers,
}

impl ClosureType {
    /// The `(flip_direction, include_outputs, include_derivers)` flags to pass
    /// to [`nix_bindings::Store::collect_fs_closure`] for this closure type.
    #[must_use]
    pub const fn flags(self) -> (bool, bool, bool) {
        match self {
            Self::Runtime => (false, false, false),
            Self::Build => (false, true, true),
            Self::Referrers => (true, false, false),
        }
    }

    /// A stable string identifier, echoed back in query results.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Runtime => "runtime",
            Self::Build => "build",
            Self::Referrers => "referrers",
        }
    }
}

/// The query `nix_store_path_query` should perform.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StorePathOperation {
    /// Check whether a path (or hash part) is a valid store path.
    Validate,
    /// Resolve a store path to its real filesystem location.
    RealPath,
    /// Walk the closure of a store path (see [`ClosureType`]).
    Closure,
    /// Resolve a hash part to its full store path.
    ResolveHash,
}

/// What `nix_search` searches for.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SearchOperation {
    /// nixpkgs packages.
    Package,
    /// NixOS module options.
    Option,
}

impl SearchOperation {
    /// A stable string identifier, echoed back in search results.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Package => "package",
            Self::Option => "option",
        }
    }
}

/// Which backend `nix_search` queries.
#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SearchSource {
    /// search.nixos.org: ranked, channel-fresh, needs network.
    #[default]
    Web,
    /// Evaluate a pinned nixpkgs flake in-process: offline, name-match only.
    Pinned,
}

/// A `[start, end)` window into a `count`-length result set, plus the paging
/// metadata returned alongside a paginated `closure` query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Page {
    /// First index of the window (clamped to `count`).
    pub start: usize,
    /// One past the last index of the window (clamped to `count`).
    pub end: usize,
    /// Whether items remain after this window.
    pub has_more: bool,
    /// The `offset` to request the next window, when `has_more`.
    pub next_offset: Option<usize>,
}

impl Page {
    /// Compute the window for `offset`/`limit` over a result of `count` items.
    ///
    /// `start <= end <= count` always holds, so the window is a valid slice
    /// range even when `offset` runs past the end. `limit` must be at least 1
    /// (callers clamp it); this keeps `next_offset` strictly greater than the
    /// current `offset`, so a paging caller always makes progress.
    #[must_use]
    pub fn new(count: usize, offset: usize, limit: usize) -> Self {
        let start = offset.min(count);
        let end = start.saturating_add(limit).min(count);
        let has_more = end < count;

        Self {
            start,
            end,
            has_more,
            next_offset: has_more.then_some(end),
        }
    }
}

/// Truncate `s` to at most `max_chars` characters (not bytes).
///
/// Returns the possibly-shortened slice and whether truncation happened.
/// Cutting on a `char` boundary keeps the result valid UTF-8, so it can be
/// embedded in a JSON string without corrupting the envelope.
#[must_use]
pub fn truncate_chars(s: &str, max_chars: usize) -> (&str, bool) {
    match s.char_indices().nth(max_chars) {
        Some((byte_index, _)) => (&s[..byte_index], true),
        None => (s, false),
    }
}

/// Parameters for `nix_eval`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct EvalParams {
    /// The Nix expression to evaluate. Required in the schema so a missing
    /// expression is rejected before any Nix work instead of dying in the
    /// parser as "unexpected end of file".
    #[schemars(description = "The Nix expression to evaluate, e.g. builtins.nixVersion")]
    pub expression: String,

    /// Optional attribute to access on the result before rendering it.
    #[schemars(
        description = "Optional single top-level attribute name to access on the result (not a dotted path)"
    )]
    pub attr: Option<String>,

    /// Cap, in characters, on the rendered result (defaults to 10000, capped
    /// server-side at 100000).
    #[schemars(
        range(min = 1, max = 100_000),
        description = "Max characters of the rendered result (default 10000, cap 100000); longer results are cut and carry {truncated, total_len, hint}"
    )]
    pub max_len: Option<u32>,
}

/// Parameters for `nix_store_path_query`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct StorePathQueryParams {
    /// The store path to query (required for `real_path` and `closure`).
    #[schemars(
        description = "Full store path, e.g. /nix/store/<hash>-<name> (required for real_path and closure; either path or hash for validate)"
    )]
    pub path: Option<String>,

    /// A hash part to look up (for `validate` and `resolve_hash`).
    #[schemars(
        description = "Store path hash part: the 32-character base-32 prefix of the basename, e.g. w816gk8hp9sbghl81rb11qvz3v563zn2 (required for resolve_hash; either path or hash for validate)"
    )]
    pub hash: Option<String>,

    /// Which query to run.
    #[schemars(description = "Operation: validate, real_path, closure, or resolve_hash")]
    pub operation: StorePathOperation,

    /// `closure` only: which dependency graph to walk (defaults to `runtime`).
    #[schemars(description = "closure only: runtime (default), build, or referrers")]
    pub closure_type: Option<ClosureType>,

    /// `closure` only: page offset into the name-sorted result (defaults to 0).
    #[schemars(description = "closure only: page offset into the sorted paths")]
    pub offset: Option<u32>,

    /// `closure` only: page size, capped server-side (defaults to 100).
    #[schemars(
        range(min = 1, max = 500),
        description = "closure only: max paths to return per page (default 100, capped at 500)"
    )]
    pub limit: Option<u32>,

    /// `closure` only: case-insensitive substring filter on entry names.
    #[schemars(
        description = "closure only: case-insensitive substring match on the entry name (the part after the store hash); count and pages reflect only matches. Use this instead of paging the whole closure to answer \"does X depend on Y\""
    )]
    pub filter: Option<String>,

    /// `closure` only: emit `{path, name}` objects instead of plain path
    /// strings (defaults to false; the name is derivable from the path).
    #[schemars(
        description = "closure only: return {path,name} objects instead of plain path strings (default false; name is the path minus /nix/store/<hash>- and costs extra tokens per entry)"
    )]
    pub include_names: Option<bool>,
}

/// Parameters for `nix_realize`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct RealizeParams {
    /// The `.drv` store path to realize.
    #[schemars(
        description = "The .drv store path to build (e.g. /nix/store/xxx-name.drv); plain output paths cannot be realized"
    )]
    pub path: String,
}

/// Parameters for `nix_flake_resolve`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct FlakeResolveParams {
    /// The flake reference to resolve and lock.
    #[schemars(description = "Flake URL to resolve (e.g. github:NixOS/nixpkgs)")]
    pub url: String,
}

/// Parameters for `nix_derivation_info`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct DerivationInfoParams {
    /// The `.drv` store path to read.
    #[schemars(description = "Store path of the .drv file")]
    pub path: String,

    /// Return the full derivation JSON instead of the capped summary.
    #[schemars(
        description = "Return the complete derivation JSON including full env/build scripts (can be tens of thousands of tokens); default is a summary with long values truncated to 500 chars"
    )]
    pub verbose: Option<bool>,
}

/// What `nix_config` does.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConfigOperation {
    /// Read the current value of one setting.
    Get,
    /// List all known settings whose key contains the query substring
    /// (or all settings when `query` is omitted), with descriptions and
    /// current values.
    Show,
    /// List setting keys only (optionally filtered by a substring query).
    List,
}

impl ConfigOperation {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "get",
            Self::Show => "show",
            Self::List => "list",
        }
    }
}

/// Parameters for `nix_config`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ConfigParams {
    /// Which operation to perform.
    #[schemars(
        description = "get: read one setting by key. show: list settings with descriptions and current values (filtered by query substring, or all). list: list setting keys only (filtered by query substring, or all)"
    )]
    pub operation: ConfigOperation,

    /// The setting key (required for `get`) or a case-insensitive substring
    /// filter (optional for `show` and `list`).
    #[schemars(
        description = "For get: the setting key (e.g. connect-timeout). For show/list: optional case-insensitive substring to filter keys (e.g. timeout)"
    )]
    pub query: Option<String>,
}

/// Parameters for `nix_search`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SearchParams {
    /// The search text.
    #[schemars(
        description = "Search text: a package name fragment (e.g. ripgrep) or option name fragment (e.g. services.nginx)"
    )]
    pub query: String,

    /// What to search for.
    #[schemars(
        description = "What to search: package (nixpkgs packages) or option (NixOS module options)"
    )]
    pub operation: SearchOperation,

    /// Which backend to query (defaults to `web`).
    #[schemars(
        description = "web (default): search.nixos.org, ranked and channel-fresh, needs network. pinned: evaluate a nixpkgs flake locally, offline, matches names only"
    )]
    pub source: Option<SearchSource>,

    /// `web` only: the channel to search (defaults to `unstable`).
    #[schemars(
        description = "web only: channel to search, 'unstable' (default) or a release like '25.11'"
    )]
    pub channel: Option<String>,

    /// `pinned` only: the flake reference to evaluate (defaults to `nixpkgs`).
    #[schemars(
        description = "pinned only: flake ref to evaluate (default 'nixpkgs' via the flake registry; e.g. github:NixOS/nixpkgs/<rev>)"
    )]
    pub flake: Option<String>,

    /// Page offset into the results (defaults to 0).
    #[schemars(description = "page offset into the ranked (web) or name-sorted (pinned) results")]
    pub offset: Option<u32>,

    /// Page size, capped server-side (defaults to 20).
    #[schemars(
        range(min = 1, max = 100),
        description = "max results per page (default 20, capped at 100)"
    )]
    pub limit: Option<u32>,
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::{
        ClosureType, ConfigOperation, ConfigParams, EvalParams, Page, SearchOperation,
        SearchParams, SearchSource, StorePathOperation, StorePathQueryParams, truncate_chars,
    };

    #[test]
    fn truncate_chars_leaves_short_strings_alone() {
        assert_eq!(truncate_chars("hello", 10), ("hello", false));
        assert_eq!(truncate_chars("hello", 5), ("hello", false));
        assert_eq!(truncate_chars("", 0), ("", false));
    }

    #[test]
    fn truncate_chars_cuts_long_strings() {
        assert_eq!(truncate_chars("hello", 4), ("hell", true));
        assert_eq!(truncate_chars("hello", 0), ("", true));
    }

    #[test]
    fn truncate_chars_counts_characters_not_bytes() {
        // Each snowman is 3 bytes; cutting at 2 chars must not split one.
        assert_eq!(
            truncate_chars("\u{2603}\u{2603}\u{2603}", 2),
            ("\u{2603}\u{2603}", true)
        );
    }

    #[test]
    fn closure_type_flags_pick_the_right_graph() {
        assert_eq!(ClosureType::Runtime.flags(), (false, false, false));
        assert_eq!(ClosureType::Build.flags(), (false, true, true));
        assert_eq!(ClosureType::Referrers.flags(), (true, false, false));
    }

    #[test]
    fn closure_type_defaults_to_runtime() {
        assert_eq!(ClosureType::default().as_str(), "runtime");
    }

    #[test]
    fn closure_type_deserializes_from_snake_case() {
        let parse = |s| serde_json::from_str::<ClosureType>(s).expect("valid closure_type");
        assert_eq!(parse("\"runtime\"").as_str(), "runtime");
        assert_eq!(parse("\"build\"").as_str(), "build");
        assert_eq!(parse("\"referrers\"").as_str(), "referrers");
        serde_json::from_str::<ClosureType>("\"bogus\"")
            .expect_err("unknown closure_type rejected");
    }

    #[test]
    fn store_path_operation_uses_underscored_names() {
        // The public API spells these with underscores; confirm serde matches.
        for name in [
            "\"validate\"",
            "\"real_path\"",
            "\"closure\"",
            "\"resolve_hash\"",
        ] {
            serde_json::from_str::<StorePathOperation>(name).expect("underscored name parses");
        }
        serde_json::from_str::<StorePathOperation>("\"realpath\"")
            .expect_err("name without an underscore is rejected");
    }

    #[test]
    fn store_path_query_params_leave_closure_fields_absent() {
        let p: StorePathQueryParams =
            serde_json::from_str(r#"{"operation":"closure","path":"/nix/store/x"}"#)
                .expect("valid params");
        assert!(matches!(p.operation, StorePathOperation::Closure));
        assert_eq!(p.path.as_deref(), Some("/nix/store/x"));
        assert!(p.closure_type.is_none());
        assert!(p.offset.is_none());
        assert!(p.limit.is_none());
    }

    #[test]
    fn store_path_query_params_require_an_operation() {
        serde_json::from_str::<StorePathQueryParams>(r#"{"path":"/nix/store/x"}"#)
            .expect_err("operation is required");
    }

    #[test]
    fn eval_params_require_an_expression() {
        serde_json::from_str::<EvalParams>("{}").expect_err("expression is required");
        serde_json::from_str::<EvalParams>(r#"{"attr":"x"}"#)
            .expect_err("attr alone is not enough");

        let p: EvalParams =
            serde_json::from_str(r#"{"expression":"1 + 1"}"#).expect("valid eval params");
        assert_eq!(p.expression, "1 + 1");
        assert!(p.attr.is_none());
    }

    #[test]
    fn search_params_require_query_and_operation() {
        serde_json::from_str::<SearchParams>(r#"{"query":"ripgrep"}"#)
            .expect_err("operation is required");
        serde_json::from_str::<SearchParams>(r#"{"operation":"package"}"#)
            .expect_err("query is required");

        let p: SearchParams = serde_json::from_str(r#"{"query":"ripgrep","operation":"package"}"#)
            .expect("valid search params");
        assert_eq!(p.query, "ripgrep");
        assert!(matches!(p.operation, SearchOperation::Package));
        assert!(p.source.is_none());
        assert!(p.channel.is_none());
        assert!(p.flake.is_none());
    }

    #[test]
    fn search_enums_use_snake_case_and_reject_unknowns() {
        let op = |s| serde_json::from_str::<SearchOperation>(s).expect("valid operation");
        assert_eq!(op("\"package\"").as_str(), "package");
        assert_eq!(op("\"option\"").as_str(), "option");
        serde_json::from_str::<SearchOperation>("\"options\"").expect_err("plural form rejected");

        assert!(matches!(
            serde_json::from_str::<SearchSource>("\"web\"").expect("web"),
            SearchSource::Web
        ));
        assert!(matches!(
            serde_json::from_str::<SearchSource>("\"pinned\"").expect("pinned"),
            SearchSource::Pinned
        ));
        serde_json::from_str::<SearchSource>("\"local\"").expect_err("unknown source rejected");
    }

    #[test]
    fn search_source_defaults_to_web() {
        assert!(matches!(SearchSource::default(), SearchSource::Web));
    }

    #[test]
    fn page_first_of_several() {
        let page = Page::new(10, 0, 3);
        assert_eq!((page.start, page.end), (0, 3));
        assert!(page.has_more);
        assert_eq!(page.next_offset, Some(3));
    }

    #[test]
    fn page_last_partial_window_is_terminal() {
        let page = Page::new(10, 9, 3);
        assert_eq!((page.start, page.end), (9, 10));
        assert!(!page.has_more);
        assert_eq!(page.next_offset, None);
    }

    #[test]
    fn page_offset_at_or_past_end_is_empty_and_terminal() {
        for offset in [10, 25] {
            let page = Page::new(10, offset, 3);
            assert_eq!((page.start, page.end), (10, 10));
            assert!(!page.has_more);
            assert_eq!(page.next_offset, None);
        }
    }

    #[test]
    fn page_limit_covering_everything_reports_no_more() {
        let page = Page::new(5, 0, 100);
        assert_eq!((page.start, page.end), (0, 5));
        assert!(!page.has_more);
        assert_eq!(page.next_offset, None);
    }

    #[test]
    fn page_over_empty_result_is_terminal() {
        let page = Page::new(0, 0, 100);
        assert_eq!((page.start, page.end), (0, 0));
        assert!(!page.has_more);
        assert_eq!(page.next_offset, None);
    }

    #[test]
    fn page_next_offset_strictly_advances() {
        let page = Page::new(100, 40, 20);
        assert_eq!(page.next_offset, Some(60));
    }

    proptest! {
        /// The window is always a valid, in-bounds slice range no larger than
        /// `limit`, and `next_offset` is `Some(end)` exactly when more remains.
        #[test]
        fn page_window_invariants(
            count in 0usize..10_000,
            offset in 0usize..12_000,
            limit in 1usize..1_000,
        ) {
            let page = Page::new(count, offset, limit);
            prop_assert!(page.start <= page.end);
            prop_assert!(page.end <= count);
            prop_assert!(page.end - page.start <= limit);
            prop_assert_eq!(page.has_more, page.end < count);
            match page.next_offset {
                Some(next) => {
                    prop_assert!(page.has_more);
                    prop_assert_eq!(next, page.end);
                    prop_assert!(next > page.start);
                }
                None => prop_assert!(!page.has_more),
            }
        }

        /// Walking pages from offset 0 via `next_offset` visits every item in
        /// `[0, count)` exactly once and always terminates.
        #[test]
        fn page_walk_covers_everything_once(count in 0usize..2_000, limit in 1usize..500) {
            let mut offset = 0usize;
            let mut visited = 0usize;
            loop {
                let page = Page::new(count, offset, limit);
                prop_assert_eq!(page.start, offset);
                visited += page.end - page.start;
                match page.next_offset {
                    Some(next) => {
                        prop_assert!(next > offset);
                        offset = next;
                    }
                    None => break,
                }
            }
            prop_assert_eq!(visited, count);
        }
    }

    #[test]
    fn config_operation_deserializes_from_snake_case() {
        let parse = |s| serde_json::from_str::<ConfigOperation>(s).expect("valid");
        assert_eq!(parse("\"get\"").as_str(), "get");
        assert_eq!(parse("\"show\"").as_str(), "show");
        assert_eq!(parse("\"list\"").as_str(), "list");
        serde_json::from_str::<ConfigOperation>("\"dump\"").expect_err("unknown rejected");
    }

    #[test]
    fn config_params_require_an_operation() {
        serde_json::from_str::<ConfigParams>("{}").expect_err("operation is required");
        serde_json::from_str::<ConfigParams>(r#"{"query":"timeout"}"#)
            .expect_err("query alone is not enough");

        let p: ConfigParams = serde_json::from_str(r#"{"operation":"get","query":"timeout"}"#)
            .expect("valid config params");
        assert!(matches!(p.operation, ConfigOperation::Get));
        assert_eq!(p.query.as_deref(), Some("timeout"));
    }

    #[test]
    fn config_params_query_is_optional_for_show_and_list() {
        let p: ConfigParams =
            serde_json::from_str(r#"{"operation":"show"}"#).expect("valid config params");
        assert!(matches!(p.operation, ConfigOperation::Show));
        assert!(p.query.is_none());

        let p: ConfigParams =
            serde_json::from_str(r#"{"operation":"list"}"#).expect("valid config params");
        assert!(matches!(p.operation, ConfigOperation::List));
        assert!(p.query.is_none());
    }
}

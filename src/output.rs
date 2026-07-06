//! Typed result payloads backing each MCP tool's structured output.
//!
//! The worker builds every reply from these structs (the single source of
//! truth for the wire shape), and `lib.rs` declares each tool's
//! `outputSchema` from the same types, so the schema advertised in
//! `tools/list` cannot drift from what the server actually returns. Field
//! doc comments double as the schema `description`s agents read.

use schemars::JsonSchema;
use serde::Serialize;

/// Generate a tool's `outputSchema` from its payload type.
///
/// # Panics
///
/// Panics if the type's schema is not object-rooted (the MCP spec requires
/// an object root for `outputSchema`). This runs once at router
/// construction, so a bad schema fails the server loudly at startup rather
/// than serving an invalid listing.
#[must_use]
pub fn schema<T: JsonSchema + std::any::Any>() -> std::sync::Arc<rmcp::model::JsonObject> {
    rmcp::handler::server::tool::schema_for_output::<T>().unwrap_or_else(|err| {
        panic!(
            "invalid output schema for {}: {err}",
            std::any::type_name::<T>()
        )
    })
}

/// `nix_store_info` result.
#[derive(Debug, Serialize, JsonSchema)]
pub struct StoreInfo {
    /// The store URI, e.g. "daemon".
    pub uri: String,
    /// The store directory, e.g. "/nix/store".
    pub store_dir: String,
    /// The store daemon's version.
    pub version: String,
    /// The linked Nix client library's version.
    pub nix_version: String,
    /// The evaluator's current system, e.g. "x86_64-linux".
    pub system: String,
}

/// `nix_eval` result.
#[derive(Debug, Serialize, JsonSchema)]
pub struct EvalResult {
    /// The value rendered as Nix syntax (strings include their quotes).
    pub result: String,
    /// Present and true only when the rendering was cut at `max_len`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
    /// Total length in characters of the untruncated rendering.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_len: Option<usize>,
    /// How to get the rest of a truncated result.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

/// One `closure` page entry: a plain store path by default, or a
/// `{path, name}` object when `include_names` is set.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum ClosureEntry {
    /// The default shape: a full store path string.
    Path(String),
    /// The `include_names = true` shape.
    Named {
        /// Full store path.
        path: String,
        /// The path's name (the basename minus the hash prefix).
        name: String,
    },
}

/// `nix_store_path_query` result.
///
/// One flat object covering all four operations; each field notes the
/// operation(s) that produce it. A per-operation `anyOf` would be more
/// precise, but the MCP `outputSchema` root must be a plain object; the
/// split is backlogged together with the tagged-enum input redesign.
#[derive(Debug, Default, Serialize, JsonSchema)]
pub struct StorePathQueryResult {
    /// `validate`: whether the path is a valid (present) store path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid: Option<bool>,
    /// `resolve_hash`, and `validate` by hash: whether the hash part
    /// resolved to a store path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub found: Option<bool>,
    /// `validate` / `real_path` / `resolve_hash`: the full store path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// `validate` / `resolve_hash`: the path's name part.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// `real_path`: the resolved filesystem location.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub real_path: Option<String>,
    /// `closure`: the queried root path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    /// `closure`: which graph was walked (runtime, build, or referrers).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closure_type: Option<String>,
    /// `closure`: total matching entries (after `filter`, before paging).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<usize>,
    /// `closure`: the requested page offset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<usize>,
    /// `closure`: the effective page size.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    /// `closure`: whether entries remain after this page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_more: Option<bool>,
    /// `closure`: the offset of the next page; null on the last page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_offset: Option<Option<usize>>,
    /// `closure`: the name filter, echoed back when one was given.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<String>,
    /// `closure`: one page of entries.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paths: Option<Vec<ClosureEntry>>,
}

/// One `nix_search` hit. Package and option hits share the shape; each
/// detail field notes which operation produces it and is omitted when
/// unknown.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SearchItem {
    /// Package attribute name (e.g. "ripgrep") or full option name
    /// (e.g. "services.nginx.enable").
    pub name: String,
    /// `package`: the package version, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// A short description, when the package or option declares one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// `option`: the option's type, e.g. "boolean".
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub option_type: Option<String>,
    /// `option`: the default value's documented rendering.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    /// `option`: the documented example, when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub example: Option<String>,
    /// `package`: programs the package installs (web source only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub programs: Option<Vec<String>>,
}

/// `nix_search` result: one page of hits plus the paging metadata, in the
/// same shape the `closure` query uses.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SearchResult {
    /// The search text, echoed back.
    pub query: String,
    /// What was searched: "package" or "option".
    pub operation: String,
    /// Which backend answered: "web" or "pinned".
    pub source: String,
    /// `web`: the normalized channel that was searched.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    /// `pinned`: the flake reference that was evaluated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flake: Option<String>,
    /// Total matches, before paging.
    pub count: usize,
    /// The requested page offset.
    pub offset: usize,
    /// The effective page size.
    pub limit: usize,
    /// Whether matches remain after this page.
    pub has_more: bool,
    /// The offset of the next page; null on the last page.
    pub next_offset: Option<usize>,
    /// One page of hits.
    pub results: Vec<SearchItem>,
}

/// One realized derivation output.
#[derive(Debug, Serialize, JsonSchema)]
pub struct RealizeEntry {
    /// The derivation output name, e.g. "out".
    pub output_name: String,
    /// Full store path of the realized output.
    pub path: String,
    /// The path's name part.
    pub name: String,
}

/// `nix_realize` result.
#[derive(Debug, Serialize, JsonSchema)]
pub struct RealizeResult {
    /// The realized outputs.
    pub outputs: Vec<RealizeEntry>,
    /// Number of realized outputs.
    pub count: usize,
}

/// `nix_flake_resolve` result.
#[derive(Debug, Serialize, JsonSchema)]
pub struct FlakeResolveResult {
    /// The flake reference as given.
    pub url: String,
    /// The flake's top-level output attribute names.
    pub outputs: Vec<String>,
}

/// `nix_derivation_info` result: the derivation's own JSON object.
///
/// The shape comes from Nix itself (name, system, builder, args, env,
/// inputDrvs, inputSrcs, outputs). Summary mode adds `truncated` and `hint`
/// keys when long values were cut.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(transparent)]
pub struct DerivationJson(pub serde_json::Map<String, serde_json::Value>);

#[cfg(test)]
mod tests {
    use super::{
        DerivationJson, EvalResult, FlakeResolveResult, RealizeResult, SearchItem, SearchResult,
        StoreInfo, StorePathQueryResult, schema,
    };

    #[test]
    fn every_output_schema_is_object_rooted() {
        // `schema` panics on a non-object root; constructing each one is the
        // whole assertion, and it mirrors what router construction does.
        for s in [
            schema::<StoreInfo>(),
            schema::<EvalResult>(),
            schema::<StorePathQueryResult>(),
            schema::<RealizeResult>(),
            schema::<FlakeResolveResult>(),
            schema::<DerivationJson>(),
            schema::<SearchResult>(),
        ] {
            assert_eq!(s["type"], "object", "schema root must be an object");
        }
    }

    #[test]
    fn optional_fields_are_omitted_not_null() {
        let eval = serde_json::to_value(EvalResult {
            result: "2".to_owned(),
            truncated: None,
            total_len: None,
            hint: None,
        })
        .expect("serialize");
        assert_eq!(eval, serde_json::json!({ "result": "2" }));

        let miss = serde_json::to_value(StorePathQueryResult {
            valid: Some(false),
            found: Some(false),
            ..StorePathQueryResult::default()
        })
        .expect("serialize");
        assert_eq!(miss, serde_json::json!({ "valid": false, "found": false }));
    }

    #[test]
    fn search_item_omits_absent_detail_fields() {
        let item = serde_json::to_value(SearchItem {
            name: "ripgrep".to_owned(),
            version: None,
            description: None,
            option_type: None,
            default: None,
            example: None,
            programs: None,
        })
        .expect("serialize");
        assert_eq!(item, serde_json::json!({ "name": "ripgrep" }));

        // The option type field serializes under its wire name, "type".
        let item = serde_json::to_value(SearchItem {
            name: "services.nginx.enable".to_owned(),
            version: None,
            description: None,
            option_type: Some("boolean".to_owned()),
            default: None,
            example: None,
            programs: None,
        })
        .expect("serialize");
        assert_eq!(item["type"], "boolean");
        assert!(item.get("option_type").is_none());
    }

    #[test]
    fn terminal_page_serializes_next_offset_as_null() {
        let page = serde_json::to_value(StorePathQueryResult {
            next_offset: Some(None),
            ..StorePathQueryResult::default()
        })
        .expect("serialize");
        assert_eq!(page, serde_json::json!({ "next_offset": null }));
    }
}

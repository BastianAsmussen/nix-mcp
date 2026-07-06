//! Backends for the `nix_search` tool.
//!
//! Two sources, one tool. `web` queries search.nixos.org's Elasticsearch
//! index over HTTP from the async handler (no Nix state involved, so it
//! never touches the worker thread). `pinned` builds a Nix expression that
//! the worker evaluates against a pinned nixpkgs flake: it matches on
//! attribute/option *names* lazily and forces details only for the page of
//! hits, so no full-nixpkgs metadata walk ever happens.
//!
//! Everything here that can be tested without a network or a store is a pure
//! function: the ES request body and response parsing, the alias picker, the
//! Nix expression builder, and the escaping helpers.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::error::{NixMcpError, Result};
use crate::output;
use crate::tools::{Page, SearchOperation, truncate_chars};

/// Base URL of the search.nixos.org Elasticsearch backend.
const ES_BASE: &str = "https://search.nixos.org/backend";

/// The public read-only credentials embedded in the search.nixos.org
/// frontend; every client of this endpoint (the site itself, mcp-nixos, ...)
/// ships them. They grant search access only.
const ES_USER: &str = "aWVSALXpZv";
const ES_PASSWORD: &str = "X8gPHnzL52wFEekuxsfQ9cSh";

/// Cap on the error-body snippet echoed back to the agent.
const ERROR_SNIPPET_MAX_CHARS: usize = 300;

/// Course-correcting suffix for every web-transport failure: name the escape
/// hatch instead of leaving the agent to guess.
const WEB_FALLBACK_HINT: &str =
    "retry, or use source=pinned to search a pinned nixpkgs flake offline";

/// The `web` search backend: an HTTP client plus a per-channel cache of the
/// concrete index alias currently behind `latest-<generation>-nixos-<channel>`.
///
/// The generation number rotates server-side every few weeks (the classic
/// breakage in hardcoded clients), and a wildcard index spans *multiple* live
/// generations at once, duplicating every hit (probe-confirmed). So the alias
/// is resolved once from `/_aliases`, cached, and invalidated when a search
/// against it 404s.
pub struct WebSearch {
    /// The HTTP client (connection pooling, timeout, UA).
    client: reqwest::Client,
    /// channel -> concrete alias, e.g. "unstable" -> "latest-48-nixos-unstable".
    aliases: Mutex<HashMap<String, String>>,
}

impl WebSearch {
    /// Build the backend with a timeout that keeps a dead network from
    /// wedging a tool call.
    ///
    /// # Errors
    ///
    /// Returns [`NixMcpError::Web`] if the TLS backend cannot initialize,
    /// which does not happen in practice.
    pub fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .user_agent(concat!("nix_mcp/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|err| NixMcpError::Web(format!("failed to build the HTTP client: {err}")))?;

        Ok(Self {
            client,
            aliases: Mutex::new(HashMap::new()),
        })
    }

    /// Run one search against search.nixos.org and build the paged result.
    ///
    /// # Errors
    ///
    /// Returns [`NixMcpError::InvalidArgument`] for an empty query or a
    /// malformed channel, and [`NixMcpError::Web`] for transport failures,
    /// non-success responses, and unrecognized response shapes; every web
    /// failure names `source=pinned` as the offline fallback.
    pub async fn search(
        &self,
        operation: SearchOperation,
        query: &str,
        channel: &str,
        offset: usize,
        limit: usize,
    ) -> Result<output::SearchResult> {
        if query.trim().is_empty() {
            return Err(NixMcpError::InvalidArgument(
                "query is empty; pass a name fragment like 'ripgrep' or 'services.nginx'"
                    .to_owned(),
            ));
        }
        let channel = normalize_channel(channel)?;
        let body = es_request_body(operation, query, offset, limit);

        let alias = self.resolve_alias(&channel).await?;
        let response = self.post_search(&alias, &body).await?;

        // A 404 means the cached generation rotated away; re-resolve once.
        let response = if response.status() == reqwest::StatusCode::NOT_FOUND {
            self.invalidate(&channel);
            let alias = self.resolve_alias(&channel).await?;
            self.post_search(&alias, &body).await?
        } else {
            response
        };

        let status = response.status();
        let text = response.text().await.map_err(web_error)?;
        if !status.is_success() {
            // Drop the cached alias so the next call starts clean.
            self.invalidate(&channel);
            let (snippet, _) = truncate_chars(&text, ERROR_SNIPPET_MAX_CHARS);
            return Err(NixMcpError::Web(format!(
                "search.nixos.org answered {status}: {snippet}; {WEB_FALLBACK_HINT}"
            )));
        }

        let (count, results) = parse_es_response(operation, &text)?;
        let page = Page::new(count, offset, limit);

        Ok(output::SearchResult {
            query: query.to_owned(),
            operation: operation.as_str().to_owned(),
            source: "web".to_owned(),
            channel: Some(channel),
            flake: None,
            count,
            offset,
            limit,
            has_more: page.has_more,
            next_offset: page.next_offset,
            results,
        })
    }

    /// POST one `_search` request against the concrete `alias`.
    async fn post_search(
        &self,
        alias: &str,
        body: &serde_json::Value,
    ) -> Result<reqwest::Response> {
        self.client
            .post(format!("{ES_BASE}/{alias}/_search"))
            .basic_auth(ES_USER, Some(ES_PASSWORD))
            .json(body)
            .send()
            .await
            .map_err(web_error)
    }

    /// Resolve (and cache) the concrete index alias for `channel`.
    async fn resolve_alias(&self, channel: &str) -> Result<String> {
        if let Some(alias) = self.aliases.lock().expect("alias cache lock").get(channel) {
            return Ok(alias.clone());
        }

        let text = self
            .client
            .get(format!("{ES_BASE}/_aliases"))
            .basic_auth(ES_USER, Some(ES_PASSWORD))
            .send()
            .await
            .map_err(web_error)?
            .error_for_status()
            .map_err(web_error)?
            .text()
            .await
            .map_err(web_error)?;

        let alias = pick_alias(&text, channel).ok_or_else(|| {
            NixMcpError::InvalidArgument(format!(
                "no search.nixos.org index for channel '{channel}'; valid channels are \
                 'unstable' or a release like '25.11'"
            ))
        })?;

        self.aliases
            .lock()
            .expect("alias cache lock")
            .insert(channel.to_owned(), alias.clone());
        Ok(alias)
    }

    /// Forget the cached alias for `channel` (it 404'd or errored).
    fn invalidate(&self, channel: &str) {
        self.aliases
            .lock()
            .expect("alias cache lock")
            .remove(channel);
    }
}

/// Map a transport failure to a legible, course-correcting web error.
#[expect(
    clippy::needless_pass_by_value,
    reason = "map_err hands over an owned error; taking a reference would force a closure at \
              every call site for no gain"
)]
fn web_error(err: reqwest::Error) -> NixMcpError {
    // reqwest error text already names the URL and the failure class.
    NixMcpError::Web(format!("{err}; {WEB_FALLBACK_HINT}"))
}

/// Normalize a channel name: trim, lowercase, and drop an optional `nixos-`
/// prefix, so `unstable`, `nixos-unstable`, and `NixOS-25.11` all work.
///
/// # Errors
///
/// Returns [`NixMcpError::InvalidArgument`] naming the valid forms if the
/// result is empty or contains anything but `[a-z0-9.]`.
pub fn normalize_channel(channel: &str) -> Result<String> {
    let lowered = channel.trim().to_lowercase();
    let bare = lowered.strip_prefix("nixos-").unwrap_or(&lowered);

    if bare.is_empty()
        || !bare
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.')
    {
        return Err(NixMcpError::InvalidArgument(format!(
            "invalid channel '{channel}'; pass 'unstable' or a release like '25.11'"
        )));
    }

    Ok(bare.to_owned())
}

/// Pick the newest `latest-<generation>-nixos-<channel>` alias out of an
/// Elasticsearch `/_aliases` response body.
#[must_use]
pub fn pick_alias(aliases_body: &str, channel: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(aliases_body).ok()?;
    let suffix = format!("-nixos-{channel}");

    let mut best: Option<(u64, String)> = None;
    for index in value.as_object()?.values() {
        let Some(aliases) = index.get("aliases").and_then(|a| a.as_object()) else {
            continue;
        };
        for name in aliases.keys() {
            let Some(generation) = name
                .strip_prefix("latest-")
                .and_then(|rest| rest.strip_suffix(&suffix))
                .and_then(|g| g.parse::<u64>().ok())
            else {
                continue;
            };
            if best.as_ref().is_none_or(|(g, _)| generation > *g) {
                best = Some((generation, name.clone()));
            }
        }
    }

    best.map(|(_, name)| name)
}

/// Build the Elasticsearch request body for one search page.
///
/// A `dis_max` of an exact term (attribute hits rank first), a contains-style
/// case-insensitive wildcard, and a relevance `multi_match` over the indexed
/// fields; filtered to the document type and trimmed to the `_source` fields
/// the result actually uses (probe-confirmed shapes).
#[must_use]
pub fn es_request_body(
    operation: SearchOperation,
    query: &str,
    offset: usize,
    limit: usize,
) -> serde_json::Value {
    let contains = format!("*{}*", query.to_lowercase());
    let (doc_type, source_fields, queries) = match operation {
        SearchOperation::Package => (
            "package",
            serde_json::json!([
                "package_attr_name",
                "package_pname",
                "package_pversion",
                "package_description",
                "package_programs"
            ]),
            serde_json::json!([
                { "term": { "package_attr_name": { "value": query, "boost": 10.0 } } },
                { "wildcard": { "package_attr_name": {
                    "value": contains, "case_insensitive": true, "boost": 4.0 } } },
                { "multi_match": { "query": query, "fields": [
                    "package_attr_name^9",
                    "package_pname^7",
                    "package_programs^6",
                    "package_description^2"
                ] } }
            ]),
        ),
        SearchOperation::Option => (
            "option",
            serde_json::json!([
                "option_name",
                "option_type",
                "option_description",
                "option_default",
                "option_example"
            ]),
            serde_json::json!([
                { "term": { "option_name": { "value": query, "boost": 10.0 } } },
                { "wildcard": { "option_name": {
                    "value": contains, "case_insensitive": true, "boost": 4.0 } } },
                { "multi_match": { "query": query, "fields": [
                    "option_name^8",
                    "option_description^2"
                ] } }
            ]),
        ),
    };

    serde_json::json!({
        "from": offset,
        "size": limit,
        "_source": source_fields,
        "query": { "bool": {
            "filter": [ { "term": { "type": { "value": doc_type } } } ],
            "must": [ { "dis_max": { "queries": queries } } ]
        } }
    })
}

/// The slice of an Elasticsearch `_search` response the tool consumes.
#[derive(serde::Deserialize)]
struct EsResponse {
    hits: EsHits,
}

#[derive(serde::Deserialize)]
struct EsHits {
    total: EsTotal,
    hits: Vec<EsHit>,
}

#[derive(serde::Deserialize)]
struct EsTotal {
    value: u64,
}

#[derive(serde::Deserialize)]
struct EsHit {
    #[serde(rename = "_source")]
    source: serde_json::Value,
}

/// Parse an Elasticsearch response into `(total_matches, items)`.
///
/// # Errors
///
/// Returns [`NixMcpError::Web`] if the body does not have the expected
/// shape, which means the endpoint changed under us.
pub fn parse_es_response(
    operation: SearchOperation,
    body: &str,
) -> Result<(usize, Vec<output::SearchItem>)> {
    let response: EsResponse = serde_json::from_str(body).map_err(|err| {
        NixMcpError::Web(format!(
            "unexpected response shape from search.nixos.org ({err}); the endpoint may have \
             changed; {WEB_FALLBACK_HINT}"
        ))
    })?;

    let count = usize::try_from(response.hits.total.value).unwrap_or(usize::MAX);
    let items = response
        .hits
        .hits
        .into_iter()
        .filter_map(|hit| es_item(operation, &hit.source))
        .collect();

    Ok((count, items))
}

/// Convert one `_source` document into a [`output::SearchItem`].
fn es_item(operation: SearchOperation, source: &serde_json::Value) -> Option<output::SearchItem> {
    let text = |key: &str| -> Option<String> {
        source
            .get(key)
            .and_then(|v| v.as_str())
            .map(str::to_owned)
            .filter(|s| !s.is_empty())
    };

    match operation {
        SearchOperation::Package => Some(output::SearchItem {
            name: text("package_attr_name")?,
            version: text("package_pversion"),
            description: text("package_description"),
            option_type: None,
            default: None,
            example: None,
            programs: source.get("package_programs").and_then(|v| {
                let programs: Vec<String> = v
                    .as_array()?
                    .iter()
                    .filter_map(|p| p.as_str().map(str::to_owned))
                    .collect();
                (!programs.is_empty()).then_some(programs)
            }),
        }),
        SearchOperation::Option => Some(output::SearchItem {
            name: text("option_name")?,
            version: None,
            description: text("option_description")
                .map(|d| strip_html(&d))
                .filter(|d| !d.is_empty()),
            option_type: text("option_type"),
            default: text("option_default"),
            example: text("option_example"),
            programs: None,
        }),
    }
}

/// Flatten the `<rendered-html>` markup option descriptions are indexed
/// with: drop tags, decode the basic entities, and collapse whitespace.
#[must_use]
pub fn strip_html(input: &str) -> String {
    let mut text = String::with_capacity(input.len());
    let mut in_tag = false;
    for c in input.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            kept if !in_tag => text.push(kept),
            _ => {}
        }
    }

    // `&amp;` last, so `&amp;lt;` decodes to the literal `&lt;`.
    let decoded = text
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&amp;", "&");

    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Render `s` as a Nix string literal, neutralizing `\`, `"`, `${`, and
/// control characters, so caller-supplied text can never escape into the
/// generated expression.
#[must_use]
pub fn nix_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '$' if chars.peek() == Some(&'{') => out.push_str("\\$"),
            plain => out.push(plain),
        }
    }
    out.push('"');
    out
}

/// Escape POSIX ERE metacharacters, so a query is matched literally by
/// `builtins.match`.
#[must_use]
pub fn regex_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(
            c,
            '\\' | '.' | '[' | ']' | '(' | ')' | '{' | '}' | '|' | '^' | '$' | '*' | '+' | '?'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// The shared lowercasing helper embedded in both pinned templates.
const NIX_LOWER: &str = r#"builtins.replaceStrings ["A" "B" "C" "D" "E" "F" "G" "H" "I" "J" "K" "L" "M" "N" "O" "P" "Q" "R" "S" "T" "U" "V" "W" "X" "Y" "Z"] ["a" "b" "c" "d" "e" "f" "g" "h" "i" "j" "k" "l" "m" "n" "o" "p" "q" "r" "s" "t" "u" "v" "w" "x" "y" "z"]"#;

/// Pinned package search: match top-level `legacyPackages` attribute names
/// lazily, then force `version`/`meta.description` only for the page window.
/// `deepSeq` inside `tryEval` keeps a throwing package (alias stubs, unfree
/// markers) from failing the whole query: it degrades to a name-only hit.
const PKG_SEARCH_TEMPLATE: &str = r#"
let
  flake = builtins.getFlake @FLAKE@;
  pkgs = flake.legacyPackages.${builtins.currentSystem};
  lower = @NIX_LOWER@;
  matches = s: builtins.match @REGEX@ (lower s) != null;
  names = builtins.filter matches (builtins.attrNames pkgs);
  count = builtins.length names;
  start = if @OFFSET@ < count then @OFFSET@ else count;
  rawEnd = start + @LIMIT@;
  end = if rawEnd < count then rawEnd else count;
  page = builtins.genList (i: builtins.elemAt names (start + i)) (end - start);
  str = v: if builtins.isString v then v else "";
  detail = n:
    let
      p = pkgs.${n};
      d = {
        name = n;
        version = str (p.version or (p.meta.version or ""));
        description = str (p.meta.description or "");
      };
      r = builtins.tryEval (builtins.deepSeq d d);
    in if r.success then r.value else { name = n; };
in builtins.toJSON { inherit count; results = builtins.map detail page; }
"#;

/// Pinned option search: walk the `nixosSystem` options tree collecting
/// dotted paths (skipping `_`-prefixed internals), match lazily on the
/// paths, and force type/description/default/example only for the page.
/// Submodule options (`<name>.*`) live under `type.getSubOptions` and are
/// not enumerated; the web source covers those.
const OPTION_SEARCH_TEMPLATE: &str = r#"
let
  flake = builtins.getFlake @FLAKE@;
  eval = flake.lib.nixosSystem { system = builtins.currentSystem; modules = [ ]; };
  lower = @NIX_LOWER@;
  matches = s: builtins.match @REGEX@ (lower s) != null;
  isOption = v: builtins.isAttrs v && (v._type or "") == "option";
  collect = prefix: set: builtins.concatLists (builtins.map (n:
    let
      path = if prefix == "" then n else "${prefix}.${n}";
      r = builtins.tryEval (
        let v = set.${n};
        in if builtins.substring 0 1 n == "_" then [ ]
           else if isOption v then [ { inherit path; opt = v; } ]
           else if builtins.isAttrs v then collect path v
           else [ ]);
    in if r.success then r.value else [ ]) (builtins.attrNames set));
  entries = builtins.filter (e: matches e.path) (collect "" eval.options);
  count = builtins.length entries;
  start = if @OFFSET@ < count then @OFFSET@ else count;
  rawEnd = start + @LIMIT@;
  end = if rawEnd < count then rawEnd else count;
  page = builtins.genList (i: builtins.elemAt entries (start + i)) (end - start);
  str = v: if builtins.isString v then v else "";
  text = v: if builtins.isString v then v else if builtins.isAttrs v then str (v.text or "") else "";
  scalarJson = v:
    if builtins.isString v || builtins.isBool v || builtins.isInt v || builtins.isFloat v
    then builtins.toJSON v
    else if v == null then "null"
    else "<complex value>";
  detail = e:
    let
      o = e.opt;
      d = {
        name = e.path;
        type = str (o.type.description or "");
        description = text (o.description or "");
        default =
          if o ? defaultText then text o.defaultText
          else if o ? default then scalarJson o.default
          else "";
        example =
          if o ? example
          then (if builtins.isAttrs o.example then text o.example else scalarJson o.example)
          else "";
      };
      r = builtins.tryEval (builtins.deepSeq d d);
    in if r.success then r.value else { name = e.path; };
in builtins.toJSON { inherit count; results = builtins.map detail page; }
"#;

/// Build the pinned-search Nix expression for one page.
///
/// The query is lowered, regex-escaped, and embedded as a `.*<literal>.*`
/// ERE; both it and the flake ref pass through [`nix_str`], so neither can
/// inject expression syntax. `offset`/`limit` are numeric.
#[must_use]
pub fn pinned_expression(
    operation: SearchOperation,
    query: &str,
    flake: &str,
    offset: usize,
    limit: usize,
) -> String {
    let template = match operation {
        SearchOperation::Package => PKG_SEARCH_TEMPLATE,
        SearchOperation::Option => OPTION_SEARCH_TEMPLATE,
    };
    let regex = format!(".*{}.*", regex_escape(&query.to_lowercase()));

    template
        .replace("@NIX_LOWER@", NIX_LOWER)
        .replace("@FLAKE@", &nix_str(flake))
        .replace("@REGEX@", &nix_str(&regex))
        .replace("@OFFSET@", &offset.to_string())
        .replace("@LIMIT@", &limit.to_string())
}

/// The JSON envelope a pinned-search expression evaluates to.
#[derive(Debug, serde::Deserialize)]
pub struct PinnedReply {
    /// Total matching names, before paging.
    pub count: usize,
    /// The page of hits, detail fields empty when unavailable.
    pub results: Vec<PinnedItem>,
}

/// One pinned-search hit as rendered by the Nix expression.
#[derive(Debug, serde::Deserialize)]
pub struct PinnedItem {
    /// Attribute or option name.
    pub name: String,
    /// Package version ("" or absent when unknown).
    #[serde(default)]
    pub version: Option<String>,
    /// Package or option description.
    #[serde(default)]
    pub description: Option<String>,
    /// Option type description.
    #[serde(rename = "type", default)]
    pub option_type: Option<String>,
    /// Option default rendering.
    #[serde(default)]
    pub default: Option<String>,
    /// Option example rendering.
    #[serde(default)]
    pub example: Option<String>,
}

impl PinnedItem {
    /// Convert to the wire shape, dropping empty detail fields.
    #[must_use]
    pub fn into_output(self) -> output::SearchItem {
        let non_empty = |v: Option<String>| v.filter(|s| !s.is_empty());
        output::SearchItem {
            name: self.name,
            version: non_empty(self.version),
            description: non_empty(self.description),
            option_type: non_empty(self.option_type),
            default: non_empty(self.default),
            example: non_empty(self.example),
            programs: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        es_request_body, nix_str, normalize_channel, parse_es_response, pick_alias,
        pinned_expression, regex_escape, strip_html,
    };
    use crate::tools::SearchOperation;

    #[test]
    fn normalize_channel_accepts_the_documented_forms() {
        assert_eq!(normalize_channel("unstable").expect("valid"), "unstable");
        assert_eq!(normalize_channel("25.11").expect("valid"), "25.11");
        assert_eq!(
            normalize_channel("nixos-unstable").expect("valid"),
            "unstable"
        );
        assert_eq!(normalize_channel(" NixOS-25.11 ").expect("valid"), "25.11");
    }

    #[test]
    fn normalize_channel_rejects_garbage_prescriptively() {
        for bad in ["", "   ", "un stable", "unstable/../secret", "nixos-"] {
            let err = normalize_channel(bad).expect_err("rejected").to_string();
            assert!(err.contains("25.11"), "must name a valid form, got: {err}");
        }
    }

    #[test]
    fn pick_alias_takes_the_newest_generation_for_the_channel() {
        let body = r#"{
            "nixos-47-unstable-aaa": {"aliases": {"latest-47-nixos-unstable": {}}},
            "nixos-48-unstable-bbb": {"aliases": {"latest-48-nixos-unstable": {}}},
            "nixos-46-25.11-ccc": {"aliases": {"latest-46-nixos-25.11": {}}},
            "group-48-manual-ddd": {"aliases": {}}
        }"#;
        assert_eq!(
            pick_alias(body, "unstable").as_deref(),
            Some("latest-48-nixos-unstable")
        );
        assert_eq!(
            pick_alias(body, "25.11").as_deref(),
            Some("latest-46-nixos-25.11")
        );
        assert_eq!(pick_alias(body, "99.99"), None);
        assert_eq!(pick_alias("not json", "unstable"), None);
    }

    #[test]
    fn es_request_body_filters_type_and_pages() {
        let body = es_request_body(SearchOperation::Package, "RipGrep", 40, 20);
        assert_eq!(body["from"], 40);
        assert_eq!(body["size"], 20);
        assert_eq!(
            body["query"]["bool"]["filter"][0]["term"]["type"]["value"],
            "package"
        );
        // The wildcard arm lowercases for the case-insensitive contains match.
        let queries = body["query"]["bool"]["must"][0]["dis_max"]["queries"]
            .as_array()
            .expect("dis_max queries");
        assert!(
            queries
                .iter()
                .any(|q| q["wildcard"]["package_attr_name"]["value"] == "*ripgrep*")
        );

        let body = es_request_body(SearchOperation::Option, "nginx", 0, 10);
        assert_eq!(
            body["query"]["bool"]["filter"][0]["term"]["type"]["value"],
            "option"
        );
    }

    #[test]
    fn parse_es_response_extracts_package_hits() {
        // Shape captured from a live probe against latest-48-nixos-unstable.
        let body = r#"{"hits":{"total":{"value":29,"relation":"eq"},"hits":[
            {"_source":{"type":"package","package_attr_name":"ripgrep",
             "package_pname":"ripgrep","package_pversion":"15.1.0",
             "package_description":"Utility that combines the usability of The Silver Searcher with the raw speed of grep",
             "package_programs":["rg"]}}
        ]}}"#;
        let (count, items) = parse_es_response(SearchOperation::Package, body).expect("parse");
        assert_eq!(count, 29);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "ripgrep");
        assert_eq!(items[0].version.as_deref(), Some("15.1.0"));
        assert_eq!(items[0].programs.as_deref(), Some(&["rg".to_owned()][..]));
        assert!(items[0].option_type.is_none());
    }

    #[test]
    fn parse_es_response_extracts_and_unwraps_option_hits() {
        let body = r#"{"hits":{"total":{"value":4,"relation":"eq"},"hits":[
            {"_source":{"option_name":"services.nginx.enable",
             "option_type":"boolean",
             "option_description":"<rendered-html><p>Whether to enable Nginx Web Server.</p>\n</rendered-html>",
             "option_default":"false","option_example":"true"}}
        ]}}"#;
        let (count, items) = parse_es_response(SearchOperation::Option, body).expect("parse");
        assert_eq!(count, 4);
        assert_eq!(items[0].name, "services.nginx.enable");
        assert_eq!(items[0].option_type.as_deref(), Some("boolean"));
        assert_eq!(
            items[0].description.as_deref(),
            Some("Whether to enable Nginx Web Server.")
        );
        assert_eq!(items[0].default.as_deref(), Some("false"));
    }

    #[test]
    fn parse_es_response_rejects_unexpected_shapes() {
        let err = parse_es_response(SearchOperation::Package, "{}")
            .expect_err("shape error")
            .to_string();
        assert!(err.contains("source=pinned"), "got: {err}");
    }

    #[test]
    fn strip_html_flattens_rendered_option_docs() {
        assert_eq!(
            strip_html(
                "<rendered-html><p>Whether to enable Nginx Web Server.</p>\n</rendered-html>"
            ),
            "Whether to enable Nginx Web Server."
        );
        assert_eq!(strip_html("a &lt;b&gt; &amp; c"), "a <b> & c");
        assert_eq!(strip_html("  spaced\n\nout  "), "spaced out");
    }

    #[test]
    fn nix_str_neutralizes_escapes_and_interpolation() {
        assert_eq!(nix_str("plain"), r#""plain""#);
        assert_eq!(nix_str(r#"a"b"#), r#""a\"b""#);
        assert_eq!(nix_str(r"a\b"), r#""a\\b""#);
        assert_eq!(nix_str("${evil}"), r#""\${evil}""#);
        assert_eq!(nix_str("a$b"), r#""a$b""#);
        assert_eq!(nix_str("line\nbreak"), r#""line\nbreak""#);
    }

    #[test]
    fn regex_escape_covers_ere_metacharacters() {
        assert_eq!(regex_escape("a.b*c"), r"a\.b\*c");
        assert_eq!(regex_escape("x(y|z)+"), r"x\(y\|z\)\+");
        assert_eq!(regex_escape("plain-name_1"), "plain-name_1");
    }

    #[test]
    fn pinned_expression_embeds_nothing_executable() {
        let evil = r#"${builtins.exec ["sh"]}"#;
        let expr = pinned_expression(SearchOperation::Package, evil, evil, 0, 10);
        assert!(
            !expr.contains(evil),
            "raw query text must not survive into the expression"
        );
        // The template's own interpolations stay intact.
        assert!(expr.contains("${builtins.currentSystem}"));
    }

    #[test]
    fn pinned_expression_fills_every_placeholder() {
        for operation in [SearchOperation::Package, SearchOperation::Option] {
            let expr = pinned_expression(operation, "ripgrep", "nixpkgs", 7, 13);
            assert!(!expr.contains('@'), "unfilled placeholder in: {expr}");
            assert!(expr.contains(r#"builtins.getFlake "nixpkgs""#));
            assert!(expr.contains(r#"".*ripgrep.*""#));
            assert!(expr.contains("if 7 < count"));
            assert!(expr.contains("start + 13"));
        }
    }
}

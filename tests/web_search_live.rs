//! Live tests for the `nix_search` web backend against the real
//! search.nixos.org endpoint.
//!
//! `#[ignore]`d by default: they need network access and depend on a service
//! we do not control, so they stay out of `cargo test` and CI. Run them
//! deliberately after touching `src/search.rs`:
//!
//! ```sh
//! cargo test --test web_search_live -- --ignored
//! ```

use nix_mcp::search::WebSearch;
use nix_mcp::tools::SearchOperation;

#[tokio::test]
#[ignore = "requires network access to search.nixos.org"]
async fn live_package_search_finds_ripgrep() {
    let web = WebSearch::new().expect("http client");
    let result = web
        .search(SearchOperation::Package, "ripgrep", "unstable", 0, 5)
        .await
        .expect("live package search");

    assert_eq!(result.source, "web");
    assert_eq!(result.channel.as_deref(), Some("unstable"));
    assert!(result.count > 0, "ripgrep must match something");
    assert!(
        result.results.iter().any(|r| r.name == "ripgrep"),
        "expected an exact ripgrep hit, got: {:?}",
        result
            .results
            .iter()
            .map(|r| r.name.as_str())
            .collect::<Vec<_>>()
    );
    let exact = result
        .results
        .iter()
        .find(|r| r.name == "ripgrep")
        .expect("exact hit");
    assert!(exact.version.is_some(), "the index carries a version");
}

#[tokio::test]
#[ignore = "requires network access to search.nixos.org"]
async fn live_option_search_finds_nginx_enable_with_clean_description() {
    let web = WebSearch::new().expect("http client");
    let result = web
        .search(
            SearchOperation::Option,
            "services.nginx.enable",
            // The `nixos-` prefix must normalize away.
            "nixos-unstable",
            0,
            5,
        )
        .await
        .expect("live option search");

    assert_eq!(result.channel.as_deref(), Some("unstable"));
    let hit = result
        .results
        .iter()
        .find(|r| r.name == "services.nginx.enable")
        .expect("exact option hit");
    assert_eq!(hit.option_type.as_deref(), Some("boolean"));
    let description = hit.description.as_deref().expect("description");
    assert!(
        !description.contains('<') && !description.contains('>'),
        "rendered-html markup must be stripped: {description}"
    );
}

#[tokio::test]
#[ignore = "requires network access to search.nixos.org"]
async fn live_unknown_channel_is_a_course_correcting_error() {
    let web = WebSearch::new().expect("http client");
    let err = web
        .search(SearchOperation::Package, "ripgrep", "99.99", 0, 5)
        .await
        .expect_err("no index exists for channel 99.99")
        .to_string();
    assert!(err.contains("25.11"), "must name a valid form, got: {err}");
}

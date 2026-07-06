//! The crate error type [`NixMcpError`] and its [`Result`] alias.

/// Failure modes for `nix_mcp` operations.
///
/// `#[non_exhaustive]` so future variants can be added without a breaking
/// change if this crate ever grows a public library surface.
#[derive(Debug)]
#[non_exhaustive]
pub enum NixMcpError {
    /// A failure surfaced by the Nix C API through `nix-bindings` (evaluation,
    /// store access, derivation reads, flake locking, ...).
    ///
    /// The Nix C API catches C++ exceptions internally and reports them through
    /// the `Context` handle, so every `nix-bindings` method that takes a context
    /// converts a thrown C++ exception into this variant instead of aborting.
    /// Context-less free helpers that pass a null context (e.g.
    /// `nix_bindings::is_pure_eval`) do not catch, and a throw there aborts the
    /// process; the worker avoids them. See the `worker` module docs.
    Nix(nix_bindings::Error),

    /// A worker-channel failure: the worker thread is gone, or it dropped the
    /// reply channel without answering. Unlike the other variants this is not
    /// something an agent can retry around, so it is the one failure class
    /// that surfaces as a protocol-level error instead of an in-band tool
    /// error (see `send_op` in the crate root).
    Channel(String),

    /// A caller supplied a missing or malformed argument.
    InvalidArgument(String),

    /// A `nix_search` web-backend failure: transport, a non-success status
    /// from search.nixos.org, or an unrecognized response shape. Always
    /// in-band; the message names `source=pinned` as the offline fallback.
    Web(String),

    /// A server-side bug surfaced as a legible error instead of a panic (a
    /// worker panic aborts the whole process by policy). Currently only
    /// reply serialization, which cannot fail for the output structs in
    /// practice.
    Internal(String),
}

impl std::fmt::Display for NixMcpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::Nix(err) => format!("Nix error: {err}"),
            Self::Channel(msg) => format!("Worker channel error: {msg}"),
            Self::InvalidArgument(msg) => format!("Invalid argument: {msg}"),
            Self::Web(msg) => format!("Web search error: {msg}"),
            Self::Internal(msg) => format!("Internal error: {msg}"),
        };

        // Nix renders exception text for a terminal, with SGR color codes
        // baked into the string before any isatty/NO_COLOR check runs, so
        // they survive the C API and would otherwise reach the agent. Every
        // variant can embed Nix text (`InvalidArgument` wraps it for
        // guidance), so strip at this single choke point: it covers tool
        // replies and `tracing` output alike.
        f.write_str(&strip_ansi(&message))
    }
}

impl std::error::Error for NixMcpError {}

impl From<nix_bindings::Error> for NixMcpError {
    fn from(err: nix_bindings::Error) -> Self {
        Self::Nix(err)
    }
}

impl From<serde_json::Error> for NixMcpError {
    fn from(err: serde_json::Error) -> Self {
        Self::Internal(format!("failed to serialize a reply: {err}"))
    }
}

/// Remove ANSI escape sequences from `input`.
///
/// Handles CSI sequences (`ESC [` parameters up to and including a final byte
/// in `@`..=`~`, which covers the SGR color codes Nix emits) and drops the
/// single following character of any other escape sequence. A trailing bare
/// `ESC` is dropped.
fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }

        if chars.peek() == Some(&'[') {
            chars.next();
            for c in chars.by_ref() {
                if ('\u{40}'..='\u{7e}').contains(&c) {
                    break;
                }
            }
        } else {
            chars.next();
        }
    }

    out
}

/// [`std::result::Result`] specialized to [`NixMcpError`].
pub type Result<T> = std::result::Result<T, NixMcpError>;

#[cfg(test)]
mod tests {
    use super::{NixMcpError, strip_ansi};

    #[test]
    fn strip_ansi_passes_plain_text_through() {
        assert_eq!(strip_ansi("no escapes here"), "no escapes here");
    }

    #[test]
    fn strip_ansi_removes_the_sgr_codes_nix_emits() {
        // The exact shape observed from a live `nix_derivation_info` probe.
        let colored = "\u{1b}[31;1merror:\u{1b}[0m store path \
                       '\u{1b}[35;1mfoo\u{1b}[0m' is malformed";
        assert_eq!(strip_ansi(colored), "error: store path 'foo' is malformed");
    }

    #[test]
    fn strip_ansi_survives_a_truncated_escape_at_the_end() {
        assert_eq!(strip_ansi("cut \u{1b}"), "cut ");
        assert_eq!(strip_ansi("cut \u{1b}[31"), "cut ");
    }

    #[test]
    fn display_prefixes_and_strips_each_variant() {
        let invalid = NixMcpError::InvalidArgument("bad \u{1b}[31mvalue\u{1b}[0m".to_owned());
        assert_eq!(invalid.to_string(), "Invalid argument: bad value");

        let channel = NixMcpError::Channel("worker gone".to_owned());
        assert_eq!(channel.to_string(), "Worker channel error: worker gone");
    }
}

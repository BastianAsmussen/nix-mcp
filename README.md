# nix_mcp

MCP server that provides LLM coding agents fast, direct access to the Nix store,
evaluator, and flake system, without spawning `nix` CLI processes.

Uses [nix-bindings](https://github.com/notashelf/nix-bindings) to keep a live
in-process Nix context, enabling sub-millisecond operations after initialization.

## Quick Start

```sh
nix develop       # enter dev shell with rustc, cargo, clippy, rust-analyzer
nix build         # build the server binary
cargo check       # fast compile-check
```

## MCP Tools

| Tool | Description |
|------|-------------|
| `nix_store_info` | Get store URI, directory, and daemon version |
| `nix_eval` | Evaluate a Nix expression, with optional attribute access |
| `nix_store_path_query` | Validate paths, resolve real paths, compute closures, resolve hash parts |
| `nix_realize` | Build/download a store path and its dependencies |
| `nix_flake_resolve` | Resolve and lock a flake reference; list output attributes |
| `nix_derivation_info` | Read derivation metadata from a `.drv` store path |
| `nix_search` | Search nixpkgs packages and NixOS options (search.nixos.org, or offline against a pinned flake) |

## Configuration

Configure an MCP client to launch this server via stdio:

```json
{
  "mcpServers": {
    "nix_mcp": {
      "command": "nix",
      "args": ["run", "github:BastianAsmussen/nix_mcp"]
    }
  }
}
```

Or point to a local build:

```json
{
  "mcpServers": {
    "nix_mcp": {
      "command": "/path/to/result/bin/nix_mcp"
    }
  }
}
```

## Architecture

A dedicated worker thread owns all Nix state (`Context`, `Store`, `EvalState`) and
processes operations sequentially through `flume` channels. The async MCP handler
(tokio, single-threaded) dispatches tool calls to the worker and awaits results.
This matches nix-bindings' `Send`-but-not-`Sync` threading contract.

## Requirements

- Nix with flakes enabled
- x86_64-linux or aarch64
- Rust 1.90.0+

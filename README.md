# gitspin

Contribution tracking across git forges — GitHub today; GitLab and Forgejo
planned. gitspin reports per-contributor commits, lines added/removed, and PR
reviews over a set of repos and a time window, aggregatable across providers.

## Architecture

Provider-agnostic by construction:

```
crates/gitspin-core     Domain model: types, TimeWindow, config, and the
                        ContributionProvider trait. No forge knowledge, no I/O.
crates/gitspin-github   GitHub adapter: read-only REST implementation of the
                        trait. Pure response parsers are unit-tested offline.
crates/gitspin-cli      CLI + HTTP surfaces, projected by Hydra from
                        api/operations.yaml (single source of truth).
```

### Hydra is the sole projection mechanism

CLI, HTTP, and MCP surfaces are generated, never hand-written:

1. Declare operations in `crates/gitspin-cli/api/operations.yaml`.
2. Regenerate from `crates/gitspin-cli`:
   `cargo run --locked -p gitspin-codegen --bin gitspin-codegen -- write`.
   (Thin shim over the pinned `hydra-codegen` library — `cargo run -p`
   cannot select git-dependency packages directly.)
3. Commit the four generated artifacts under `crates/gitspin-cli/generated/`.
4. CI gates staleness: the same command with `check`.

`hydra-core` and `hydra-codegen` are pinned as git dependencies at the
immutable `v0.3.0` tag. Tools are LLM-native but never require an LLM: every
surface is ordinary Rust with one dispatch function
(`gitspin_cli::execute_operation`).

## Provider status

| Provider | Status |
|----------|--------|
| GitHub   | Implemented (read-only: commits, contributors, PR reviews) |
| GitLab   | Deferred — declared in `ProviderConfig`, adapter stub returns `NotImplemented` |
| Forgejo  | Deferred — same |

## Usage

```bash
cargo run -p gitspin-cli --bin gitspin -- provider-info
cargo run -p gitspin-cli --bin gitspin -- list-contributors --provider github --owner TechGodHQ --repo gitspin
cargo run -p gitspin-cli --bin gitspin -- list-commits --provider github --owner TechGodHQ --repo gitspin \
  --since 2026-01-01T00:00:00Z
```

HTTP surface: `cargo run -p gitspin-cli --bin gitspin-http` (serves
`127.0.0.1:8943`). MCP tool schemas ship in
`crates/gitspin-cli/generated/mcp.json`.

## Development

```bash
cargo build --locked --all-targets
cargo test --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --all -- --check
cd crates/gitspin-cli && cargo run --locked -p gitspin-codegen --bin gitspin-codegen -- check
```

## Roadmap

- GitHub adapter: per-commit stats fetch to implement `lines_changed`; window
  filtering for reviews; pagination.
- GitLab adapter.
- Forgejo adapter.
- Cross-provider aggregation reports.

# AGENTS.md

Agent-specific guidance for Bitcredit-Core. See [README.md](README.md) for the project overview
and the organisation's [contributing guide](https://github.com/BitcreditProtocol/.github/blob/master/CONTRIBUTING.md)
for shared contribution rules.

Working preferences are defaults; the compatibility requirements below are mandatory.
If a task conflicts with them, surface the conflict.

Be terse. Make the smallest change that fully solves the task. Avoid unrelated refactoring
and documentation.

## Project Map

Dependencies run `bcr-ebill-core` → `bcr-ebill-persistence` → `bcr-ebill-api` →
`bcr-ebill-transport` → `bcr-ebill-wasm`; only the last ships, as the npm package
`@bitcredit/bcr-ebill-wasm`. There is no server: keys and all state live in the browser
(SurrealDB over IndexedDB), and Nostr relays, Esplora and mints are external services the
client talks to. The native target exists so `cargo check/test/clippy` run on a host — no
crate defines a binary.

## Quality Gates

Full gate before every PR; the workspace suite is one command:

    just check   # wasm dev build, fmt --check, cargo check, cargo test --all,
                 # clippy --all-targets --all-features -D warnings, cargo deny check
    just wasm    # wasm-pack dev build alone — the fast wasm32 compile check

Needs `wasm-pack` and `cargo-deny` on PATH ([docs/wasm.md](docs/wasm.md) covers wasm-pack).
CI does not use `just`: `Rust CI` marks fmt, clippy and deny `continue-on-error` and
`Test coverage` runs `cargo test --workspace`, so a green CI does not prove clippy or fmt
passed — `just check` is where they are enforced, and the PR checklist asks for both.

- Tests run natively only; `#[cfg(target_arch = "wasm32")]` branches are compile-checked by
  the wasm build and never executed (there is no `wasm-bindgen-test`).
- Keep tests hermetic: in-memory SurrealDB (`kv-mem`), `mockall`, `mockito` and the
  in-process `nostr-relay-builder` relay are the fixtures; CI has no live relay, Esplora or mint.
- NEVER trigger the `WASM Release` workflow to test anything: it tags, creates a GitHub
  release and publishes to npm, none of which is undoable. Validate wasm builds with
  `Rust CI` instead (README, "WASM publication approval").

## Key Patterns

- **Compile for wasm32 and native.** wasm32 is single-threaded: services use
  `ServiceTraitBounds` (`bcr-ebill-core/src/application/mod.rs`; `Send + Sync` natively, empty
  on wasm32) instead of bare bounds, `tokio_with_wasm::alias as tokio` for timers and spawning,
  and Cargo `[target.'cfg(...)']` tables for platform deps. A bare `tokio::time` call compiles
  natively and fails only at `just wasm`. Build wasm from the crate — its `.cargo/config.toml`
  sets the wasm32 target and the `getrandom_backend="wasm_js"` cfg; `pkg/` is wasm-pack
  output, untracked and never hand-edited.
- **`protocol/` types are the wire format.** Blocks are borsh-serialised, hashed and
  Schnorr-signed, so a field change alters hashes other clients already verify. Chains are
  append-only (`Blockchain::try_add_block`, no removal) and every inbound block is re-validated
  locally: relays are transport, not consensus (CHANGELOG 0.5.1-1 still calls chain reordering
  a pre-mainnet placeholder). Backwards compatibility of wire formats and persisted data is
  mandatory. Never introduce changes that break existing clients or stored data.
- **Bill, mint and payment states are separate machines.** `BillState` in `application/bill`,
  `MintRequestStatus` in `protocol/mint` and Esplora payment checks
  (`bcr-ebill-api/src/external/bitcoin.rs`) are computed independently. A valid signature
  proves who signed, not that a bill is paid or a mint solvent; `Accepted`/`MintingEnabled`
  mean the mint agreed, and `MintOffer.proofs` is separately optional.
- **The JS boundary is a public API.** Types in `crates/bcr-ebill-wasm/src/data/` derive
  `Tsify` and become the npm package's TypeScript surface used by the web frontend; shape
  changes there are breaking API changes.
- **Dependencies float.** `Cargo.lock` is gitignored, so CI resolves fresh within `Cargo.toml`
  ranges — don't commit one. `bcr-common` is pinned by git `rev` in the root `Cargo.toml` and
  bumped there (deny.toml allows git sources only from the BitcreditProtocol org).

## Common Gotchas

1. **Keep `[profile.dev] opt-level = 1`** (root `Cargo.toml`); its comment records that
   opt-level 0 hits a SurrealDB index-out-of-bounds bug.
2. **Amounts cross to JS as strings.** `Sum` serialises as a string to avoid JavaScript
   precision loss (CHANGELOG 0.5.1-1); don't expose sats as numbers.

This list grows from real incidents only — add one whenever an agent or human loses time
here; it is the cheapest productivity investment in the repo.

## Glossary

Names that look alike belong to separate machines (Key Patterns); never join them.

- **`BillAcceptState::Accepted`** — an `Accept` block proves the drawee assented, not that anyone paid.
- **`BillPaymentState::Paid`** — Esplora confirmed the funds; every lesser state is unpaid.
- **`QuoteStatusReply::Accepted`** / **`MintRequestStatus::Accepted`** — the mint (remote) or
  this client (stored) recorded offer assent; minting is not yet allowed.
- **`MintingEnabled`** — the mint permits issuance; `MintOffer.proofs` may still be absent.
- **signed** — a verified block authenticates its signer, nothing about business state.

## Plans and work artifacts

- Plans, research notes and scratch files stay outside the worktree or in the gitignored
  `docs/plans/`; working state is not product documentation.
- The merged PR plus its CHANGELOG bullet is the implementation record. Do not add a second
  checklist or PR summary to the repo; it drifts from the code.

## Working Agreements

Organisation-wide rules (branch protection, reviews, labels, Dependabot) live in the
[contributing guide](https://github.com/BitcreditProtocol/.github/blob/master/CONTRIBUTING.md).
This section is the per-task delta.

- Open pull requests against `master`. Branch from it too: basing work on another branch
  conflicts in exactly the files other people are changing.
- Never open, mark ready or merge a PR, and never push a tag, unless the developer
  explicitly asks. Each is visible to the whole team, and a tag also starts a release.
- Commit small and often. Each commit is self-contained, passes the gate above and is
  reviewable on its own; the subject says why, not just what. Reviewers only catch
  mistakes in changes they can hold in their head.
- Titles: conventional-commit style in plain language, e.g. `fix(api): recourse blocks
  reach the drawee again`. Release notes are built from labels (see
  [`.github/release.yml`](.github/release.yml)), so label `bug`, `enhancement`,
  `documentation` or `dependencies`.
- Body: the problem in a sentence or two, then how it was fixed, then how it was verified.
  The [PR template](.github/PULL_REQUEST_TEMPLATE.md) asks exactly that. End with the
  model and harness that did the work.
- Evidence: the test that failed before and passes now, or for a wire or public API
  change, the consumer that was rebuilt against it. Upload evidence to the PR on GitHub;
  never commit PR-only screenshots or assets.
- One concern per PR. If the description needs an "also", split it.
- Babysitting a PR: poll checks and comments newer than the last push; verify each bot
  finding against the source, fix the real ones, dismiss false positives with a written
  reason. No status check is required to merge, so a red check may predate your change:
  confirm that before blaming it, and say so in the PR. Stay quiet when nothing is new;
  stop when checks are green on the latest commit.
- Every PR adds a bullet to the top section of [CHANGELOG.md](CHANGELOG.md). Describe
  public API changes in the PR and changelog. The version is bumped once per
  cycle in the root `[workspace.package]` (`init X.Y.Z` commits), never per PR; scheme in
  [docs/versioning.md](docs/versioning.md).
- Workflows pin actions by commit SHA with a version comment (#982); keep that form.
  Releases follow [docs/wasm_releasing.md](docs/wasm_releasing.md) plus the README
  approval rules (`master` and `hotfix/*` only).

## See Also

- [docs/index.md](docs/index.md) — documentation hub
- [docs/concepts.md](docs/concepts.md) — bill actions and states by role; read before touching bill logic
- [docs/wasm.md](docs/wasm.md), [docs/wasm_configuration.md](docs/wasm_configuration.md) — building, the JS playground (`just serve`), runtime `Config`
- [docs/testing.md](docs/testing.md) — which layer gets which kind of test
- [.github/copilot-instructions.md](.github/copilot-instructions.md) — Copilot adapter with a longer architecture tour; check it for drift when gates change here

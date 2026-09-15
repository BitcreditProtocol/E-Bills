# E-Bills

Core for Bitcredit E-Bills project.

### Crates

The project consists of the following crates:

* `bcr-ebill-core` - core data models and traits
* `bcr-ebill-persistence` - persistence traits and SurrealDB implementation
* `bcr-ebill-transport` - network transport API traits and Nostr implementation
* `bcr-ebill-api` - API of the E-Bills project, contains most of the business logic
* `bcr-ebill-wasm` - Entrypoint for WASM version of the E-Bill API

### Entrypoint

There is a `WASM` entry point into the API. You can find the documentation to build and configure it [here](docs/index.md):

### Tests

You can run the existing tests using the following commands in the project root:

```bash
// without logs
cargo test

// with logs - (env_logger needs to be activated in the test to show logs)
RUST_LOG=info cargo test -- --nocapture
```

## Contribute

Check out the organisation's [contributing guide](https://github.com/BitcreditProtocol/.github/blob/master/CONTRIBUTING.md).

## WASM publication approval

The manual `WASM Release` workflow uses the `release-wasm` GitHub environment.
Publication requires approval from one of the reviewers configured for that
environment. GitHub environment settings are the source of truth for the list.
Self-approval is allowed, administrators retain their bypass, and there is no
additional wait timer. The workflow's existing initiator allowlist still applies.

Only the `master` branch and branches matching `hotfix/*` may use this
environment. Use names such as `hotfix/0.5.7-1` for new hotfix branches. Historical
branches keep their names; they do not gain publication permission from an older
naming convention. No tag policies are configured.

Validate WASM builds with the normal `Rust CI` workflow. Do not run the publication
workflow merely to test environment settings: it creates a release and publishes
the npm package.

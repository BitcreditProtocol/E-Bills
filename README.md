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

Check out the project [contributing guide](./CONTRIBUTING.md).

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

## WASM publication recovery

The workflow prepares the npm tarball and GitHub release assets before creating
any tag, release or npm version. It saves their SHA-256 checksums, npm integrity,
source commit, version and original run ID in the immutable `release-package`
Actions artifact for 90 days. Source and generated package versions must match.
SemVer build metadata remains in the source tag and package; npm registry version
identity excludes build metadata.

To recover a partial publication, use **Re-run failed jobs** or rerun the
**WASM Release and Publish** job (`release`) on the original run. This native
partial rerun retains the original package artifact. Do not use **Re-run all
jobs**, which removes previous artifacts despite their retention period.
The existing `release-wasm` environment protection still applies.

If the artifact is absent on a retry, complete native job history must prove
that package saving never started before a fresh preparation is allowed. A
missing previously saved package or an incomplete inventory/history stops
recovery, even when no publication is visible yet.

The job restores the original package, verifies its saved bytes
and existing tags and assets, and adds only missing publication results. A lost
write response is checked against remote state before continuing. Conflicting
content or an unavailable artifact after publication starts stops recovery; do
not move tags or overwrite assets.
The GitHub release stays draft until its assets and npm integrity are confirmed.
Stable versions use npm `latest`; prereleases use `next`.

GitHub allows native reruns for 30 days after the original run. Keeping the
artifact for 90 days does not extend that window. Beyond it, retain the evidence
and arrange a separate operator recovery; a fresh dispatch cannot adopt an
existing version without its original saved artifact.

`WASM release regression checks` simulates partial writes, lost responses,
conflicts and unreadable metadata without publication credentials. Normal
`Rust CI` also builds and packs the real WASM output without publishing it.

# Agent Instructions

- Before pushing or opening/updating a PR, run `just ci`.
- If `just` is not installed, install it or run the exact commands from the `ci` recipe manually; do not skip local CI.
- Treat `just ci` as the local source of truth for GitHub Actions coverage. It runs locked dependency fetch, formatting check, clippy with `-D warnings`, all-target/all-feature tests, release build, CLI help, fixture config, and validation-error checks.
- Do not substitute `cargo test` alone for PR validation.

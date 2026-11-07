# Contributing

Follow the [README setup](README.md#build-from-source). Use Windows with the MSVC toolchain. Never commit credentials, personal email addresses, signing keys, or private logs.

## Checks

Dot-source `scripts/prepare-native.ps1` in the shell before native checks. Run:

```powershell
bun install --frozen-lockfile
bun test
bun run build
bun run test:ui
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml dictation::model::tests::local_huggingface_links_work_and_network_links_are_rejected -- --ignored --exact --nocapture
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
```

These include the Windows CI checks in `.github/workflows/check.yml`. UI smoke requires installed Microsoft Edge and uses test data. It can rewrite `docs/screens`; keep only intentional screenshot changes. Native permission and device behavior still need Windows testing.

Regenerate dependency notices after dependency changes with `bun run scripts/third-party-notices.ts`. Install dependencies first. Review the model and trademark section separately; the script preserves it.

## Branches and pull requests

Treat main as protected. Make changes on a topic branch and submit a PR to main. All changes go through PRs. Keep each PR focused. Explain the problem, changes, checks, and limitations. UI changes need screenshots and a note if they use test data. Use raw.githubusercontent.com links pinned to the screenshot commit in PR descriptions.

Use a Conventional Commit title such as `fix: correct media seeking` or `docs: clarify setup`. Titles affect release versioning. Do not change version numbers for ordinary contributions. See [releasing](docs/releasing.md) for the version PR process. Maintainers review and merge; contributors must not bypass required checks.

## Bug reports

Use the bug report form. Include Tuck and Windows versions, reproduction steps, expected and actual behavior. Remove private text from logs and images. Report vulnerabilities privately through [Security](SECURITY.md). Follow the [code of conduct](CODE_OF_CONDUCT.md).

# Stonks Chat App

Native GPUI desktop client for Stoat. This standalone tree contains the app (`crates/stonks-app`), protocol and domain code (`crates/stonks-core`), and shared UI (`crates/stonks-ui`). It does not require a web frontend or a sibling checkout.

Use the pinned `rust-toolchain.toml`. See [contributing](CONTRIBUTING.md) for locked checks and [native packaging](packaging/gpui/README.md) for platform prerequisites. App version comes from `crates/stonks-app/Cargo.toml`.

Release discovery reads stable version metadata only from `mkrotel14/stonks-chat-app`. A missing repository or release is reported as unavailable; there is no fallback, download, or update installation. CI produces run-scoped artifacts, not releases.

Project code is Apache-2.0; see [LICENSE](LICENSE) and [third-party notices](THIRD_PARTY_NOTICES.md) for separate terms.

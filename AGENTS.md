# Native GPUI development

Keep full feature contexts in `crates/stonks-app/src/features/`. Colocate substantial workflows with their view, state, actions and tests. The shell composes feature views and navigation. Cross-feature primitives belong in `src/shared/`, reusable design-system controls in `stonks-ui`, and transport/domain code in `stonks-core`. Preserve established interaction patterns and use shared controls and semantic tokens.

Use the pinned Rust toolchain, locked dependency checks, and the narrowest relevant tests. Visually verify UI changes where possible and report what remains unverified. Do not build installer packages or publish releases unless explicitly requested. See `CONTRIBUTING.md` for commands.

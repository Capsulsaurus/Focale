# Focale

Deterministic, guided raw photo developer. Rust workspace; GUI is winit + wgpu + egui.
The product spec is `docs/prd.md`; every delegated design decision is recorded in
`docs/architecture.md`. Read both before changing processing code.

## Invariants (from the PRD — never violate)

- **Determinism:** the export path is CPU-only and bit-identical across machines and
  architectures. No `fast-math`, no non-deterministic parallel reductions, fixed
  iteration orders. GPU is preview-only.
- **Permanent versioning:** changing any algorithm's output requires a new pipeline
  version while keeping the old implementation. Same rule for the sidecar schema.
- **Local-only:** no network calls anywhere in the application.
- **AGPL-3.0:** verify license compatibility before adding any crate or model weights.
- **One way to do each thing:** no redundant/duplicate UI controls.

## Quality

Validate changes with `just check` (exactly what CI runs), or individually:

```bash
cargo test --workspace                                  # correctness
cargo fmt --check                                       # formatting
cargo clippy --workspace --all-targets -- -D warnings   # lint
```

- All `pub` items need doc comments where non-obvious; processing code documents its
  algorithm source (paper/reference implementation).
- Floating-point code on the export path must not depend on evaluation order that the
  compiler is free to change; keep reductions explicit and sequential.

## Commits

Commits MUST follow [Conventional Commits](https://www.conventionalcommits.org/)
(`feat:`, `fix:`, `chore:`, …) — enforced by `convco` at commit-msg, pre-push, and in
CI on pull requests. Merge commits are exempt.

## Releases

Releases are driven by release-plz: it maintains a version-bump pull request, and
merging that PR tags the release and generates `CHANGELOG.md`. Never bump the version
or tag manually.

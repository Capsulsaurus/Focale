# Focale

Mathematically-guided RAW image processor.

## Guiding Principles

- Color representation: Mathematically model and expose images in physically faithful model.
- Offer tools to extend photos with creative freedom
- User-driven design: Everything you see feels like you can touch it. Responsive. Interactive. Intuitive.

## Implementation Guidelines

- Lean on open-source dependencies where possible and hand-roll for things missing. The upcoming `gamut` ecosystem should be able to handle majority of complexities in codebase.
- UI is designed to be consistent with common applications. No user guide should be necessary to explain any new feature.
- Assume hardware is capable and scale upwards.
- Use Rust where possible for its memory guarantees and modern toolchain. Drop to C/C++ for native APIs if strictly necessary. Compile with LLVM for all targets.
- Subsystems clearly define ownership of logic.

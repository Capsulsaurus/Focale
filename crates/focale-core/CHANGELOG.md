# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/Capsulsaurus/Focale/releases/tag/focale-core-v0.1.0) - 2026-07-11

### Added

- *(app)* desktop application and headless export CLI
- *(core)* wire v1 render orchestration and freeze regression golden
- *(core)* implement v1 stages 7-10 (detail, retouch, geometry, finishing)
- *(core)* implement v1 stages 3-6 (white balance, tone, colour, local)
- *(core)* implement v1 mask rasterization
- *(core)* declare v1 stage modules and preview scale contract
- *(core)* add pipeline engine skeleton and gray plane; scaffold focale-export
- *(core)* add colour-science module
- *(core)* add rawshift-backed raw decode to linear camera RGB f32
- *(core)* add working image buffer (interleaved RGB f32)
- *(core)* add pipeline parameter and mask models

### Fixed

- *(core)* flip WB tint sign to match the magenta-positive convention

### Other

- scaffold Rust workspace and development tooling

# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Work-in-progress feature that is not yet released.

## [0.2.0] - 2026-05-15

### Added

- New scenario pack `motor-limits` covering lower-bound torque clamping.
- `compiler --explain` flag that prints the decision trace for a scenario.

### Changed

- `PRODUCT_SCHEMA_VERSION` bumped to `2`. Compiled artifacts from 0.1.x are
  not forward-compatible and must be recompiled.

### Fixed

- Interpreter no longer panics on empty policy input; now returns exit 2
  with a clear error.

## [0.1.0] - 2026-04-09

### Added

- First public release of ConfigFlux.
- Compiler, runtime, and interpreter binaries.
- ROS2 SDK adapter with lifecycle-mode coverage.
- Scenario packs: early-binding generator, software BOM.

### Security

- Supply chain audit baseline established; dependency lockfile committed.

## [0.0.1] - 2026-03-01

### Added

- Internal prototype snapshot. Not published.

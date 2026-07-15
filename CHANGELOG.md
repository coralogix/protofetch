# Changelog
All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.21](https://github.com/coralogix/protofetch/compare/v0.1.19...v0.1.21) - 2026-07-15

### Added

- CLI backend in addition to libgit2 ([#191](https://github.com/coralogix/protofetch/pull/191))
- support targeted lock updates ([#233](https://github.com/coralogix/protofetch/pull/233))

### Other

- Release 0.1.21
- Expand git2 version range to include 0.21.x ([#236](https://github.com/coralogix/protofetch/pull/236))
- Restore using libgit2 backend by default ([#235](https://github.com/coralogix/protofetch/pull/235))
- release v0.1.20 ([#234](https://github.com/coralogix/protofetch/pull/234))
- update flake.lock ([#231](https://github.com/coralogix/protofetch/pull/231))
- *(ci)* sign flake lock update commits ([#232](https://github.com/coralogix/protofetch/pull/232))
- update flack.lock automatically ([#228](https://github.com/coralogix/protofetch/pull/228))
- strip release binaries ([#230](https://github.com/coralogix/protofetch/pull/230))

## [0.1.20](https://github.com/coralogix/protofetch/compare/v0.1.19...v0.1.20) - 2026-07-14

### Added

- CLI backend in addition to libgit2 ([#191](https://github.com/coralogix/protofetch/pull/191))
- support targeted lock updates ([#233](https://github.com/coralogix/protofetch/pull/233))

### Other

- update flake.lock ([#231](https://github.com/coralogix/protofetch/pull/231))
- *(ci)* sign flake lock update commits ([#232](https://github.com/coralogix/protofetch/pull/232))
- update flack.lock automatically ([#228](https://github.com/coralogix/protofetch/pull/228))
- strip release binaries ([#230](https://github.com/coralogix/protofetch/pull/230))

## [0.1.19](https://github.com/coralogix/protofetch/compare/v0.1.18...v0.1.19) - 2026-06-19

### Other

- Rewrite proto copying ([#226](https://github.com/coralogix/protofetch/pull/226))
- Use rayon for dependency fetches ([#225](https://github.com/coralogix/protofetch/pull/225))
- Fully remove the old resolve implementation ([#224](https://github.com/coralogix/protofetch/pull/224))
- Rewrite module resolution ([#223](https://github.com/coralogix/protofetch/pull/223))
- Use rayon for proto copy parallelism ([#222](https://github.com/coralogix/protofetch/pull/222))
- Make it explicit that we copy single proto source at a time ([#221](https://github.com/coralogix/protofetch/pull/221))
- Extract e2e test fixtures into files ([#220](https://github.com/coralogix/protofetch/pull/220))
- Documentation and tests ([#219](https://github.com/coralogix/protofetch/pull/219))
- Content root fixes ([#218](https://github.com/coralogix/protofetch/pull/218))

## [0.1.18](https://github.com/coralogix/protofetch/compare/v0.1.17...v0.1.18) - 2026-06-05

### Fixed

- merge allow/deny policies across dependencies ([#210](https://github.com/coralogix/protofetch/pull/210))
- scope worktrees per original repo paths ([#207](https://github.com/coralogix/protofetch/pull/207))

### Other

- add e2e tests ([#209](https://github.com/coralogix/protofetch/pull/209))
- replace config crate with toml + explicit env reading ([#211](https://github.com/coralogix/protofetch/pull/211))
- *(ci)* update remaining GitHub actions using Node.js 20 ([#208](https://github.com/coralogix/protofetch/pull/208))
- Gate real publishes on a full dry-run pass of every target ([#206](https://github.com/coralogix/protofetch/pull/206))
- Add workflow-level id-token: write to ci.yml + npm-side filename fix needed ([#205](https://github.com/coralogix/protofetch/pull/205))
- update toml and other dependencies ([#201](https://github.com/coralogix/protofetch/pull/201))

## [0.1.17](https://github.com/coralogix/protofetch/compare/v0.1.16...v0.1.17) - 2026-05-22

### Other

- Update rust toolchain to 1.95.0 ([#200](https://github.com/coralogix/protofetch/pull/200))

## [0.1.16](https://github.com/coralogix/protofetch/compare/v0.1.15...v0.1.16) - 2026-05-21

### Added

- parallelize resolve / fetch / copy (CX-40150) ([#194](https://github.com/coralogix/protofetch/pull/194))
- *(build)* omit dependencies if building as lib ([#197](https://github.com/coralogix/protofetch/pull/197))

### Fixed

- Choose the longest matching content root ([#184](https://github.com/coralogix/protofetch/pull/184))

### Other

- Add per-PR preview releases for @coralogix/protofetch ([#196](https://github.com/coralogix/protofetch/pull/196))
- Update release workflow with OIDC trusted publishing ([#195](https://github.com/coralogix/protofetch/pull/195))
- Bump rand from 0.8.5 to 0.8.6 ([#193](https://github.com/coralogix/protofetch/pull/193))
- Bump git2 from 0.20.2 to 0.20.4 ([#189](https://github.com/coralogix/protofetch/pull/189))
- Keep directory tree traversal in one place ([#188](https://github.com/coralogix/protofetch/pull/188))
- Replace macos-13 github runner with macos-15 ([#186](https://github.com/coralogix/protofetch/pull/186))
- Bump rsa from 0.9.7 to 0.9.10 ([#187](https://github.com/coralogix/protofetch/pull/187))
- Add CODEOWNERS ([#185](https://github.com/coralogix/protofetch/pull/185))
- Fix broken examples with * ([#182](https://github.com/coralogix/protofetch/pull/182))
- Clarify * behavior in `allow_policies` and `deny_policies` ([#181](https://github.com/coralogix/protofetch/pull/181))
- Remove protobuf system deps from transitive dependencies and fix worktree creation logging ([#180](https://github.com/coralogix/protofetch/pull/180))

## [0.1.15](https://github.com/coralogix/protofetch/compare/v0.1.14...v0.1.15) - 2025-11-17

### Other

- Do not try same credentials over and over ([#178](https://github.com/coralogix/protofetch/pull/178))

## [0.1.14](https://github.com/coralogix/protofetch/compare/v0.1.13...v0.1.14) - 2025-11-10

### Other

- Add regex file filtering policy ([#169](https://github.com/coralogix/protofetch/pull/169))

## [0.1.13](https://github.com/coralogix/protofetch/compare/v0.1.12...v0.1.13) - 2025-10-29

### Added

- rename npm package to @coralogix/protofetch (#172)

### Other

- Replace simple-binary-install with custom implementation for pnpm compatibility (#170)

## [0.1.12](https://github.com/coralogix/protofetch/compare/v0.1.11...v0.1.12) - 2025-10-21

### Other

- Preserve dependencies order ([#166](https://github.com/coralogix/protofetch/pull/166))
- Delete apple_sdk.frameworks.Security ([#165](https://github.com/coralogix/protofetch/pull/165))
- Add to the readme an example of subtree checkout ([#164](https://github.com/coralogix/protofetch/pull/164))
- Include content_roots in README ([#163](https://github.com/coralogix/protofetch/pull/163))
- Upgrade dependencies and tooling ([#160](https://github.com/coralogix/protofetch/pull/160))
- fix typo (#159)

## [0.1.11](https://github.com/coralogix/protofetch/compare/v0.1.10...v0.1.11) - 2025-02-27

### Other

- Fix cache directory lock ([#157](https://github.com/coralogix/protofetch/pull/157))

## [0.1.10](https://github.com/coralogix/protofetch/compare/v0.1.9...v0.1.10) - 2025-02-25

### Other
- Fix packages not being attached to the release ([#155](https://github.com/coralogix/protofetch/pull/155))

## [0.1.9](https://github.com/coralogix/protofetch/compare/v0.1.8...v0.1.9) - 2025-02-24

### Other
- Update dependencies ([#152](https://github.com/coralogix/protofetch/pull/152))
- Update upload/download artifact actions ([#153](https://github.com/coralogix/protofetch/pull/153))

## [0.1.8](https://github.com/coralogix/protofetch/compare/v0.1.7...v0.1.8) - 2024-08-16

### Other
- Use more robust cache locking ([#150](https://github.com/coralogix/protofetch/pull/150))
- Fix fetching when no branch is specified ([#148](https://github.com/coralogix/protofetch/pull/148))

## [0.1.7](https://github.com/coralogix/protofetch/compare/v0.1.6...v0.1.7) - 2024-07-29

### Other
- Fix nix flake build and check this on CI ([#145](https://github.com/coralogix/protofetch/pull/145))
- Update dependencies ([#144](https://github.com/coralogix/protofetch/pull/144))

## [0.1.6](https://github.com/coralogix/protofetch/compare/v0.1.5...v0.1.6) - 2024-07-02

### Other
- Fetch optimizations ([#142](https://github.com/coralogix/protofetch/pull/142))

## [0.1.5](https://github.com/coralogix/protofetch/compare/v0.1.4...v0.1.5) - 2024-06-27

### Other
- Cache lock improvements ([#140](https://github.com/coralogix/protofetch/pull/140))

## [0.1.4](https://github.com/coralogix/protofetch/compare/v0.1.3...v0.1.4) - 2024-05-22

### Other
- Prevent concurrent cache access ([#135](https://github.com/coralogix/protofetch/pull/135))

# Changelog

All notable changes to this project will be documented in this file.

## [0.1.2] - 2023-11-13

### Bug Fixes

- Allow clippy::needless_raw_string_hashes,
- Update rust crate aho-corasick to 1.1.1 ([#23](https://github.com/rustic-rs/rustic_core/issues/23))
- Update rust crate rayon to 1.8.0 ([#24](https://github.com/rustic-rs/rustic_core/issues/24))
- Update rust crate binrw to 0.13.0 ([#25](https://github.com/rustic-rs/rustic_core/issues/25))
- Update rust crate aho-corasick to 1.1.2 ([#36](https://github.com/rustic-rs/rustic_core/issues/36))
- Update rust crate clap to 4.4.7 ([#37](https://github.com/rustic-rs/rustic_core/issues/37))
- Update rust crate reqwest to 0.11.22 ([#38](https://github.com/rustic-rs/rustic_core/issues/38))
- Update rust crate serde_json to 1.0.108 ([#39](https://github.com/rustic-rs/rustic_core/issues/39))
- Update rust crate thiserror to 1.0.50 ([#40](https://github.com/rustic-rs/rustic_core/issues/40))
- Update rust crate enum-map to 2.7.0 ([#43](https://github.com/rustic-rs/rustic_core/issues/43))
- Update serde monorepo to 1.0.190 ([#41](https://github.com/rustic-rs/rustic_core/issues/41))
- Update rust crate cached to 0.46.0 ([#42](https://github.com/rustic-rs/rustic_core/issues/42))
- Update rust crate serde_with to 3.4.0 ([#44](https://github.com/rustic-rs/rustic_core/issues/44))
- Update rust crate zstd to 0.13.0 ([#45](https://github.com/rustic-rs/rustic_core/issues/45))
- Update rust crate binrw to 0.13.1 ([#46](https://github.com/rustic-rs/rustic_core/issues/46))
- Update rust crate cached to 0.46.1 ([#47](https://github.com/rustic-rs/rustic_core/issues/47))
- Update rust crate enum-map to 2.7.1 ([#49](https://github.com/rustic-rs/rustic_core/issues/49))
- Update serde monorepo to 1.0.192 ([#50](https://github.com/rustic-rs/rustic_core/issues/50))
- Update rust crate enum-map-derive to 0.15.0 ([#51](https://github.com/rustic-rs/rustic_core/issues/51))
- Update rust crate clap to 4.4.8 ([#52](https://github.com/rustic-rs/rustic_core/issues/52))
- Update rust crate aes256ctr_poly1305aes to 0.2.0 ([#54](https://github.com/rustic-rs/rustic_core/issues/54))
- Temporarily allow unused import for `cached` proc macro to fix lint warning when not on *nix systems

### Documentation

- Fix version in readme as well
- Change contributing headline
- Remove outdated information from lib.rs and Readme about features

### Miscellaneous Tasks

- Initial commit :rocket:
- Add lockfile and reset version
- Add documentation link
- Add public api check to releases
- Add cross ci check
- Fix mistakenly commented out ubuntu test and comment out mac-os
- Add workflow to update public api fixtures
- Rename workflow
- Update target_os in public api check
- Push changes to pr branch
- Push fixtures when test fails (means new fixtures have been generated)
- Remove cargo lock
- Remove lockfile maintenance from renovate
- Generate link to definition
- Update actions hashes
- Push fixtures without ifs
- Run public-api check also on macos
- Update msrv
- Add os to commit for fixtures
- Remove category due to limit == 5 on crates.io
- Remove binary postfix leftover
- Fix some typos ([#20](https://github.com/rustic-rs/rustic_core/issues/20))
- Fix postprocessing repository url in cliff.toml
- Update cross ci
- Rename cross ci
- Add careful tests
- Add msrv check
- Add feature powerset check
- Rename step
- Add powerset beta check
- Use matrix for toolchain
- Make more use of toolchains
- Add miri test
- Add miri setup step to keep output clean
- Warn on miri isolation error
- Set `-Zmiri-disable-isolation`
- Don't run Miri for now due to: https://github.com/rust-lang/miri/issues/3066
- Patch sha2 for miri
- Remove wrong sed flag
- Fix sed call
- Add x86_64-pc-windows-gnu to cross-ci
- Add -- --nocapture to testharness for extensive output for miri
- Don't let miri matrix fail fast
- Split long-running careful tests and CI
- Use results for workflows to check for outcome more easily
- Remove doubling workflows from renovate PR und Push
- Compile dependencies with optimizations in dev mode
- Update dprint plugins

### Restore

- Add caching for user/group names ([#33](https://github.com/rustic-rs/rustic_core/issues/33))


## [0.1.1] - 2023-09-18

### Bug Fixes

- Correct glob-matching for relative paths
  ([#783](https://github.com/rustic-rs/rustic/issues/783))

### Documentation

- Update Readme layout, move docs to separate repository, add rustic_core
  Readme.md ([#820](https://github.com/rustic-rs/rustic/issues/820))
- Add rustic_core logo
- Set subtitle for rustic_core readme
- Fix item links in documentation for rustic_core
- Pass "--document-private-items" to rustdoc via metadata in manifest

### Features

- Option to disable requiring git repository for git-ignore rules
- Wait for password-command to exit
- Add `--json` option to `forget` command
  ([#806](https://github.com/rustic-rs/rustic/issues/806))

### Miscellaneous Tasks

- Lint markdown with dprint, run initial dprint fmt
  ([#830](https://github.com/rustic-rs/rustic/issues/830))
- Lint has been removed
- Add cliff.toml and generate rustic_core changelog
- Add documentation field to rustic_core manifest
- Relink to new image location

### Refactor

- Replace `nom` with `shellwords` to split strings
  ([#752](https://github.com/rustic-rs/rustic/issues/752))
- Add metadata to crate manifests
  ([#822](https://github.com/rustic-rs/rustic/issues/822))

### Build

- Bump public-api from 0.29.1 to 0.31.2
  ([#695](https://github.com/rustic-rs/rustic/issues/695))
- Bump public-api from 0.31.2 to 0.31.3
  ([#796](https://github.com/rustic-rs/rustic/issues/796))
- Bump rustdoc-json from 0.8.6 to 0.8.7
  ([#794](https://github.com/rustic-rs/rustic/issues/794))

### Prune

- Add example using rustic_core
- Don't abort if time is unset for pack-to-delete

### Repoinfo

- Add options --json, --only-files, --only-index

### Rest/rclone

- Make # of retries cusomizable and use sensible default

### Restore

- Download multiple contiguous blobs in one request

### Rustic_core

- Add NoProgress and NoProgressBars (e.g. for examples)

## [0.1.0] - 2023-08-11

- Initial refactoring out of rustic_core from rustic-rs

<!-- generated by git-cliff -->

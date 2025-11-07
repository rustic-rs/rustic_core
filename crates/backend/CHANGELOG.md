# Changelog

All notable changes to this project will be documented in this file.

## [0.5.3](https://github.com/rustic-rs/rustic_core/compare/rustic_backend-v0.5.2...rustic_backend-v0.5.3) - 2025-09-12

### Fixed

- fix clippy lints ([#407](https://github.com/rustic-rs/rustic_core/pull/407))
- *(deps)* lock file maintenance rust dependencies (#389)

### Other

- update dependencies ([#428](https://github.com/rustic-rs/rustic_core/pull/428))
- update to 2024 edition and fix clippy lints (#399)
- update dependencies and fix clippy lints / remove opendal::ftp support (#405)
- Update opendal to 0.51.0 (#391)

## [0.5.2](https://github.com/rustic-rs/rustic_core/compare/rustic_backend-v0.5.1...rustic_backend-v0.5.2) - 2024-11-27

### Fixed

- *(backend)* temporarily remove ftp service to fix build in ci/cd ([#371](https://github.com/rustic-rs/rustic_core/pull/371))

### Other

- *(deps)* update dependencies
- add note about temporarily disabling ftp feature in opendal backend due to build issues
- update README badges for rustic_core and rustic_backend to include MSRV

## [0.5.1](https://github.com/rustic-rs/rustic_core/compare/rustic_backend-v0.5.0...rustic_backend-v0.5.1) - 2024-11-24

### Other

- Revert "feat(async): add `async_compatible` methods to identify backend compatibility ([#355](https://github.com/rustic-rs/rustic_core/pull/355))"

## [0.5.0](https://github.com/rustic-rs/rustic_core/compare/rustic_backend-v0.4.2...rustic_backend-v0.5.0) - 2024-11-18

### Added

- *(async)* add `async_compatible` methods to identify backend compatibility ([#355](https://github.com/rustic-rs/rustic_core/pull/355))
- add 'yandex-disk' to enabled opendal services and update opendal to 0.50.2 ([#360](https://github.com/rustic-rs/rustic_core/pull/360))

### Other

- *(error)* enhance error logging and output formatting ([#361](https://github.com/rustic-rs/rustic_core/pull/361))
- *(backend)* simplify code in local backend ([#362](https://github.com/rustic-rs/rustic_core/pull/362))
- *(backend)* migrate from `backoff` to `backon` ([#356](https://github.com/rustic-rs/rustic_core/pull/356))
- *(error)* improve error messages and file handling ([#334](https://github.com/rustic-rs/rustic_core/pull/334))
- *(deps)* lock file maintenance rust dependencies ([#345](https://github.com/rustic-rs/rustic_core/pull/345))
- *(deps)* [**breaking**] upgrade to new conflate version ([#300](https://github.com/rustic-rs/rustic_core/pull/300))
- *(errors)* [**breaking**] Improve error handling, display and clean up codebase ([#321](https://github.com/rustic-rs/rustic_core/pull/321))

## [0.4.2](https://github.com/rustic-rs/rustic_core/compare/rustic_backend-v0.4.1...rustic_backend-v0.4.2) - 2024-10-24

### Fixed

- fix opendal paths on windows ([#340](https://github.com/rustic-rs/rustic_core/pull/340))

## [0.4.1](https://github.com/rustic-rs/rustic_core/compare/rustic_backend-v0.4.0...rustic_backend-v0.4.1) - 2024-10-03

### Fixed

- *(docs)* left over from migration to `conflate` crate ([#296](https://github.com/rustic-rs/rustic_core/pull/296))

## [0.4.0](https://github.com/rustic-rs/rustic_core/compare/rustic_backend-v0.3.0...rustic_backend-v0.4.0) - 2024-10-02

### Fixed

- Add #[non_exhaustive] to pub structs which may be extended in future ([#293](https://github.com/rustic-rs/rustic_core/pull/293))
- *(backend)* [**breaking**] Use correct merge strategy for repository options ([#291](https://github.com/rustic-rs/rustic_core/pull/291))

### Other

- *(deps)* update dependencies ([#292](https://github.com/rustic-rs/rustic_core/pull/292))
- *(deps)* use conflate instead of merge crate ([#284](https://github.com/rustic-rs/rustic_core/pull/284))

## [0.3.0](https://github.com/rustic-rs/rustic_core/compare/rustic_backend-v0.2.1...rustic_backend-v0.3.0) - 2024-09-23

### Added

- [**breaking**] Use CommandInput for commands ([#269](https://github.com/rustic-rs/rustic_core/pull/269))

### Other

- remove readme versions in usage section for easier release due to release PR ([#271](https://github.com/rustic-rs/rustic_core/pull/271))
- [**breaking**] Use different Id types ([#256](https://github.com/rustic-rs/rustic_core/pull/256))
- *(deps)* Update opendal ([#268](https://github.com/rustic-rs/rustic_core/pull/268))

## [0.2.1](https://github.com/rustic-rs/rustic_core/compare/rustic_backend-v0.2.0...rustic_backend-v0.2.1) - 2024-09-06

### Added
- Add autocompletion hints  ([#257](https://github.com/rustic-rs/rustic_core/pull/257))

### Fixed
- Re-add missing opendal services ([#249](https://github.com/rustic-rs/rustic_core/pull/249))

### Other
- Revert "backend: specify core version"

## [0.2.0](https://github.com/rustic-rs/rustic_core/compare/rustic_backend-v0.1.1...rustic_backend-v0.2.0) - 2024-08-18

### Added
- *(backends)* Add throttle option to opendal backend ([#216](https://github.com/rustic-rs/rustic_core/pull/216))
- *(backend)* [**breaking**] remove s3 and sftp wrapper around opendal ([#200](https://github.com/rustic-rs/rustic_core/pull/200))
- [**breaking**] move clippy lints to cargo manifest and fix upcoming issues all over the workspace ([#176](https://github.com/rustic-rs/rustic_core/pull/176))
- *(opendal)* Add option connections ([#155](https://github.com/rustic-rs/rustic_core/pull/155))

### Fixed
- clippy lints ([#220](https://github.com/rustic-rs/rustic_core/pull/220))
- *(backends)* local: Only create repo dir when creating the repository ([#206](https://github.com/rustic-rs/rustic_core/pull/206))
- *(deps)* update rust crate reqwest to 0.11.26 ([#196](https://github.com/rustic-rs/rustic_core/pull/196))
- *(deps)* update rust crate thiserror to 1.0.58 ([#192](https://github.com/rustic-rs/rustic_core/pull/192))
- *(deps)* update rust crate anyhow to 1.0.81 ([#191](https://github.com/rustic-rs/rustic_core/pull/191))
- *(rclone)* Use semver for version checking ([#188](https://github.com/rustic-rs/rustic_core/pull/188))
- *(deps)* update rust crate clap to 4.5.2 ([#183](https://github.com/rustic-rs/rustic_core/pull/183))
- *(config)* Merge repository options for multiple config sources ([#171](https://github.com/rustic-rs/rustic_core/pull/171))
- *(backend)* Give useful error message when no repository is given.
- updated msrv and fix clippy lints ([#160](https://github.com/rustic-rs/rustic_core/pull/160))

### Other
- dependency updates
- *(deps)* more version updates ([#237](https://github.com/rustic-rs/rustic_core/pull/237))
- Update MSRV to 1.76.0
- *(deps)* Several version updates ([#234](https://github.com/rustic-rs/rustic_core/pull/234))
- Update MSRV (needed by opendal)
- update opendal to 0.46 and refactor accordingly ([#225](https://github.com/rustic-rs/rustic_core/pull/225))
- Update MSRV to 1.73.0
- fix clippy lints
- *(deps)* update dependencies ([#180](https://github.com/rustic-rs/rustic_core/pull/180))
- add rustic_testing to workspace crates
- reset again after release to workspace dependencies for workspace crates

## [rustic_backend-v0.1.1] - 2024-02-02

### Documentation

- Fix c&p for SftpBackend
- Update examples and other minor things

### Features

- Add options rclone-command, use-password, rest-url to rclone backend
  ([#139](https://github.com/rustic-rs/rustic_core/issues/139))

### Miscellaneous Tasks

- Add rustic_backend to release-pr workflow

### Backend

- Add sftp backend ([#126](https://github.com/rustic-rs/rustic_core/issues/126))

## [rustic_backend-v0.1.0] - 2023-08-10

- Reserving name on crates.io

<!-- generated by git-cliff -->

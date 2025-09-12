# Changelog

All notable changes to this project will be documented in this file.

## [0.8.0](https://github.com/rustic-rs/rustic_core/compare/rustic_core-v0.7.3...rustic_core-v0.8.0) - 2025-09-12

### Added

- Add fixed size chunking and allow fine-tune of rabin chunking (#422)
- Add env argument to the run function of command input (#420)
- Show processed file when chunker gives an error (#421)
- snapshots can be identified with latest~N (N >= 0) (#416)
- expose Tree::serialize method
- *(repository)* add progress_bars method
- *(repository)* Add find_ids and stream_files_list (#411)
- *(commands)* Add delete_unchanged option to forget (#386)
- *(commands)* [**breaking**] rename backup skip_identical_parent to skip_if_unchanged (#387)
- *(warmup)* [**breaking**] Add warmup wait command (#379)

### Fixed

- Make example for time format even more explicit (#425)
- Add example for time format (#424)
- fix clippy lints ([#423](https://github.com/rustic-rs/rustic_core/pull/423))
- [**breaking**] Allow to unset append-only mode (#414)
- improve handling of u32 conversions (#412)
- *(repository)* use KeyId in delete_key() (#410)
- Fix repair index (#406)
- Allow to request identical snapshot multiple times (#408)
- fix clippy lints ([#407](https://github.com/rustic-rs/rustic_core/pull/407))
- fix clippy lints
- [**breaking**] Don't panic when reading empty files (#381)

### Other

- update dependencies ([#428](https://github.com/rustic-rs/rustic_core/pull/428))
- Add SnapshotFile::from_strs() to search for multiple snapshots ([#419](https://github.com/rustic-rs/rustic_core/pull/419))
- *(repository)* [**breaking**] Add more control over used keys (#383)
- update to 2024 edition and fix clippy lints (#399)
- update dependencies and fix clippy lints / remove opendal::ftp support (#405)

## [0.7.3](https://github.com/rustic-rs/rustic_core/compare/rustic_core-v0.7.2...rustic_core-v0.7.3) - 2024-12-06

### Fixed

- *(chunker)* Don't underflow with wrong size_hint ([#378](https://github.com/rustic-rs/rustic_core/pull/378))

## [0.7.2](https://github.com/rustic-rs/rustic_core/compare/rustic_core-v0.7.1...rustic_core-v0.7.2) - 2024-11-30

### Added

- Add a "minutely" timeline ([#374](https://github.com/rustic-rs/rustic_core/pull/374))

### Fixed

- clippy lints

## [0.7.1](https://github.com/rustic-rs/rustic_core/compare/rustic_core-v0.7.0...rustic_core-v0.7.1) - 2024-11-27

### Other

- *(deps)* update dependencies
- update README badges for rustic_core and rustic_backend to include MSRV

## [0.7.0](https://github.com/rustic-rs/rustic_core/compare/rustic_core-v0.6.1...rustic_core-v0.7.0) - 2024-11-24

### Other

- remove webdav feature ([#366](https://github.com/rustic-rs/rustic_core/pull/366))
- Revert "feat(async): add `async_compatible` methods to identify backend compatibility ([#355](https://github.com/rustic-rs/rustic_core/pull/355))"

## [0.6.1](https://github.com/rustic-rs/rustic_core/compare/rustic_core-v0.6.0...rustic_core-v0.6.1) - 2024-11-19

### Added

- make FilePolicy usable in cli and config

### Fixed

- *(error)* add missing context to error in cache backend

## [0.6.0](https://github.com/rustic-rs/rustic_core/compare/rustic_core-v0.5.5...rustic_core-v0.6.0) - 2024-11-18

### Added

- *(async)* add `async_compatible` methods to identify backend compatibility ([#355](https://github.com/rustic-rs/rustic_core/pull/355))

### Fixed

- prevent overwriting hot repository on init ([#353](https://github.com/rustic-rs/rustic_core/pull/353))

### Other

- *(error)* enhance error logging and output formatting ([#361](https://github.com/rustic-rs/rustic_core/pull/361))
- *(deps)* remove Derivative and replace with Default impl due to RUSTSEC-2024-0388 ([#359](https://github.com/rustic-rs/rustic_core/pull/359))
- *(error)* improve error messages and file handling ([#334](https://github.com/rustic-rs/rustic_core/pull/334))
- *(deps)* lock file maintenance rust dependencies ([#345](https://github.com/rustic-rs/rustic_core/pull/345))
- *(deps)* remove cdc and switch to rustic_cdc ([#348](https://github.com/rustic-rs/rustic_core/pull/348))
- *(deps)* [**breaking**] upgrade to new conflate version ([#300](https://github.com/rustic-rs/rustic_core/pull/300))
- *(errors)* [**breaking**] Improve error handling, display and clean up codebase ([#321](https://github.com/rustic-rs/rustic_core/pull/321))

## [0.5.5](https://github.com/rustic-rs/rustic_core/compare/rustic_core-v0.5.4...rustic_core-v0.5.5) - 2024-10-25

### Fixed

- *(errors)* forwarding Display impl should work again ([#342](https://github.com/rustic-rs/rustic_core/pull/342))

## [0.5.4](https://github.com/rustic-rs/rustic_core/compare/rustic_core-v0.5.3...rustic_core-v0.5.4) - 2024-10-24

### Added

- *(commands)* Add convenient names for read-data-subset n/m ([#328](https://github.com/rustic-rs/rustic_core/pull/328))

### Fixed

- OpenFile::read_at no longer errors on invalid offset or length ([#331](https://github.com/rustic-rs/rustic_core/pull/331))

### Other

- *(deps)* update actions ([#338](https://github.com/rustic-rs/rustic_core/pull/338))

## [0.5.3](https://github.com/rustic-rs/rustic_core/compare/rustic_core-v0.5.2...rustic_core-v0.5.3) - 2024-10-10

### Other

- Revert "refactor(errors): improve message and add logging when sending tree from backend panics" ([#325](https://github.com/rustic-rs/rustic_core/pull/325))

## [0.5.2](https://github.com/rustic-rs/rustic_core/compare/rustic_core-v0.5.1...rustic_core-v0.5.2) - 2024-10-09

### Fixed

- *(errors)* use correct error variant for data encryption ([#323](https://github.com/rustic-rs/rustic_core/pull/323))
- *(errors)* handle out of bounds access in PathList display ([#313](https://github.com/rustic-rs/rustic_core/pull/313))
- *(errors)* better error message for hot/cold repo in check ([#297](https://github.com/rustic-rs/rustic_core/pull/297))

### Other

- *(commands)* decouple logic from option structs for check, prune, repair, key, and restore ([#317](https://github.com/rustic-rs/rustic_core/pull/317))
- *(errors)* improve message and add logging when sending tree from backend panics ([#314](https://github.com/rustic-rs/rustic_core/pull/314))

## [0.5.1](https://github.com/rustic-rs/rustic_core/compare/rustic_core-v0.5.0...rustic_core-v0.5.1) - 2024-10-03

### Fixed

- fix check without --read-data ([#299](https://github.com/rustic-rs/rustic_core/pull/299))
- *(docs)* left over from migration to `conflate` crate ([#296](https://github.com/rustic-rs/rustic_core/pull/296))

## [0.5.0](https://github.com/rustic-rs/rustic_core/compare/rustic_core-v0.4.0...rustic_core-v0.5.0) - 2024-10-02

### Added

- Add read-data-subset to CheckOptions; allow to check given trees ([#262](https://github.com/rustic-rs/rustic_core/pull/262))
- Add Repository method to update snapshot collections ([#260](https://github.com/rustic-rs/rustic_core/pull/260))

### Fixed

- Add #[non_exhaustive] to pub structs which may be extended in future ([#293](https://github.com/rustic-rs/rustic_core/pull/293))
- Don't query the default cache directory when a custom one is set ([#285](https://github.com/rustic-rs/rustic_core/pull/285))

### Other

- *(deps)* update dependencies ([#292](https://github.com/rustic-rs/rustic_core/pull/292))
- *(deps)* use conflate instead of merge crate ([#284](https://github.com/rustic-rs/rustic_core/pull/284))

## [0.4.0](https://github.com/rustic-rs/rustic_core/compare/rustic_core-v0.3.1...rustic_core-v0.4.0) - 2024-09-23

### Added

- make IndexPack::pack_size() public
- Add Repository::drop_index and ::drop_data_from_index ([#166](https://github.com/rustic-rs/rustic_core/pull/166))
- *(commands)* Add option stdin_command to be used in CLI and config file ([#266](https://github.com/rustic-rs/rustic_core/pull/266))
- [**breaking**] Use CommandInput for commands ([#269](https://github.com/rustic-rs/rustic_core/pull/269))
- Add CommandInput ([#252](https://github.com/rustic-rs/rustic_core/pull/252))

### Fixed

- de/serialize tags as DisplayFromStr ([#270](https://github.com/rustic-rs/rustic_core/pull/270))
- [**breaking**] use plural names for options ([#267](https://github.com/rustic-rs/rustic_core/pull/267))
- fix clippy lint
- *(test)* shorten snapshot names for windows environment
- [**breaking**] improve password-command error reporting ([#265](https://github.com/rustic-rs/rustic_core/pull/265))
- properly finish progress bar in Repository::get_snapshot_group ([#263](https://github.com/rustic-rs/rustic_core/pull/263))

### Other

- remove readme versions in usage section for easier release due to release PR ([#271](https://github.com/rustic-rs/rustic_core/pull/271))
- [**breaking**] Use different Id types ([#256](https://github.com/rustic-rs/rustic_core/pull/256))
- Use serde_with::skip_serializing_none instead of manual mapping ([#251](https://github.com/rustic-rs/rustic_core/pull/251))

## [0.3.1](https://github.com/rustic-rs/rustic_core/compare/rustic_core-v0.3.0...rustic_core-v0.3.1) - 2024-09-06

### Added

- Add autocompletion hints  ([#257](https://github.com/rustic-rs/rustic_core/pull/257))

### Fixed

- don't give invalid password error for other keyfile errors ([#247](https://github.com/rustic-rs/rustic_core/pull/247))
- adjust tests to new Rust version ([#259](https://github.com/rustic-rs/rustic_core/pull/259))
- fix FromStr for SnapshotGroupCriterion ([#254](https://github.com/rustic-rs/rustic_core/pull/254))
- make more Indexed traits public ([#253](https://github.com/rustic-rs/rustic_core/pull/253))
- fix StringList::contains_all ([#246](https://github.com/rustic-rs/rustic_core/pull/246))
- *(build)* unbreak building on OpenBSD ([#245](https://github.com/rustic-rs/rustic_core/pull/245))

## [0.3.0](https://github.com/rustic-rs/rustic_core/compare/rustic_core-v0.2.0...rustic_core-v0.3.0) - 2024-08-18

### Added

- *(forget)* [**breaking**] Make keep-* Options and add keep-none ([#238](https://github.com/rustic-rs/rustic_core/pull/238))
- add search methods to Repository ([#212](https://github.com/rustic-rs/rustic_core/pull/212))
- [**breaking**] Allow specifying many options in config profile without array ([#211](https://github.com/rustic-rs/rustic_core/pull/211))
- [**breaking**] move clippy lints to cargo manifest and fix upcoming issues all over the workspace ([#176](https://github.com/rustic-rs/rustic_core/pull/176))
- Add extra check before writing data ([#154](https://github.com/rustic-rs/rustic_core/pull/154))
- Allow missing fields in snapshot summary
- Hide plain text password from help text ([#170](https://github.com/rustic-rs/rustic_core/pull/170))
- Add Repository::to_indexed_checked and ::to_index_ids_checked() ([#168](https://github.com/rustic-rs/rustic_core/pull/168))
- *(prune)* Add more debug info to stats ([#162](https://github.com/rustic-rs/rustic_core/pull/162))
- Add append-only repository mode ([#164](https://github.com/rustic-rs/rustic_core/pull/164))

### Fixed

- parse commands given by arg or env using shell_words ([#240](https://github.com/rustic-rs/rustic_core/pull/240))
- Allow non-value/null xattr ([#235](https://github.com/rustic-rs/rustic_core/pull/235))
- ensure Rust 1.76.0 compiles
- backup file even if failed listing extended attributes ([#233](https://github.com/rustic-rs/rustic_core/pull/233))
- Export types so the Repository type can be fully specified ([#229](https://github.com/rustic-rs/rustic_core/pull/229))
- Always sort StringList ([#226](https://github.com/rustic-rs/rustic_core/pull/226))
- *(commands)* Properly finish progress bars
- *(commands)* [**breaking**] Fix edge case for repair index ([#219](https://github.com/rustic-rs/rustic_core/pull/219))
- clippy lints ([#220](https://github.com/rustic-rs/rustic_core/pull/220))
- *(errors)* Show filenames in error message coming from ignore source ([#215](https://github.com/rustic-rs/rustic_core/pull/215))
- *(paths)* Handle paths starting with "." correctly ([#213](https://github.com/rustic-rs/rustic_core/pull/213))
- Add warning about unsorted files and sort where necessary ([#205](https://github.com/rustic-rs/rustic_core/pull/205))
- *(deps)* update rust crate thiserror to 1.0.58 ([#192](https://github.com/rustic-rs/rustic_core/pull/192))
- *(deps)* update rust crate anyhow to 1.0.81 ([#191](https://github.com/rustic-rs/rustic_core/pull/191))
- *(deps)* update rust crate serde_with to 3.7.0 ([#189](https://github.com/rustic-rs/rustic_core/pull/189))
- *(rclone)* Use semver for version checking ([#188](https://github.com/rustic-rs/rustic_core/pull/188))
- *(deps)* update rust crate strum to 0.26.2 ([#187](https://github.com/rustic-rs/rustic_core/pull/187))
- *(deps)* update rust crate clap to 4.5.2 ([#183](https://github.com/rustic-rs/rustic_core/pull/183))
- Set correct content for symlink with parent snapshot ([#174](https://github.com/rustic-rs/rustic_core/pull/174))
- update dependency nix ([#169](https://github.com/rustic-rs/rustic_core/pull/169))
- *(memory)* Limit memory usage for restore when having large pack files ([#165](https://github.com/rustic-rs/rustic_core/pull/165))
- *(prune)* Correct number of repacks ([#167](https://github.com/rustic-rs/rustic_core/pull/167))
- updated msrv and fix clippy lints ([#160](https://github.com/rustic-rs/rustic_core/pull/160))

### Other

- dependency updates
- Ensure that MSRV 1.76 works
- *(deps)* more version updates ([#237](https://github.com/rustic-rs/rustic_core/pull/237))
- Update MSRV to 1.76.0
- *(deps)* Several version updates ([#234](https://github.com/rustic-rs/rustic_core/pull/234))
- fix clippy lints ([#236](https://github.com/rustic-rs/rustic_core/pull/236))
- Update MSRV (needed by opendal)
- update sha2 dependency
- add integration tests for `prune` and `ls` ([#221](https://github.com/rustic-rs/rustic_core/pull/221))
- *(error)* Add error sources ([#217](https://github.com/rustic-rs/rustic_core/pull/217))
- add more warnings
- make SnapshotFile::cmp_group public ([#210](https://github.com/rustic-rs/rustic_core/pull/210))
- Update MSRV to 1.73.0
- fix clippy lints
- add backup integration tests using snapshots ([#175](https://github.com/rustic-rs/rustic_core/pull/175))
- replace dep bitmask-enum by enumset ([#173](https://github.com/rustic-rs/rustic_core/pull/173))
- *(deps)* update dependencies ([#180](https://github.com/rustic-rs/rustic_core/pull/180))
- use release-plz action, remove public api fixtures incl. test and related ci and other release related ci
- Add unit tests for extra verification ([#172](https://github.com/rustic-rs/rustic_core/pull/172))
- rustic_config v0.1.0
- add rustic_testing to workspace crates

## [rustic_core-v0.2.0] - 2024-02-01

### Bug Fixes

- Update rust crate itertools to 0.12.0
  ([#57](https://github.com/rustic-rs/rustic_core/issues/57))
- Update rust crate enum-map to 2.7.2
  ([#60](https://github.com/rustic-rs/rustic_core/issues/60))
- Update rust crate enum-map-derive to 0.16.0
  ([#62](https://github.com/rustic-rs/rustic_core/issues/62))
- Update serde monorepo to 1.0.193
  ([#66](https://github.com/rustic-rs/rustic_core/issues/66))
- Update rust crate url to 2.5.0
  ([#67](https://github.com/rustic-rs/rustic_core/issues/67))
- Update rust crate enum-map-derive to 0.17.0
  ([#69](https://github.com/rustic-rs/rustic_core/issues/69))
- Update rust crate enum-map to 2.7.3
  ([#68](https://github.com/rustic-rs/rustic_core/issues/68))
- Update rust crate binrw to 0.13.2
  ([#71](https://github.com/rustic-rs/rustic_core/issues/71))
- Remove unmaintained `actions-rs` ci actions
- Update rust crate cachedir to 0.3.1
  ([#84](https://github.com/rustic-rs/rustic_core/issues/84))
- Update rust crate clap to 4.4.11
  ([#81](https://github.com/rustic-rs/rustic_core/issues/81))
- Update rust crate filetime to 0.2.23
  ([#87](https://github.com/rustic-rs/rustic_core/issues/87))
- Update rust crate serde-aux to 4.3.1
  ([#91](https://github.com/rustic-rs/rustic_core/issues/91))
- Update rust crate crossbeam-channel to 0.5.9
  ([#93](https://github.com/rustic-rs/rustic_core/issues/93))
- Update rust crate thiserror to 1.0.51
  ([#95](https://github.com/rustic-rs/rustic_core/issues/95))
- Update rust crate reqwest to 0.11.23
  ([#99](https://github.com/rustic-rs/rustic_core/issues/99))
- Update rust crate crossbeam-channel to 0.5.10
  ([#107](https://github.com/rustic-rs/rustic_core/issues/107))
- Update rust crate thiserror to 1.0.52
  ([#108](https://github.com/rustic-rs/rustic_core/issues/108))
- Don't produce error when initializing a new hot/cold repository
  ([#112](https://github.com/rustic-rs/rustic_core/issues/112))
- Add missing Serialize derive on KeepOptions
- Repair index: Don't set "to-delete" flag for newly read pack files
  ([#113](https://github.com/rustic-rs/rustic_core/issues/113))
- Update rust crate clap to 4.4.12
  ([#114](https://github.com/rustic-rs/rustic_core/issues/114))
- Update rust crate serde_json to 1.0.110
  ([#115](https://github.com/rustic-rs/rustic_core/issues/115))
- Update rust crate thiserror to 1.0.56
  ([#116](https://github.com/rustic-rs/rustic_core/issues/116))
- Update serde monorepo to 1.0.194
  ([#117](https://github.com/rustic-rs/rustic_core/issues/117))
- Update rust crate cached to 0.47.0
  ([#119](https://github.com/rustic-rs/rustic_core/issues/119))
- Update rust crate serde_json to 1.0.111
  ([#120](https://github.com/rustic-rs/rustic_core/issues/120))
- Update rust crate clap to 4.4.13
  ([#121](https://github.com/rustic-rs/rustic_core/issues/121))
- Update rust crate ignore to 0.4.22
  ([#123](https://github.com/rustic-rs/rustic_core/issues/123))
- Update serde monorepo to 1.0.195
  ([#124](https://github.com/rustic-rs/rustic_core/issues/124))
- Update rust crate serde-aux to 4.4.0
  ([#132](https://github.com/rustic-rs/rustic_core/issues/132))
- Update rust crate clap to 4.4.18
  ([#130](https://github.com/rustic-rs/rustic_core/issues/130))
- Update rust crate rayon to 1.8.1
  ([#131](https://github.com/rustic-rs/rustic_core/issues/131))
- Update rust crate opendal to 0.44.2
  ([#133](https://github.com/rustic-rs/rustic_core/issues/133))
- Update rust crate serde_with to 3.5.0
  ([#134](https://github.com/rustic-rs/rustic_core/issues/134))
- Don't abort on negative elapsed time in packer/indexer
  ([#138](https://github.com/rustic-rs/rustic_core/issues/138))
- Clippy missing backticks for item

### Documentation

- Fix c&p for SftpBackend
- Update examples and other minor things
- Update Changelog
- Update intra doc links
- Add features and fix intra-doc links

### Features

- Add `--custom-ignorefile` command line flag
  ([#74](https://github.com/rustic-rs/rustic_core/issues/74))
- Add options rclone-command, use-password, rest-url to rclone backend
  ([#139](https://github.com/rustic-rs/rustic_core/issues/139))
- Add vfs and webdav fs
  ([#106](https://github.com/rustic-rs/rustic_core/issues/106))

### Generated

- Updated Public API fixtures for linux
- Updated Public API fixtures for macos
- Updated Public API fixtures for windows

### Miscellaneous Tasks

- Run actions that need secrets.GITHUB_TOKEN only on rustic-rs org
- Set MSRV to 1.70.0
- Update dtolnay/rust-toolchain
- Update taiki-e/install-action
- Update rustsec/audit-check
- Activate automerge for github action digest update
- Release
- Add rustic_backend to release-pr workflow
- Update dependencies
- Fix directory for public api fixtures for core

### Backend

- Add sftp backend ([#126](https://github.com/rustic-rs/rustic_core/issues/126))

### Backup

- Add option to omit identical backups
  ([#56](https://github.com/rustic-rs/rustic_core/issues/56))
- Run size scanning parallel to backup; add no-scan option
  ([#97](https://github.com/rustic-rs/rustic_core/issues/97))

### Cache

- Don't write warnings if cache files don't exist
  ([#100](https://github.com/rustic-rs/rustic_core/issues/100))

### Copy

- Add better progress
  ([#94](https://github.com/rustic-rs/rustic_core/issues/94))
- Double-check for duplicate blobs
  ([#148](https://github.com/rustic-rs/rustic_core/issues/148))

### Prune

- Add option early_delete_index
  ([#63](https://github.com/rustic-rs/rustic_core/issues/63))
- Change default of max-repack to 10%
  ([#64](https://github.com/rustic-rs/rustic_core/issues/64))

## [rustic_core-v0.1.2] - 2023-11-13

### Bug Fixes

- Allow clippy::needless_raw_string_hashes,
- Update rust crate aho-corasick to 1.1.1
  ([#23](https://github.com/rustic-rs/rustic_core/issues/23))
- Update rust crate rayon to 1.8.0
  ([#24](https://github.com/rustic-rs/rustic_core/issues/24))
- Update rust crate binrw to 0.13.0
  ([#25](https://github.com/rustic-rs/rustic_core/issues/25))
- Update rust crate aho-corasick to 1.1.2
  ([#36](https://github.com/rustic-rs/rustic_core/issues/36))
- Update rust crate clap to 4.4.7
  ([#37](https://github.com/rustic-rs/rustic_core/issues/37))
- Update rust crate reqwest to 0.11.22
  ([#38](https://github.com/rustic-rs/rustic_core/issues/38))
- Update rust crate serde_json to 1.0.108
  ([#39](https://github.com/rustic-rs/rustic_core/issues/39))
- Update rust crate thiserror to 1.0.50
  ([#40](https://github.com/rustic-rs/rustic_core/issues/40))
- Update rust crate enum-map to 2.7.0
  ([#43](https://github.com/rustic-rs/rustic_core/issues/43))
- Update serde monorepo to 1.0.190
  ([#41](https://github.com/rustic-rs/rustic_core/issues/41))
- Update rust crate cached to 0.46.0
  ([#42](https://github.com/rustic-rs/rustic_core/issues/42))
- Update rust crate serde_with to 3.4.0
  ([#44](https://github.com/rustic-rs/rustic_core/issues/44))
- Update rust crate zstd to 0.13.0
  ([#45](https://github.com/rustic-rs/rustic_core/issues/45))
- Update rust crate binrw to 0.13.1
  ([#46](https://github.com/rustic-rs/rustic_core/issues/46))
- Update rust crate cached to 0.46.1
  ([#47](https://github.com/rustic-rs/rustic_core/issues/47))
- Update rust crate enum-map to 2.7.1
  ([#49](https://github.com/rustic-rs/rustic_core/issues/49))
- Update serde monorepo to 1.0.192
  ([#50](https://github.com/rustic-rs/rustic_core/issues/50))
- Update rust crate enum-map-derive to 0.15.0
  ([#51](https://github.com/rustic-rs/rustic_core/issues/51))
- Update rust crate clap to 4.4.8
  ([#52](https://github.com/rustic-rs/rustic_core/issues/52))
- Update rust crate aes256ctr_poly1305aes to 0.2.0
  ([#54](https://github.com/rustic-rs/rustic_core/issues/54))
- Temporarily allow unused import for `cached` proc macro to fix lint warning
  when not on *nix systems

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
- Don't run Miri for now due to: <https://github.com/rust-lang/miri/issues/3066>
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
- Update changelog
- Prepare release

### Release

- Rustic_core v0.1.1 ([#2](https://github.com/rustic-rs/rustic_core/issues/2))

### Restore

- Add caching for user/group names
  ([#33](https://github.com/rustic-rs/rustic_core/issues/33))

## [rustic_core-v0.1.1] - 2023-09-18

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

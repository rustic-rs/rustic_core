<p align="center">
<img src="https://raw.githubusercontent.com/rustic-rs/assets/main/logos/readme_header_config.png" height="400" />
</p>
<p align="center"><b>Library for configuration support in rustic</b></p>
<p align="center">
<a href="https://crates.io/crates/rustic_config"><img src="https://img.shields.io/crates/v/rustic_config.svg" /></a>
<a href="https://docs.rs/rustic_config/"><img src="https://img.shields.io/docsrs/rustic_config?style=flat&amp;labelColor=1c1d42&amp;color=4f396a&amp;logo=Rust&amp;logoColor=white" /></a>
<a href="https://github.com/rustic-rs/rustic_core/blob/main/crates/backend/LICENSE-APACHE"><img src="https://img.shields.io/badge/license-Apache2.0/MIT-blue.svg" /></a>
<a href="https://crates.io/crates/rustic_config"><img src="https://img.shields.io/crates/d/rustic_config.svg" /></a>
<p>

## About

This library is a part of the [rustic](https://rustic.cli.rs) project and
provides a set of types for the
[`rustic_core`](https://crates.io/crates/rustic_core) library. It is used to
collect everything regarding configuration for backends, CLI, and other related
functionality.

**Note**: `rustic_config` is in an early development stage and its API is
subject to change in the next releases. If you want to give feedback on that,
please open an [issue](https://github.com/rustic-rs/rustic_core/issues).

## Contact

You can ask questions in the
[Discussions](https://github.com/rustic-rs/rustic/discussions) or have a look at
the [FAQ](https://rustic.cli.rs/docs/FAQ.html).

| Contact       | Where?                                                                                                          |
| ------------- | --------------------------------------------------------------------------------------------------------------- |
| Issue Tracker | [GitHub Issues](https://github.com/rustic-rs/rustic_core/issues/choose)                                         |
| Discord       | [![Discord](https://dcbadge.vercel.app/api/server/WRUWENZnzQ?style=flat-square)](https://discord.gg/WRUWENZnzQ) |
| Discussions   | [GitHub Discussions](https://github.com/rustic-rs/rustic/discussions)                                           |

## Usage

Add this to your `Cargo.toml`:

```toml
[dependencies]
rustic_config = "0.1"
```

<!-- ## Crate features

This crate exposes a few features for controlling dependency usage:

- **cli** - Enables support for CLI features by enabling `merge` and `clap`
  features. *This feature is disabled by default*.

- **clap** - Enables a dependency on the `clap` crate and enables parsing from
  the commandline. *This feature is disabled by default*.

- **merge** - Enables support for merging multiple values into one, which
  enables the `merge` dependency. This is needed for parsing commandline
  arguments and merging them into one (e.g. `config`). *This feature is disabled
  by default*. -->

## Usage & Examples

Due to being a support crate for
[`rustic_core`](https://crates.io/crates/rustic_core), there are no examples
here. Please check the examples in the
[`rustic_core`](https://crates.io/crates/rustic_core) crate.

## Contributing

Found a bug?
[Open an issue!](https://github.com/rustic-rs/rustic_core/issues/choose)

Got an idea for an improvement? Don't keep it to yourself!

- [Contribute fixes](https://github.com/rustic-rs/rustic_core/contribute) or new
  features via a pull requests!

Please make sure, that you read the
[contribution guide](https://rustic.cli.rs/docs/contributing-to-rustic.html).

## Minimum Rust version policy

This crate's minimum supported `rustc` version is `1.71.1`.

The current policy is that the minimum Rust version required to use this crate
can be increased in minor version updates. For example, if `crate 1.0` requires
Rust 1.20.0, then `crate 1.0.z` for all values of `z` will also require Rust
1.20.0 or newer. However, `crate 1.y` for `y > 0` may require a newer minimum
version of Rust.

In general, this crate will be conservative with respect to the minimum
supported version of Rust.

## License

Licensed under either of:

- [Apache License, Version 2.0](./LICENSE-APACHE)
- [MIT license](./LICENSE-MIT)

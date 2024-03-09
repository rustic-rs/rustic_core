//! `rustic_core` Integration Test for the Public API of the library
//!
//! Installs the nightly toolchain, produces documentation and derives
//! the public API from the rustdoc JSON. Then compares it with our
//! specified one.
//!
//! You can run them with 'nextest':
//! `cargo nextest run -r --workspace -E 'test(api)'`.
//!
//! To bless a new public API (e.g. in case of a new release)
//! you need to run:
//! `UPDATE_EXPECT=1 cargo test public_api`

use std::error::Error;

#[test]
#[ignore = "breaking changes, run before releasing"]
fn public_api() -> Result<(), Box<dyn Error>> {
    // Install a compatible nightly toolchain if it is missing
    rustup_toolchain::install(public_api::MINIMUM_NIGHTLY_RUST_VERSION)?;

    // Build rustdoc JSON
    let rustdoc_json = rustdoc_json::Builder::default()
        .toolchain(public_api::MINIMUM_NIGHTLY_RUST_VERSION)
        .build()
        .unwrap();

    // Derive the public API from the rustdoc JSON
    let public_api = public_api::Builder::from_rustdoc_json(rustdoc_json)
        .build()
        .unwrap();

    // Assert that the public API looks correct
    #[cfg(target_os = "windows")]
    expect_test::expect_file!["public_api_fixtures/public-api_win.txt"]
        .assert_eq(&public_api.to_string());
    #[cfg(target_os = "linux")]
    expect_test::expect_file!["public_api_fixtures/public-api_linux.txt"]
        .assert_eq(&public_api.to_string());
    #[cfg(target_os = "macos")]
    expect_test::expect_file!["public_api_fixtures/public-api_macos.txt"]
        .assert_eq(&public_api.to_string());

    Ok(())
}

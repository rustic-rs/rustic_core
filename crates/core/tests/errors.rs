use rstest::{fixture, rstest};
use std::backtrace::Backtrace;

use rustic_core::{ErrorKind, RusticError, Severity, Status};

#[fixture]
fn error() -> RusticError {
    RusticError::new(
        ErrorKind::Io,
        "A file could not be read, make sure the file is existing and readable by the system.",
    )
    .status(Status::Permanent)
    .severity(Severity::Error)
    .code("E001".into())
    .add_context("path", "/path/to/file")
    .add_context("called", "used s3 backend")
    .source(std::io::Error::new(std::io::ErrorKind::Other, "networking error").into())
    .backtrace(Backtrace::disabled())
}

#[rstest]
fn test_error_display(error: RusticError) {
    insta::assert_snapshot!(error);
}

#[rstest]
fn test_error_debug(error: RusticError) {
    insta::assert_debug_snapshot!(error);
}

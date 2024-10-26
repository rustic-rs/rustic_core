use rstest::{fixture, rstest};
use std::backtrace::Backtrace;

use rustic_core::{ErrorKind, RusticError, Severity, Status};

#[fixture]
fn error() -> RusticError {
    RusticError::new(
        ErrorKind::Io,
        "A file could not be read, make sure the file is existing and readable by the system.",
    )
    .attach_status(Status::Permanent)
    .attach_severity(Severity::Error)
    .attach_error_code("E001".into())
    .attach_context("path", "/path/to/file")
    .attach_context("called", "used s3 backend")
    .attach_source(std::io::Error::new(std::io::ErrorKind::Other, "networking error").into())
}

#[rstest]
fn test_error_display(error: RusticError) {
    insta::assert_snapshot!(error);
}

#[rstest]
fn test_error_debug(error: RusticError) {
    insta::assert_debug_snapshot!(error);
}

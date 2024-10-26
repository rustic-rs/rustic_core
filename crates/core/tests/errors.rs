use rstest::{fixture, rstest};

use rustic_core::{ErrorKind, RusticError, Severity, Status};

#[fixture]
fn error() -> Box<RusticError> {
    RusticError::new(
        ErrorKind::Io,
        "A file could not be read, make sure the file is existing and readable by the system.",
    )
    .attach_status(Status::Permanent)
    .attach_severity(Severity::Error)
    .attach_error_code("C001")
    .attach_context("path", "/path/to/file")
    .attach_context("called", "used s3 backend")
    .attach_source(std::io::Error::new(
        std::io::ErrorKind::Other,
        "networking error",
    ))
}

#[rstest]
fn test_error_display(error: Box<RusticError>) {
    insta::assert_snapshot!(error);
}

#[rstest]
fn test_error_debug(error: Box<RusticError>) {
    insta::assert_debug_snapshot!(error);
}

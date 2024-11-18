use rstest::{fixture, rstest};

use rustic_core::{ErrorKind, RusticError, Severity, Status};

#[fixture]
fn error() -> Box<RusticError> {
    RusticError::with_source(
        ErrorKind::InputOutput,
        "A file could not be read, make sure the file at `{path}` is existing and readable by the system.",
        std::io::Error::new(std::io::ErrorKind::ConnectionReset, "Networking Error"),
    )
    .attach_context("path", "/path/to/file")
    .attach_context("called", "used s3 backend")
    .attach_status(Status::Permanent)
    .attach_severity(Severity::Error)
    .attach_error_code("C001")
    .append_guidance_line("Appended guidance line")
    .prepend_guidance_line("Prepended guidance line")
    .attach_existing_issue_url("https://github.com/rustic-rs/rustic_core/issues/209")
    .ask_report()
}

#[fixture]
fn minimal_error() -> Box<RusticError> {
    RusticError::new(
        ErrorKind::InputOutput,
        "A file could not be read, make sure the file at `{path}` is existing and readable by the system.",
    )
    .attach_context("path", "/path/to/file")
}

#[rstest]
#[allow(clippy::boxed_local)]
fn test_error_display(error: Box<RusticError>) {
    insta::assert_snapshot!(error);
}

#[rstest]
#[allow(clippy::boxed_local)]
fn test_error_debug(error: Box<RusticError>) {
    insta::assert_debug_snapshot!(error);
}

#[rstest]
#[allow(clippy::boxed_local)]
fn test_log_output_passes(error: Box<RusticError>) {
    insta::assert_snapshot!(error.display_log());
}

#[rstest]
#[allow(clippy::boxed_local)]
fn test_log_output_minimal_passes(minimal_error: Box<RusticError>) {
    insta::assert_snapshot!(minimal_error.display_log());
}

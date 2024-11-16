use crate::SupportedBackend;
use rustic_core::{ErrorKind, RusticError, RusticResult};

/// A backend location. This is a string that represents the location of the backend.
#[derive(PartialEq, Eq, Debug, Clone)]
pub struct BackendLocation(String);

impl std::ops::Deref for BackendLocation {
    type Target = String;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<str> for BackendLocation {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for BackendLocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)?;
        Ok(())
    }
}

/// Splits the given url into the backend type and the path.
///
/// # Arguments
///
/// * `url` - The url to split.
///
/// # Errors
///
/// * If the url is not a valid url, an error is returned.
///
/// # Returns
///
/// A tuple with the backend type and the path.
///
/// # Notes
///
/// If the url is a windows path, the type will be "local".
pub fn location_to_type_and_path(
    raw_location: &str,
) -> RusticResult<(SupportedBackend, BackendLocation)> {
    match raw_location.split_once(':') {
        #[cfg(windows)]
        Some((drive_letter, _)) if drive_letter.len() == 1 && !raw_location.contains('/') => Ok((
            SupportedBackend::Local,
            BackendLocation(raw_location.to_string()),
        )),
        #[cfg(windows)]
        Some((scheme, path)) if scheme.contains('\\') || path.contains('\\') => Ok((
            SupportedBackend::Local,
            BackendLocation(raw_location.to_string()),
        )),
        Some((scheme, path)) => Ok((
            SupportedBackend::try_from(scheme).map_err(|err| {
                RusticError::with_source(
                ErrorKind::Unsupported,
                "The backend type `{name}` is not supported. Please check the given backend and try again.",
                err
            )
            .attach_context("name", scheme)
            })?,
            BackendLocation(path.to_string()),
        )),
        None => Ok((
            SupportedBackend::Local,
            BackendLocation(raw_location.to_string()),
        )),
    }
}

#[cfg(test)]
mod tests {

    #[allow(unused_imports)]
    use rstest::rstest;

    #[allow(unused_imports)]
    use super::*;

    #[rstest]
    #[cfg(not(windows))]
    #[case("local:/tmp/repo", (SupportedBackend::Local, BackendLocation::try_from("/tmp/repo").unwrap()))]
    #[cfg(not(windows))]
    #[case("/tmp/repo", (SupportedBackend::Local, BackendLocation::try_from("/tmp/repo").unwrap()))]
    #[cfg(feature = "rclone")]
    #[case(
        "rclone:remote:/tmp/repo",
        (SupportedBackend::Rclone,
        BackendLocation::try_from("remote:/tmp/repo").unwrap())
    )]
    #[cfg(feature = "rest")]
    #[case(
        "rest:https://example.com/tmp/repo",
        (SupportedBackend::Rest,
        BackendLocation::try_from("https://example.com/tmp/repo").unwrap())
    )]
    #[cfg(feature = "opendal")]
    #[case(
        "opendal:https://example.com/tmp/repo",
        (SupportedBackend::OpenDAL,
        BackendLocation::try_from("https://example.com/tmp/repo").unwrap())
    )]
    #[cfg(windows)]
    #[case(
        r#"C:\tmp\repo"#,
        (SupportedBackend::Local,
        BackendLocation::try_from(r#"C:\tmp\repo"#).unwrap())
    )]
    #[should_panic]
    #[cfg(windows)]
    #[case(
        r#"C:/tmp/repo"#,
        (SupportedBackend::Local,
        BackendLocation::try_from(r#"C:/tmp/repo"#).unwrap())
    )]
    #[cfg(windows)]
    #[case(
        r#"\\.\C:\Test\repo"#,
        (SupportedBackend::Local,
        BackendLocation::try_from(r#"\\.\C:\Test\repo"#).unwrap())
    )]
    #[cfg(windows)]
    #[case(
        r#"\\?\C:\Test\repo"#,
        (SupportedBackend::Local,
        BackendLocation::try_from(r#"\\?\C:\Test\repo"#).unwrap())
    )]
    #[cfg(windows)]
    #[case(
        r#"\\.\Volume{b75e2c83-0000-0000-0000-602f00000000}\Test\repo"#,
        (SupportedBackend::Local,
        BackendLocation::try_from(r#"\\.\Volume{b75e2c83-0000-0000-0000-602f00000000}\Test\repo"#).unwrap())
    )]
    #[cfg(windows)]
    #[case(
        r#"\\?\Volume{b75e2c83-0000-0000-0000-602f00000000}\Test\repo"#,
        (SupportedBackend::Local,
        BackendLocation::try_from(r#"\\?\Volume{b75e2c83-0000-0000-0000-602f00000000}\Test\repo"#).unwrap())
    )]
    #[cfg(windows)]
    #[case(
        r#"\\Server2\Share\Test\repo"#,
        (SupportedBackend::Local,
        BackendLocation::try_from(r#"\\Server2\Share\Test\repo"#).unwrap())
    )]
    #[cfg(windows)]
    #[case(
        r#"\\?\UNC\Server\Share\Test\repo"#,
        (SupportedBackend::Local,
        BackendLocation::try_from(r#"\\?\UNC\Server\Share\Test\repo"#).unwrap())
    )]
    #[cfg(windows)]
    #[case(
        r#"C:\Projects\apilibrary\"#,
        (SupportedBackend::Local,
        BackendLocation::try_from(r#"C:\Projects\apilibrary\"#).unwrap())
    )]
    // A relative path from the current directory of the C: drive.
    #[cfg(windows)]
    #[case(
        r#"C:Projects\apilibrary\"#,
        (SupportedBackend::Local,
        BackendLocation::try_from(r#"C:Projects\apilibrary\"#).unwrap())
    )]
    // A relative path from the root of the current drive.
    #[cfg(windows)]
    #[case(
        r#"\Program Files\Custom Utilities\rustic\Repositories\"#,
        (SupportedBackend::Local,
        BackendLocation::try_from(r#"\Program Files\Custom Utilities\rustic\Repositories\"#).unwrap())
    )]
    #[should_panic]
    #[cfg(windows)]
    #[case(
        r#"..\Publications\TravelBrochures\"#,
        (SupportedBackend::Local,
        BackendLocation::try_from(r#"..\Publications\TravelBrochures\"#).unwrap())
    )]
    #[should_panic]
    #[cfg(windows)]
    #[case(
        r#"2023\repos\"#,
        (SupportedBackend::Local,
        BackendLocation::try_from(r#"2023\repos\"#).unwrap())
    )]
    // The root directory of the C: drive on localhost.
    #[cfg(windows)]
    #[case(
        r#"\\localhost\C$\"#, 
        (SupportedBackend::Local,
        BackendLocation::try_from(r#"\\localhost\C$\"#).unwrap())
    )]
    #[cfg(windows)]
    #[case(
        r#"\\127.0.0.1\c$\temp\repo\"#, 
        (SupportedBackend::Local,
        BackendLocation::try_from(r#"\\127.0.0.1\c$\temp\repo\"#).unwrap())
    )]
    fn test_location_to_type_and_path_is_ok(
        #[case] url: &str,
        #[case] expected: (SupportedBackend, BackendLocation),
    ) {
        // Check https://learn.microsoft.com/en-us/dotnet/standard/io/file-path-formats
        assert_eq!(location_to_type_and_path(url).unwrap(), expected);
    }
}

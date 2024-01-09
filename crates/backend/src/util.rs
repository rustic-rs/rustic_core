use std::path::Path;

use crate::SupportedBackend;
use anyhow::Result;
use dunce::canonicalize;
use url::Url;

#[derive(PartialEq, Debug)]
pub struct BackendLocation(String);

impl std::ops::Deref for BackendLocation {
    type Target = String;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl TryFrom<&str> for BackendLocation {
    type Error = anyhow::Error;

    fn try_from(value: &str) -> std::result::Result<Self, Self::Error> {
        match (canonicalize(Path::new(value)), Url::parse(value), value) {
            (Ok(_), Ok(_), _) => Ok(BackendLocation(value.to_owned())),
            (Ok(val), _, _) => Ok(BackendLocation(val.to_str().unwrap().to_owned())),
            (_, Ok(val), _) => Ok(BackendLocation(val.to_string())),
            #[cfg(windows)]
            (_, _, val) if val.starts_with('\\') => Ok(BackendLocation(val.to_string())),
            _ => Err(anyhow::anyhow!("Invalid backend location: {}", value)),
        }
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
/// # Returns
///
/// A tuple with the backend type and the path.
///
/// # Notes
///
/// If the url is a windows path, the type will be "local".
pub fn location_to_type_and_path(
    raw_location: &str,
) -> Result<(SupportedBackend, BackendLocation)> {
    match raw_location.split_once(':') {
        #[cfg(windows)]
        Some((drive_letter, _)) if drive_letter.len() == 1 && !raw_location.contains('/') => Ok((
            SupportedBackend::Local,
            BackendLocation::try_from(raw_location)?,
        )),
        #[cfg(windows)]
        Some((scheme, path)) if scheme.contains('\\') || path.contains('\\') => Ok((
            SupportedBackend::Local,
            BackendLocation::try_from(raw_location)?,
        )),
        Some((scheme, path)) => Ok((
            SupportedBackend::try_from(scheme)?,
            BackendLocation::try_from(path)?,
        )),
        None => Ok((
            SupportedBackend::Local,
            BackendLocation::try_from(raw_location)?,
        )),
    }
}

#[cfg(test)]
mod tests {

    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case("local:/tmp/repo", (SupportedBackend::Local, BackendLocation::try_from("/tmp/repo").unwrap()))]
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
    #[cfg(feature = "s3")]
    #[case(
        "s3:https://example.com/tmp/repo",
        (SupportedBackend::S3,
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

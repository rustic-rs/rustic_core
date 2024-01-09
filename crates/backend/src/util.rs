use crate::SupportedBackend;
use anyhow::Result;

#[derive(PartialEq, Debug)]
pub struct BackendUrl<'a>(&'a str);

impl<'a> std::ops::Deref for BackendUrl<'a> {
    type Target = &'a str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'a> AsRef<str> for BackendUrl<'a> {
    fn as_ref(&self) -> &str {
        self.0
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
pub fn url_to_type_and_path(url: &str) -> Result<(SupportedBackend, BackendUrl)> {
    match url.split_once(':') {
        #[cfg(windows)]
        Some((drive, _)) if drive.len() == 1 => Ok((SupportedBackend::Local, BackendUrl(url))),
        Some((scheme, path)) if scheme.contains('\\') || path.contains('\\') => {
            Ok((SupportedBackend::Local, BackendUrl(url)))
        }
        Some((scheme, path)) => Ok((SupportedBackend::try_from(scheme)?, BackendUrl(path))),
        None => Ok((SupportedBackend::Local, BackendUrl(url))),
    }
}

#[cfg(test)]
mod tests {

    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case("local:/tmp/repo", (SupportedBackend::Local, BackendUrl("/tmp/repo")))]
    #[case(
        "rclone:remote:/tmp/repo",
        (SupportedBackend::Rclone,
        BackendUrl("remote:/tmp/repo"))
    )]
    #[cfg(feature = "rest")]
    #[case(
        "rest:https://example.com/tmp/repo",
        (SupportedBackend::Rest,
        BackendUrl("https://example.com/tmp/repo"))
    )]
    #[case(
        "opendal:https://example.com/tmp/repo",
        (SupportedBackend::OpenDAL,
        BackendUrl("https://example.com/tmp/repo"))
    )]
    #[case(
        "s3:https://example.com/tmp/repo",
        (SupportedBackend::S3,
        BackendUrl("https://example.com/tmp/repo"))
    )]
    #[case(
        r#"C:\tmp\repo"#,
        (SupportedBackend::Local,
        BackendUrl(r#"C:\tmp\repo"#))
    )]
    #[case(
        r#"C:/tmp/repo"#,
        (SupportedBackend::Local,
        BackendUrl(r#"C:/tmp/repo"#))
    )]
    #[case(
        r#"\\.\C:\Test\repo"#,
        (SupportedBackend::Local,
        BackendUrl(r#"\\.\C:\Test\repo"#))
    )]
    #[case(
        r#"\\?\C:\Test\repo"#,
        (SupportedBackend::Local,
        BackendUrl(r#"\\?\C:\Test\repo"#))
    )]
    #[case(
        r#"\\.\Volume{b75e2c83-0000-0000-0000-602f00000000}\Test\repo"#,
        (SupportedBackend::Local,
        BackendUrl(r#"\\.\Volume{b75e2c83-0000-0000-0000-602f00000000}\Test\repo"#))
    )]
    #[case(
        r#"\\?\Volume{b75e2c83-0000-0000-0000-602f00000000}\Test\repo"#,
        (SupportedBackend::Local,
        BackendUrl(r#"\\?\Volume{b75e2c83-0000-0000-0000-602f00000000}\Test\repo"#))
    )]
    #[case(
        r#"\\Server2\Share\Test\repo"#,
        (SupportedBackend::Local,
        BackendUrl(r#"\\Server2\Share\Test\repo"#))
    )]
    #[case(
        r#"\\?\UNC\Server\Share\Test\repo"#,
        (SupportedBackend::Local,
        BackendUrl(r#"\\?\UNC\Server\Share\Test\repo"#))
    )]
    #[case(
        r#"C:\Projects\apilibrary\"#,
        (SupportedBackend::Local,
        BackendUrl(r#"C:\Projects\apilibrary\"#))
    )]
    // A relative path from the current directory of the C: drive.
    #[case(
        r#"C:Projects\apilibrary\"#,
        (SupportedBackend::Local,
        BackendUrl(r#"C:Projects\apilibrary\"#))
    )]
    // A relative path from the root of the current drive.
    #[case(
        r#"\Program Files\Custom Utilities\rustic\Repositories\"#,
        (SupportedBackend::Local,
        BackendUrl(r#"\Program Files\Custom Utilities\rustic\Repositories\"#))
    )]
    #[case(
        r#"..\Publications\TravelBrochures\"#,
        (SupportedBackend::Local,
        BackendUrl(r#"..\Publications\TravelBrochures\"#))
    )]
    #[case(
        r#"2023\repos\"#,
        (SupportedBackend::Local,
        BackendUrl(r#"2023\repos\"#))
    )]
    // The root directory of the C: drive on system07.
    #[case(
        r#"\\system07\C$\"#, 
        (SupportedBackend::Local,
        BackendUrl(r#"\\system07\C$\"#))
    )]
    #[case(
        r#"\\127.0.0.1\c$\temp\repo\"#, 
        (SupportedBackend::Local,
        BackendUrl(r#"\\127.0.0.1\c$\temp\repo\"#))
    )]
    fn test_url_to_type_and_path_is_ok(
        #[case] url: &str,
        #[case] expected: (SupportedBackend, BackendUrl),
    ) {
        // Check https://learn.microsoft.com/en-us/dotnet/standard/io/file-path-formats
        assert_eq!(url_to_type_and_path(url).unwrap(), expected);
    }
}

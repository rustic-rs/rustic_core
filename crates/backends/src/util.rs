use std::path::PathBuf;

use crate::SupportedBackend;

#[derive(PartialEq, Debug)]
pub struct BackendUrl(PathBuf);

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
pub fn url_to_type_and_path(url: &str) -> anyhow::Result<(SupportedBackend, BackendUrl)> {
    match url.split_once(':') {
        #[cfg(windows)]
        Some((drive, _)) if drive.len() == 1 => {
            Ok((SupportedBackend::try_from("local")?, BackendUrl(url.into())))
        }
        Some((scheme, _)) if scheme.contains('\\') => {
            Ok((SupportedBackend::Local, BackendUrl(url.into())))
        }
        Some((scheme, path)) => Ok((SupportedBackend::try_from(scheme)?, BackendUrl(path.into()))),
        None => Ok((SupportedBackend::try_from("local")?, BackendUrl(url.into()))),
    }
}

#[cfg(test)]
mod tests {

    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case("local:/tmp/repo", (SupportedBackend::Local, BackendUrl("/tmp/repo".into())))]
    #[case(
        "rclone:remote:/tmp/repo",
        (SupportedBackend::Rclone,
        BackendUrl("remote:/tmp/repo".into()))
    )]
    #[case(
        "rest:https://example.com/tmp/repo",
        (SupportedBackend::Rest,
        BackendUrl("https://example.com/tmp/repo".into()))
    )]
    #[case(
        "opendal:https://example.com/tmp/repo",
        (SupportedBackend::OpenDAL,
        BackendUrl("https://example.com/tmp/repo".into()))
    )]
    #[case(
        "s3:https://example.com/tmp/repo",
        (SupportedBackend::S3,
        BackendUrl("https://example.com/tmp/repo".into()))
    )]
    #[case(
        r#"C:\tmp\repo"#,
        (SupportedBackend::Local,
        BackendUrl(r#"C:\tmp\repo"#.into()))
    )]
    #[case(
        r#"\\.\C:\\tmp\\repo"#,
        (SupportedBackend::Local,
        BackendUrl(r#"\\.\C:\tmp\repo"#.into()))
    )]
    fn test_url_to_type_and_path_is_ok(
        #[case] url: &str,
        #[case] expected: (SupportedBackend, BackendUrl),
    ) {
        assert_eq!(url_to_type_and_path(url).unwrap(), expected);

        // TODO: https://learn.microsoft.com/en-us/dotnet/standard/io/file-path-formats
        // "\Program Files\Custom Utilities\"
        // "2018\January.xlsx"
        // "..\Publications\TravelBrochures\"
        // "C:\Projects\apilibrary\"
        // "C:Projects\apilibrary\"
        // "\\system07\C$\ "
        // "\\Server2\Share\Test\"
        // "\\.\C:\Test\"
        // "\\?\C:\Test\"
        // "\\.\Volume{b75e2c83-0000-0000-0000-602f00000000}\Test\"
        // "\\?\Volume{b75e2c83-0000-0000-0000-602f00000000}\Test\"
        // "\\?\UNC\Server\Share\Test\"
    }
}

use crate::SupportedBackend;
use anyhow::Result;
use url::Url;

#[derive(PartialEq, Debug)]
pub struct BackendUrl(Url);

impl std::ops::Deref for BackendUrl {
    type Target = Url;

    fn deref(&self) -> &Self::Target {
        &self.0
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
        Some((drive, _)) if drive.len() == 1 => Ok((
            SupportedBackend::try_from("local")?,
            BackendUrl(Url::from_directory_path(url).expect("URL is not a valid directory path")),
        )),
        #[cfg(windows)]
        Some((scheme, _)) if scheme.contains('\\') => Ok((
            SupportedBackend::Local,
            BackendUrl(Url::from_directory_path(url).expect("URL is not a valid directory path")),
        )),
        Some((scheme, path)) => Ok((
            SupportedBackend::try_from(scheme)?,
            BackendUrl(Url::from_directory_path(path).expect("URL is not a valid directory path")),
        )),
        None => Ok((
            SupportedBackend::try_from("local")?,
            BackendUrl(Url::from_directory_path(url).expect("URL is not a valid directory path")),
        )),
    }
}

#[cfg(test)]
mod tests {

    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case("local:/tmp/repo", (SupportedBackend::Local, BackendUrl(Url::from_directory_path("/tmp/repo").unwrap())))]
    #[case(
        "rclone:remote:/tmp/repo",
        (SupportedBackend::Rclone,
        BackendUrl(Url::from_directory_path("remote:/tmp/repo").unwrap()))
    )]
    #[case(
        "rest:https://example.com/tmp/repo",
        (SupportedBackend::Rest,
        BackendUrl(Url::from_directory_path("https://example.com/tmp/repo").unwrap()))
    )]
    #[case(
        "opendal:https://example.com/tmp/repo",
        (SupportedBackend::OpenDAL,
        BackendUrl(Url::from_directory_path("https://example.com/tmp/repo").unwrap()))
    )]
    #[case(
        "s3:https://example.com/tmp/repo",
        (SupportedBackend::S3,
        BackendUrl(Url::from_directory_path("https://example.com/tmp/repo").unwrap()))
    )]
    #[case(
        r#"C:\tmp\repo"#,
        (SupportedBackend::Local,
        BackendUrl(Url::from_directory_path(r#"C:\tmp\repo"#).unwrap()))
    )]
    #[case(
        r#"\\.\C:\\tmp\\repo"#,
        (SupportedBackend::Local,
        BackendUrl(Url::from_directory_path(r#"\\.\C:\tmp\repo"#).unwrap()))
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

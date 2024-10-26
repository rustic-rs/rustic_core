pub(crate) mod error {
    //! Error types and Result module.
    #![allow(clippy::doc_markdown)]
    use derive_setters::Setters;
    use smol_str::SmolStr;
    use std::{backtrace::Backtrace, fmt::{self, Display}};
    pub(crate) mod constants {
        pub const DEFAULT_DOCS_URL: &str = "https://rustic.cli.rs/docs/errors/";
        pub const DEFAULT_ISSUE_URL: &str = "https://github.com/rustic-rs/rustic_core/issues/new";
    }
    /// Result type that is being returned from methods that can fail and thus have [`RusticError`]s.
    pub type RusticResult<T, E = Box<RusticError>> = Result<T, E>;
    #[setters(strip_option, into)]
    #[non_exhaustive]
    /// Errors that can result from rustic.
    pub struct RusticError {
        /// The kind of the error.
        kind: ErrorKind,
        /// Chain to the cause of the error.
        source: Option<Box<(dyn std::error::Error + Send + Sync)>>,
        /// The error message with guidance.
        guidance: SmolStr,
        /// The context of the error.
        context: Box<[(&'static str, SmolStr)]>,
        /// The URL of the documentation for the error.
        docs_url: Option<SmolStr>,
        /// Error code.
        code: Option<SmolStr>,
        /// The URL of the issue tracker for opening a new issue.
        new_issue_url: Option<SmolStr>,
        /// The URL of an already existing issue that is related to this error.
        existing_issue_url: Option<SmolStr>,
        /// Severity of the error.
        severity: Option<Severity>,
        /// The status of the error.
        status: Option<Status>,
        /// Backtrace of the error.
        backtrace: Option<Backtrace>,
    }
    #[allow(unused_qualifications)]
    #[automatically_derived]
    impl std::error::Error for RusticError {
        fn source(&self) -> ::core::option::Option<&(dyn std::error::Error + 'static)> {
            use thiserror::__private::AsDynError as _;
            ::core::option::Option::Some(self.source.as_ref()?.as_dyn_error())
        }
    }
    #[automatically_derived]
    impl ::core::fmt::Debug for RusticError {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            let names: &'static _ = &[
                "kind",
                "source",
                "guidance",
                "context",
                "docs_url",
                "code",
                "new_issue_url",
                "existing_issue_url",
                "severity",
                "status",
                "backtrace",
            ];
            let values: &[&dyn ::core::fmt::Debug] = &[
                &self.kind,
                &self.source,
                &self.guidance,
                &self.context,
                &self.docs_url,
                &self.code,
                &self.new_issue_url,
                &self.existing_issue_url,
                &self.severity,
                &self.status,
                &&self.backtrace,
            ];
            ::core::fmt::Formatter::debug_struct_fields_finish(
                f,
                "RusticError",
                names,
                values,
            )
        }
    }
    impl RusticError {
        /// The kind of the error.
        pub fn kind(self, value: impl ::std::convert::Into<ErrorKind>) -> Self {
            RusticError {
                kind: value.into(),
                ..self
            }
        }
        /// Chain to the cause of the error.
        pub fn source(
            self,
            value: impl ::std::convert::Into<Box<(dyn std::error::Error + Send + Sync)>>,
        ) -> Self {
            RusticError {
                source: Some(value.into()),
                ..self
            }
        }
        /// The error message with guidance.
        pub fn guidance(self, value: impl ::std::convert::Into<SmolStr>) -> Self {
            RusticError {
                guidance: value.into(),
                ..self
            }
        }
        /// The context of the error.
        pub fn context(
            self,
            value: impl ::std::convert::Into<Box<[(&'static str, SmolStr)]>>,
        ) -> Self {
            RusticError {
                context: value.into(),
                ..self
            }
        }
        /// The URL of the documentation for the error.
        pub fn docs_url(self, value: impl ::std::convert::Into<SmolStr>) -> Self {
            RusticError {
                docs_url: Some(value.into()),
                ..self
            }
        }
        /// Error code.
        pub fn code(self, value: impl ::std::convert::Into<SmolStr>) -> Self {
            RusticError {
                code: Some(value.into()),
                ..self
            }
        }
        /// The URL of the issue tracker for opening a new issue.
        pub fn new_issue_url(self, value: impl ::std::convert::Into<SmolStr>) -> Self {
            RusticError {
                new_issue_url: Some(value.into()),
                ..self
            }
        }
        /// The URL of an already existing issue that is related to this error.
        pub fn existing_issue_url(
            self,
            value: impl ::std::convert::Into<SmolStr>,
        ) -> Self {
            RusticError {
                existing_issue_url: Some(value.into()),
                ..self
            }
        }
        /// Severity of the error.
        pub fn severity(self, value: impl ::std::convert::Into<Severity>) -> Self {
            RusticError {
                severity: Some(value.into()),
                ..self
            }
        }
        /// The status of the error.
        pub fn status(self, value: impl ::std::convert::Into<Status>) -> Self {
            RusticError {
                status: Some(value.into()),
                ..self
            }
        }
        /// Backtrace of the error.
        pub fn backtrace(self, value: impl ::std::convert::Into<Backtrace>) -> Self {
            RusticError {
                backtrace: Some(value.into()),
                ..self
            }
        }
    }
    impl Display for RusticError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_fmt(format_args!("{0} occurred in `rustic_core`", self.kind))?;
            f.write_fmt(format_args!("\n\nMessage:\n{0}", self.guidance))?;
            if !self.context.is_empty() {
                f.write_fmt(format_args!("\n\nContext:\n"))?;
                f.write_fmt(
                    format_args!(
                        "{0}",
                        self
                            .context
                            .iter()
                            .map(|(k, v)| ::alloc::__export::must_use({
                                let res = ::alloc::fmt::format(
                                    format_args!("{0}: {1}", k, v),
                                );
                                res
                            }))
                            .collect::<Vec<_>>()
                            .join(",\n"),
                    ),
                )?;
            }
            if let Some(cause) = &self.source {
                f.write_fmt(format_args!("\n\nCaused by: {0}", cause))?;
            }
            if let Some(severity) = &self.severity {
                f.write_fmt(format_args!("\n\nSeverity: {0:?}", severity))?;
            }
            if let Some(status) = &self.status {
                f.write_fmt(format_args!("\n\nStatus: {0:?}", status))?;
            }
            if let Some(code) = &self.code {
                let default_docs_url = SmolStr::from(constants::DEFAULT_DOCS_URL);
                let docs_url = self.docs_url.as_ref().unwrap_or(&default_docs_url);
                f.write_fmt(
                    format_args!("\n\nFor more information, see: {0}{1}", docs_url, code),
                )?;
            }
            if let Some(existing_issue_url) = &self.existing_issue_url {
                f.write_fmt(
                    format_args!(
                        "\n\nThis might be a related issue, please check it for a possible workaround and/or further guidance: {0}",
                        existing_issue_url,
                    ),
                )?;
            }
            let default_issue_url = SmolStr::from(constants::DEFAULT_ISSUE_URL);
            let new_issue_url = self
                .new_issue_url
                .as_ref()
                .unwrap_or(&default_issue_url);
            f.write_fmt(
                format_args!(
                    "\n\nIf you think this is an undiscovered bug, please open an issue at: {0}",
                    new_issue_url,
                ),
            )?;
            if let Some(backtrace) = &self.backtrace {
                f.write_fmt(format_args!("\n\nBacktrace:\n{0:?}", backtrace))?;
            }
            Ok(())
        }
    }
    impl RusticError {
        /// Creates a new error with the given kind and guidance.
        pub fn new(kind: ErrorKind, guidance: impl Into<String>) -> Box<Self> {
            Box::new(Self {
                kind,
                guidance: guidance.into().into(),
                context: Box::default(),
                source: None,
                code: None,
                docs_url: None,
                new_issue_url: None,
                existing_issue_url: None,
                severity: None,
                status: None,
                backtrace: Some(Backtrace::capture()),
            })
        }
        /// Checks if the error has a specific error code.
        pub fn is_code(&self, code: &str) -> bool {
            self.code.as_ref().map_or(false, |c| c.as_str() == code)
        }
        /// Expose the inner error kind.
        ///
        /// This is useful for matching on the error kind.
        pub fn into_inner(self) -> ErrorKind {
            self.kind
        }
        /// Checks if the error is due to an incorrect password
        pub fn is_incorrect_password(&self) -> bool {
            match self.kind {
                ErrorKind::Password => true,
                _ => false,
            }
        }
        /// Creates a new error from a given error.
        pub fn from<T: std::error::Error + Display + Send + Sync + 'static>(
            error: T,
            kind: ErrorKind,
        ) -> Box<Self> {
            Box::new(Self {
                kind,
                guidance: error.to_string().into(),
                context: Box::default(),
                source: Some(Box::new(error)),
                code: None,
                docs_url: None,
                new_issue_url: None,
                existing_issue_url: None,
                severity: None,
                status: None,
                backtrace: Some(Backtrace::capture()),
            })
        }
        /// Adds a context to the error.
        #[must_use]
        pub fn with_context(
            mut self,
            key: &'static str,
            value: impl Into<String>,
        ) -> Box<Self> {
            let mut context = self.context.to_vec();
            context.push((key, value.into().into()));
            self.context = context.into_boxed_slice();
            Box::new(self)
        }
    }
    /// Severity of an error, ranging from informational to fatal.
    pub enum Severity {
        /// Informational
        Info,
        /// Warning
        Warning,
        /// Error
        Error,
        /// Fatal
        Fatal,
    }
    #[automatically_derived]
    impl ::core::fmt::Debug for Severity {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            ::core::fmt::Formatter::write_str(
                f,
                match self {
                    Severity::Info => "Info",
                    Severity::Warning => "Warning",
                    Severity::Error => "Error",
                    Severity::Fatal => "Fatal",
                },
            )
        }
    }
    #[automatically_derived]
    impl ::core::clone::Clone for Severity {
        #[inline]
        fn clone(&self) -> Severity {
            *self
        }
    }
    #[automatically_derived]
    impl ::core::marker::Copy for Severity {}
    #[automatically_derived]
    impl ::core::marker::StructuralPartialEq for Severity {}
    #[automatically_derived]
    impl ::core::cmp::PartialEq for Severity {
        #[inline]
        fn eq(&self, other: &Severity) -> bool {
            let __self_discr = ::core::intrinsics::discriminant_value(self);
            let __arg1_discr = ::core::intrinsics::discriminant_value(other);
            __self_discr == __arg1_discr
        }
    }
    #[automatically_derived]
    impl ::core::cmp::Eq for Severity {
        #[inline]
        #[doc(hidden)]
        #[coverage(off)]
        fn assert_receiver_is_total_eq(&self) -> () {}
    }
    /// Status of an error, indicating whether it is permanent, temporary, or persistent.
    pub enum Status {
        /// Permanent, may not be retried
        Permanent,
        /// Temporary, may be retried
        Temporary,
        /// Persistent, may be retried, but may not succeed
        Persistent,
    }
    #[automatically_derived]
    impl ::core::fmt::Debug for Status {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            ::core::fmt::Formatter::write_str(
                f,
                match self {
                    Status::Permanent => "Permanent",
                    Status::Temporary => "Temporary",
                    Status::Persistent => "Persistent",
                },
            )
        }
    }
    #[automatically_derived]
    impl ::core::clone::Clone for Status {
        #[inline]
        fn clone(&self) -> Status {
            *self
        }
    }
    #[automatically_derived]
    impl ::core::marker::Copy for Status {}
    #[automatically_derived]
    impl ::core::marker::StructuralPartialEq for Status {}
    #[automatically_derived]
    impl ::core::cmp::PartialEq for Status {
        #[inline]
        fn eq(&self, other: &Status) -> bool {
            let __self_discr = ::core::intrinsics::discriminant_value(self);
            let __arg1_discr = ::core::intrinsics::discriminant_value(other);
            __self_discr == __arg1_discr
        }
    }
    #[automatically_derived]
    impl ::core::cmp::Eq for Status {
        #[inline]
        #[doc(hidden)]
        #[coverage(off)]
        fn assert_receiver_is_total_eq(&self) -> () {}
    }
    /// [`ErrorKind`] describes the errors that can happen while executing a high-level command.
    ///
    /// This is a non-exhaustive enum, so additional variants may be added in future. It is
    /// recommended to match against the wildcard `_` instead of listing all possible variants,
    /// to avoid problems when new variants are added.
    #[non_exhaustive]
    pub enum ErrorKind {
        /// Backend Error
        Backend,
        /// IO Error
        Io,
        /// Password Error
        Password,
        /// Repository Error
        Repository,
        /// Command Error
        Command,
        /// Config Error
        Config,
        /// Index Error
        Index,
        /// Key Error
        Key,
        /// Blob Error
        Blob,
        /// Crypto Error
        Cryptography,
        /// Compression Error
        Compression,
        /// Parsing Error
        Parsing,
        /// Conversion Error
        Conversion,
        /// Permission Error
        Permission,
        /// Polynomial Error
        Polynomial,
        /// Multithreading Error
        Multithreading,
        /// Processing Error
        Processing,
        /// Something is not supported
        Unsupported,
        /// External Command
        ExternalCommand,
    }
    #[allow(unused_qualifications)]
    #[automatically_derived]
    impl std::error::Error for ErrorKind {}
    #[automatically_derived]
    impl ::core::fmt::Debug for ErrorKind {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            ::core::fmt::Formatter::write_str(
                f,
                match self {
                    ErrorKind::Backend => "Backend",
                    ErrorKind::Io => "Io",
                    ErrorKind::Password => "Password",
                    ErrorKind::Repository => "Repository",
                    ErrorKind::Command => "Command",
                    ErrorKind::Config => "Config",
                    ErrorKind::Index => "Index",
                    ErrorKind::Key => "Key",
                    ErrorKind::Blob => "Blob",
                    ErrorKind::Cryptography => "Cryptography",
                    ErrorKind::Compression => "Compression",
                    ErrorKind::Parsing => "Parsing",
                    ErrorKind::Conversion => "Conversion",
                    ErrorKind::Permission => "Permission",
                    ErrorKind::Polynomial => "Polynomial",
                    ErrorKind::Multithreading => "Multithreading",
                    ErrorKind::Processing => "Processing",
                    ErrorKind::Unsupported => "Unsupported",
                    ErrorKind::ExternalCommand => "ExternalCommand",
                },
            )
        }
    }
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _: () = {
        trait DisplayToDisplayDoc {
            fn __displaydoc_display(&self) -> Self;
        }
        impl<T: ::core::fmt::Display> DisplayToDisplayDoc for &T {
            fn __displaydoc_display(&self) -> Self {
                self
            }
        }
        extern crate std;
        trait PathToDisplayDoc {
            fn __displaydoc_display(&self) -> std::path::Display<'_>;
        }
        impl PathToDisplayDoc for std::path::Path {
            fn __displaydoc_display(&self) -> std::path::Display<'_> {
                self.display()
            }
        }
        impl PathToDisplayDoc for std::path::PathBuf {
            fn __displaydoc_display(&self) -> std::path::Display<'_> {
                self.display()
            }
        }
        impl ::core::fmt::Display for ErrorKind {
            fn fmt(
                &self,
                formatter: &mut ::core::fmt::Formatter,
            ) -> ::core::fmt::Result {
                #[allow(unused_variables)]
                match self {
                    Self::Backend => formatter.write_fmt(format_args!("Backend Error")),
                    Self::Io => formatter.write_fmt(format_args!("IO Error")),
                    Self::Password => formatter.write_fmt(format_args!("Password Error")),
                    Self::Repository => {
                        formatter.write_fmt(format_args!("Repository Error"))
                    }
                    Self::Command => formatter.write_fmt(format_args!("Command Error")),
                    Self::Config => formatter.write_fmt(format_args!("Config Error")),
                    Self::Index => formatter.write_fmt(format_args!("Index Error")),
                    Self::Key => formatter.write_fmt(format_args!("Key Error")),
                    Self::Blob => formatter.write_fmt(format_args!("Blob Error")),
                    Self::Cryptography => {
                        formatter.write_fmt(format_args!("Crypto Error"))
                    }
                    Self::Compression => {
                        formatter.write_fmt(format_args!("Compression Error"))
                    }
                    Self::Parsing => formatter.write_fmt(format_args!("Parsing Error")),
                    Self::Conversion => {
                        formatter.write_fmt(format_args!("Conversion Error"))
                    }
                    Self::Permission => {
                        formatter.write_fmt(format_args!("Permission Error"))
                    }
                    Self::Polynomial => {
                        formatter.write_fmt(format_args!("Polynomial Error"))
                    }
                    Self::Multithreading => {
                        formatter.write_fmt(format_args!("Multithreading Error"))
                    }
                    Self::Processing => {
                        formatter.write_fmt(format_args!("Processing Error"))
                    }
                    Self::Unsupported => {
                        formatter.write_fmt(format_args!("Something is not supported"))
                    }
                    Self::ExternalCommand => {
                        formatter.write_fmt(format_args!("External Command"))
                    }
                }
            }
        }
    };
}

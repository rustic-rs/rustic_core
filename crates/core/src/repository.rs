// Note: we need a fully qualified Vec here for clap, see https://github.com/clap-rs/clap/issues/4481
#![allow(unused_qualifications)]

mod warm_up;

use std::{
    cmp::Ordering,
    fs::File,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
};

use bytes::Bytes;
use derive_setters::Setters;
use log::{debug, error, info};
use serde_with::{serde_as, DisplayFromStr, OneOrMany};

use crate::{
    backend::{
        cache::{Cache, CachedBackend},
        decrypt::{DecryptBackend, DecryptReadBackend, DecryptWriteBackend},
        hotcold::HotColdBackend,
        local_destination::LocalDestination,
        node::Node,
        warm_up::WarmUpAccessBackend,
        FileType, ReadBackend, WriteBackend,
    },
    blob::{
        tree::{FindMatches, FindNode, NodeStreamer, TreeStreamerOptions as LsOptions},
        BlobType,
    },
    commands::{
        self,
        backup::BackupOptions,
        check::CheckOptions,
        config::ConfigOptions,
        copy::CopySnapshot,
        forget::{ForgetGroups, KeepOptions},
        key::KeyOptions,
        prune::{PruneOptions, PrunePlan},
        repair::{
            index::{index_checked_from_collector, RepairIndexOptions},
            snapshots::RepairSnapshotsOptions,
        },
        repoinfo::{IndexInfos, RepoFileInfos},
        restore::{RestoreOptions, RestorePlan},
    },
    crypto::aespoly1305::Key,
    error::{CommandErrorKind, KeyFileErrorKind, RepositoryErrorKind, RusticErrorKind},
    id::Id,
    index::{
        binarysorted::{IndexCollector, IndexType},
        GlobalIndex, IndexEntry, ReadGlobalIndex, ReadIndex,
    },
    progress::{NoProgressBars, Progress, ProgressBars},
    repofile::{
        keyfile::find_key_in_backend,
        snapshotfile::{SnapshotGroup, SnapshotGroupCriterion},
        ConfigFile, PathList, RepoFile, SnapshotFile, SnapshotSummary, Tree,
    },
    repository::{warm_up::warm_up, warm_up::warm_up_wait},
    vfs::OpenFile,
    RepositoryBackends, RusticResult,
};

#[cfg(feature = "clap")]
use clap::ValueHint;

mod constants {
    /// Estimated item capacity used for cache in [`FullIndex`](super::FullIndex)
    pub(super) const ESTIMATED_ITEM_CAPACITY: usize = 32;

    /// Estimated weight capacity used for cache in [`FullIndex`](super::FullIndex) (in bytes)
    pub(super) const WEIGHT_CAPACITY: u64 = 32_000_000;
}

/// Options for using and opening a [`Repository`]
#[serde_as]
#[cfg_attr(feature = "clap", derive(clap::Parser))]
#[cfg_attr(feature = "merge", derive(merge::Merge))]
#[derive(Clone, Default, Debug, serde::Deserialize, serde::Serialize, Setters)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
#[setters(into, strip_option)]
pub struct RepositoryOptions {
    /// Password of the repository
    ///
    /// # Warning
    ///
    /// Using --password can reveal the password in the process list!
    #[cfg_attr(
        feature = "clap",
        clap(long, global = true, env = "RUSTIC_PASSWORD", hide_env_values = true)
    )]
    // TODO: Security related: use `secrecy` library (#663)
    pub password: Option<String>,

    /// File to read the password from
    #[cfg_attr(
        feature = "clap",
        clap(
            short,
            long,
            global = true,
            env = "RUSTIC_PASSWORD_FILE",
            conflicts_with = "password",
            value_hint = ValueHint::FilePath,
        )
    )]
    pub password_file: Option<PathBuf>,

    /// Command to read the password from. Password is read from stdout
    #[cfg_attr(feature = "clap", clap(
        long,
        global = true,
        env = "RUSTIC_PASSWORD_COMMAND",
        conflicts_with_all = &["password", "password_file"],
        value_parser = clap::builder::ValueParser::new(shell_words::split),
        default_value = "",
    ))]
    #[cfg_attr(feature = "merge", merge(strategy = merge::vec::overwrite_empty))]
    #[serde_as(as = "OneOrMany<_>")]
    // Note: we need a fully qualified Vec here for clap, see https://github.com/clap-rs/clap/issues/4481
    pub password_command: std::vec::Vec<String>,

    /// Don't use a cache.
    #[cfg_attr(feature = "clap", clap(long, global = true, env = "RUSTIC_NO_CACHE"))]
    #[cfg_attr(feature = "merge", merge(strategy = merge::bool::overwrite_false))]
    pub no_cache: bool,

    /// Use this dir as cache dir instead of the standard cache dir
    #[cfg_attr(
        feature = "clap",
        clap(
            long,
            global = true,
            conflicts_with = "no_cache",
            env = "RUSTIC_CACHE_DIR",
            value_hint = ValueHint::DirPath,
        )
    )]
    pub cache_dir: Option<PathBuf>,

    /// Warm up needed data pack files by only requesting them without processing
    #[cfg_attr(feature = "clap", clap(long, global = true))]
    #[cfg_attr(feature = "merge", merge(strategy = merge::bool::overwrite_false))]
    pub warm_up: bool,

    /// Warm up needed data pack files by running the command with %id replaced by pack id
    #[cfg_attr(feature = "clap", clap(
        long,
        global = true,
        conflicts_with = "warm_up", 
        value_parser = clap::builder::ValueParser::new(shell_words::split),
        action = clap::ArgAction::Set,
        default_value = "",
    ))]
    #[cfg_attr(feature = "merge", merge(strategy = merge::vec::overwrite_empty))]
    #[serde_as(as = "OneOrMany<_>")]
    // Note: we need a fully qualified Vec here for clap, see https://github.com/clap-rs/clap/issues/4481
    pub warm_up_command: std::vec::Vec<String>,

    /// Duration (e.g. 10m) to wait after warm up
    #[cfg_attr(feature = "clap", clap(long, global = true, value_name = "DURATION"))]
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub warm_up_wait: Option<humantime::Duration>,
}

impl RepositoryOptions {
    /// Evaluates the password given by the repository options
    ///
    /// # Errors
    ///
    /// * [`RepositoryErrorKind::OpeningPasswordFileFailed`] - If opening the password file failed
    /// * [`RepositoryErrorKind::ReadingPasswordFromReaderFailed`] - If reading the password failed
    /// * [`RepositoryErrorKind::FromSplitError`] - If splitting the password command failed
    /// * [`RepositoryErrorKind::PasswordCommandParsingFailed`] - If parsing the password command failed
    /// * [`RepositoryErrorKind::ReadingPasswordFromCommandFailed`] - If reading the password from the command failed
    ///
    /// # Returns
    ///
    /// The password or `None` if no password is given
    ///
    /// [`RepositoryErrorKind::OpeningPasswordFileFailed`]: crate::error::RepositoryErrorKind::OpeningPasswordFileFailed
    /// [`RepositoryErrorKind::ReadingPasswordFromReaderFailed`]: crate::error::RepositoryErrorKind::ReadingPasswordFromReaderFailed
    /// [`RepositoryErrorKind::FromSplitError`]: crate::error::RepositoryErrorKind::FromSplitError
    /// [`RepositoryErrorKind::PasswordCommandParsingFailed`]: crate::error::RepositoryErrorKind::PasswordCommandParsingFailed
    /// [`RepositoryErrorKind::ReadingPasswordFromCommandFailed`]: crate::error::RepositoryErrorKind::ReadingPasswordFromCommandFailed
    pub fn evaluate_password(&self) -> RusticResult<Option<String>> {
        match (&self.password, &self.password_file, &self.password_command) {
            (Some(pwd), _, _) => Ok(Some(pwd.clone())),
            (_, Some(file), _) => {
                let mut file = BufReader::new(
                    File::open(file).map_err(RepositoryErrorKind::OpeningPasswordFileFailed)?,
                );
                Ok(Some(read_password_from_reader(&mut file)?))
            }
            (_, _, command) if !command.is_empty() => {
                debug!("commands: {command:?}");
                let command = Command::new(&command[0])
                    .args(&command[1..])
                    .stdout(Stdio::piped())
                    .spawn()?;
                let Ok(output) = command.wait_with_output() else {
                    return Err(RepositoryErrorKind::PasswordCommandParsingFailed.into());
                };
                if !output.status.success() {
                    #[allow(clippy::option_if_let_else)]
                    let s = match output.status.code() {
                        Some(c) => format!("exited with status code {c}"),
                        None => "was terminated".into(),
                    };
                    error!("password-command {s}");
                    return Err(RepositoryErrorKind::ReadingPasswordFromCommandFailed.into());
                }

                let mut pwd = BufReader::new(&*output.stdout);
                Ok(Some(match read_password_from_reader(&mut pwd) {
                    Ok(val) => val,
                    Err(_) => {
                        return Err(RepositoryErrorKind::ReadingPasswordFromCommandFailed.into())
                    }
                }))
            }
            (None, None, _) => Ok(None),
        }
    }
}

/// Read a password from a reader
///
/// # Arguments
///
/// * `file` - The reader to read the password from
///
/// # Errors
///
/// * [`RepositoryErrorKind::ReadingPasswordFromReaderFailed`] - If reading the password failed
///
/// [`RepositoryErrorKind::ReadingPasswordFromReaderFailed`]: crate::error::RepositoryErrorKind::ReadingPasswordFromReaderFailed
pub fn read_password_from_reader(file: &mut impl BufRead) -> RusticResult<String> {
    let mut password = String::new();
    _ = file
        .read_line(&mut password)
        .map_err(RepositoryErrorKind::ReadingPasswordFromReaderFailed)?;

    // Remove the \n from the line if present
    if password.ends_with('\n') {
        _ = password.pop();
    }

    // Remove the \r from the line if present
    if password.ends_with('\r') {
        _ = password.pop();
    }

    Ok(password)
}

#[derive(Debug, Clone)]
/// A `Repository` allows all kind of actions to be performed.
///
/// # Type Parameters
///
/// * `P` - The type of the progress bar
/// * `S` - The type of the status
///
/// # Notes
///
/// A repository can be in different states and allows some actions only when in certain state(s).
pub struct Repository<P, S> {
    /// The name of the repository
    pub name: String,

    /// The `HotColdBackend` to use for this repository
    pub(crate) be: Arc<dyn WriteBackend>,

    /// The Backend to use for hot files
    pub(crate) be_hot: Option<Arc<dyn WriteBackend>>,

    /// The options used for this repository
    opts: RepositoryOptions,

    /// The progress bar to use
    pub(crate) pb: P,

    /// The status
    status: S,
}

impl Repository<NoProgressBars, ()> {
    /// Create a new repository from the given [`RepositoryOptions`] (without progress bars)
    ///
    /// # Arguments
    ///
    /// * `opts` - The options to use for the repository
    /// * `backends` - The backends to create/access a repository on
    ///
    /// # Errors
    ///
    /// * [`RepositoryErrorKind::NoRepositoryGiven`] - If no repository is given
    /// * [`RepositoryErrorKind::NoIDSpecified`] - If the warm-up command does not contain `%id`
    /// * [`BackendAccessErrorKind::BackendLoadError`] - If the specified backend cannot be loaded, e.g. is not supported
    ///
    /// [`BackendAccessErrorKind::BackendLoadError`]: crate::error::BackendAccessErrorKind::BackendLoadError
    pub fn new(opts: &RepositoryOptions, backends: &RepositoryBackends) -> RusticResult<Self> {
        Self::new_with_progress(opts, backends, NoProgressBars {})
    }
}

impl<P> Repository<P, ()> {
    /// Create a new repository from the given [`RepositoryOptions`] with given progress bars
    ///
    /// # Type Parameters
    ///
    /// * `P` - The type of the progress bar
    ///
    /// # Arguments
    ///
    /// * `opts` - The options to use for the repository
    /// * `backends` - The backends to create/access a repository on
    /// * `pb` - The progress bars to use
    ///
    /// # Errors
    ///
    /// * [`RepositoryErrorKind::NoRepositoryGiven`] - If no repository is given
    /// * [`RepositoryErrorKind::NoIDSpecified`] - If the warm-up command does not contain `%id`
    /// * [`BackendAccessErrorKind::BackendLoadError`] - If the specified backend cannot be loaded, e.g. is not supported
    ///
    /// [`RepositoryErrorKind::NoRepositoryGiven`]: crate::error::RepositoryErrorKind::NoRepositoryGiven
    /// [`RepositoryErrorKind::NoIDSpecified`]: crate::error::RepositoryErrorKind::NoIDSpecified
    /// [`BackendAccessErrorKind::BackendLoadError`]: crate::error::BackendAccessErrorKind::BackendLoadError
    pub fn new_with_progress(
        opts: &RepositoryOptions,
        backends: &RepositoryBackends,
        pb: P,
    ) -> RusticResult<Self> {
        let mut be = backends.repository();
        let be_hot = backends.repo_hot();

        if !opts.warm_up_command.is_empty() {
            if opts.warm_up_command.iter().all(|c| !c.contains("%id")) {
                return Err(RepositoryErrorKind::NoIDSpecified.into());
            }
            info!("using warm-up command {:?}", opts.warm_up_command);
        }

        if opts.warm_up {
            be = WarmUpAccessBackend::new_warm_up(be);
        }

        let mut name = be.location();
        if let Some(be_hot) = &be_hot {
            be = Arc::new(HotColdBackend::new(be, be_hot.clone()));
            name.push('#');
            name.push_str(&be_hot.location());
        }

        Ok(Self {
            name,
            be,
            be_hot,
            opts: opts.clone(),
            pb,
            status: (),
        })
    }
}

impl<P, S> Repository<P, S> {
    /// Evaluates the password given by the repository options
    ///
    /// # Errors
    ///
    /// * [`RepositoryErrorKind::OpeningPasswordFileFailed`] - If opening the password file failed
    /// * [`RepositoryErrorKind::ReadingPasswordFromReaderFailed`] - If reading the password failed
    /// * [`RepositoryErrorKind::FromSplitError`] - If splitting the password command failed
    /// * [`RepositoryErrorKind::PasswordCommandParsingFailed`] - If parsing the password command failed
    /// * [`RepositoryErrorKind::ReadingPasswordFromCommandFailed`] - If reading the password from the command failed
    ///
    /// # Returns
    ///
    /// The password or `None` if no password is given
    ///
    /// [`RepositoryErrorKind::OpeningPasswordFileFailed`]: crate::error::RepositoryErrorKind::OpeningPasswordFileFailed
    /// [`RepositoryErrorKind::ReadingPasswordFromReaderFailed`]: crate::error::RepositoryErrorKind::ReadingPasswordFromReaderFailed
    /// [`RepositoryErrorKind::FromSplitError`]: crate::error::RepositoryErrorKind::FromSplitError
    /// [`RepositoryErrorKind::PasswordCommandParsingFailed`]: crate::error::RepositoryErrorKind::PasswordCommandParsingFailed
    /// [`RepositoryErrorKind::ReadingPasswordFromCommandFailed`]: crate::error::RepositoryErrorKind::ReadingPasswordFromCommandFailed
    pub fn password(&self) -> RusticResult<Option<String>> {
        self.opts.evaluate_password()
    }

    /// Returns the Id of the config file
    ///
    /// # Errors
    ///
    /// * [`RepositoryErrorKind::ListingRepositoryConfigFileFailed`] - If listing the repository config file failed
    /// * [`RepositoryErrorKind::MoreThanOneRepositoryConfig`] - If there is more than one repository config file
    ///
    /// # Returns
    ///
    /// The id of the config file or `None` if no config file is found
    ///
    /// [`RepositoryErrorKind::ListingRepositoryConfigFileFailed`]: crate::error::RepositoryErrorKind::ListingRepositoryConfigFileFailed
    /// [`RepositoryErrorKind::MoreThanOneRepositoryConfig`]: crate::error::RepositoryErrorKind::MoreThanOneRepositoryConfig
    pub fn config_id(&self) -> RusticResult<Option<Id>> {
        let config_ids = self
            .be
            .list(FileType::Config)
            .map_err(|_| RepositoryErrorKind::ListingRepositoryConfigFileFailed)?;

        match config_ids.len() {
            1 => Ok(Some(config_ids[0])),
            0 => Ok(None),
            _ => Err(RepositoryErrorKind::MoreThanOneRepositoryConfig(self.name.clone()).into()),
        }
    }

    /// Open the repository.
    ///
    /// This gets the decryption key and reads the config file
    ///
    /// # Errors
    ///
    /// * [`RepositoryErrorKind::NoPasswordGiven`] - If no password is given
    /// * [`RepositoryErrorKind::ReadingPasswordFromReaderFailed`] - If reading the password failed
    /// * [`RepositoryErrorKind::OpeningPasswordFileFailed`] - If opening the password file failed
    /// * [`RepositoryErrorKind::PasswordCommandParsingFailed`] - If parsing the password command failed
    /// * [`RepositoryErrorKind::ReadingPasswordFromCommandFailed`] - If reading the password from the command failed
    /// * [`RepositoryErrorKind::FromSplitError`] - If splitting the password command failed
    /// * [`RepositoryErrorKind::NoRepositoryConfigFound`] - If no repository config file is found
    /// * [`RepositoryErrorKind::KeysDontMatchForRepositories`] - If the keys of the hot and cold backend don't match
    /// * [`RepositoryErrorKind::IncorrectPassword`] - If the password is incorrect
    /// * [`KeyFileErrorKind::NoSuitableKeyFound`] - If no suitable key is found
    /// * [`RepositoryErrorKind::ListingRepositoryConfigFileFailed`] - If listing the repository config file failed
    /// * [`RepositoryErrorKind::MoreThanOneRepositoryConfig`] - If there is more than one repository config file
    ///
    /// # Returns
    ///
    /// The open repository
    ///
    /// [`RepositoryErrorKind::NoPasswordGiven`]: crate::error::RepositoryErrorKind::NoPasswordGiven
    /// [`RepositoryErrorKind::ReadingPasswordFromReaderFailed`]: crate::error::RepositoryErrorKind::ReadingPasswordFromReaderFailed
    /// [`RepositoryErrorKind::OpeningPasswordFileFailed`]: crate::error::RepositoryErrorKind::OpeningPasswordFileFailed
    /// [`RepositoryErrorKind::PasswordCommandParsingFailed`]: crate::error::RepositoryErrorKind::PasswordCommandParsingFailed
    /// [`RepositoryErrorKind::ReadingPasswordFromCommandFailed`]: crate::error::RepositoryErrorKind::ReadingPasswordFromCommandFailed
    /// [`RepositoryErrorKind::FromSplitError`]: crate::error::RepositoryErrorKind::FromSplitError
    /// [`RepositoryErrorKind::NoRepositoryConfigFound`]: crate::error::RepositoryErrorKind::NoRepositoryConfigFound
    /// [`RepositoryErrorKind::KeysDontMatchForRepositories`]: crate::error::RepositoryErrorKind::KeysDontMatchForRepositories
    /// [`RepositoryErrorKind::IncorrectPassword`]: crate::error::RepositoryErrorKind::IncorrectPassword
    /// [`KeyFileErrorKind::NoSuitableKeyFound`]: crate::error::KeyFileErrorKind::NoSuitableKeyFound
    /// [`RepositoryErrorKind::ListingRepositoryConfigFileFailed`]: crate::error::RepositoryErrorKind::ListingRepositoryConfigFileFailed
    /// [`RepositoryErrorKind::MoreThanOneRepositoryConfig`]: crate::error::RepositoryErrorKind::MoreThanOneRepositoryConfig
    pub fn open(self) -> RusticResult<Repository<P, OpenStatus>> {
        let password = self
            .password()?
            .ok_or(RepositoryErrorKind::NoPasswordGiven)?;
        self.open_with_password(&password)
    }

    /// Open the repository with a given password.
    ///
    /// This gets the decryption key and reads the config file
    ///
    /// # Arguments
    ///
    /// * `password` - The password to use
    ///
    /// # Errors
    ///
    /// * [`RepositoryErrorKind::NoRepositoryConfigFound`] - If no repository config file is found
    /// * [`RepositoryErrorKind::KeysDontMatchForRepositories`] - If the keys of the hot and cold backend don't match
    /// * [`RepositoryErrorKind::IncorrectPassword`] - If the password is incorrect
    /// * [`KeyFileErrorKind::NoSuitableKeyFound`] - If no suitable key is found
    /// * [`RepositoryErrorKind::ListingRepositoryConfigFileFailed`] - If listing the repository config file failed
    /// * [`RepositoryErrorKind::MoreThanOneRepositoryConfig`] - If there is more than one repository config file
    ///
    /// [`RepositoryErrorKind::NoRepositoryConfigFound`]: crate::error::RepositoryErrorKind::NoRepositoryConfigFound
    /// [`RepositoryErrorKind::KeysDontMatchForRepositories`]: crate::error::RepositoryErrorKind::KeysDontMatchForRepositories
    /// [`RepositoryErrorKind::IncorrectPassword`]: crate::error::RepositoryErrorKind::IncorrectPassword
    /// [`KeyFileErrorKind::NoSuitableKeyFound`]: crate::error::KeyFileErrorKind::NoSuitableKeyFound
    /// [`RepositoryErrorKind::ListingRepositoryConfigFileFailed`]: crate::error::RepositoryErrorKind::ListingRepositoryConfigFileFailed
    /// [`RepositoryErrorKind::MoreThanOneRepositoryConfig`]: crate::error::RepositoryErrorKind::MoreThanOneRepositoryConfig
    pub fn open_with_password(self, password: &str) -> RusticResult<Repository<P, OpenStatus>> {
        let config_id = self
            .config_id()?
            .ok_or(RepositoryErrorKind::NoRepositoryConfigFound(
                self.name.clone(),
            ))?;

        if let Some(be_hot) = &self.be_hot {
            let mut keys = self
                .be
                .list_with_size(FileType::Key)
                .map_err(RusticErrorKind::Backend)?;
            keys.sort_unstable_by_key(|key| key.0);
            let mut hot_keys = be_hot
                .list_with_size(FileType::Key)
                .map_err(RusticErrorKind::Backend)?;
            hot_keys.sort_unstable_by_key(|key| key.0);
            if keys != hot_keys {
                return Err(RepositoryErrorKind::KeysDontMatchForRepositories(self.name).into());
            }
        }

        let key = find_key_in_backend(&self.be, &password, None).map_err(|err| {
            match err.into_inner() {
                RusticErrorKind::KeyFile(KeyFileErrorKind::NoSuitableKeyFound) => {
                    RepositoryErrorKind::IncorrectPassword.into()
                }
                err => err,
            }
        })?;
        info!("repository {}: password is correct.", self.name);
        let dbe = DecryptBackend::new(self.be.clone(), key);
        let config: ConfigFile = dbe.get_file(&config_id)?;
        self.open_raw(key, config)
    }

    /// Initialize a new repository with given options using the password defined in `RepositoryOptions`
    ///
    /// This returns an open repository which can be directly used.
    ///
    /// # Type Parameters
    ///
    /// * `P` - The type of the progress bar
    ///
    /// # Arguments
    ///
    /// * `key_opts` - The options to use for the key
    /// * `config_opts` - The options to use for the config
    ///
    /// # Errors
    ///
    /// * [`RepositoryErrorKind::NoPasswordGiven`] - If no password is given
    /// * [`RepositoryErrorKind::ReadingPasswordFromReaderFailed`] - If reading the password failed
    /// * [`RepositoryErrorKind::OpeningPasswordFileFailed`] - If opening the password file failed
    /// * [`RepositoryErrorKind::PasswordCommandParsingFailed`] - If parsing the password command failed
    /// * [`RepositoryErrorKind::ReadingPasswordFromCommandFailed`] - If reading the password from the command failed
    /// * [`RepositoryErrorKind::FromSplitError`] - If splitting the password command failed
    ///
    /// [`RepositoryErrorKind::NoPasswordGiven`]: crate::error::RepositoryErrorKind::NoPasswordGiven
    /// [`RepositoryErrorKind::ReadingPasswordFromReaderFailed`]: crate::error::RepositoryErrorKind::ReadingPasswordFromReaderFailed
    /// [`RepositoryErrorKind::OpeningPasswordFileFailed`]: crate::error::RepositoryErrorKind::OpeningPasswordFileFailed
    /// [`RepositoryErrorKind::PasswordCommandParsingFailed`]: crate::error::RepositoryErrorKind::PasswordCommandParsingFailed
    /// [`RepositoryErrorKind::ReadingPasswordFromCommandFailed`]: crate::error::RepositoryErrorKind::ReadingPasswordFromCommandFailed
    /// [`RepositoryErrorKind::FromSplitError`]: crate::error::RepositoryErrorKind::FromSplitError
    pub fn init(
        self,
        key_opts: &KeyOptions,
        config_opts: &ConfigOptions,
    ) -> RusticResult<Repository<P, OpenStatus>> {
        let password = self
            .password()?
            .ok_or(RepositoryErrorKind::NoPasswordGiven)?;
        self.init_with_password(&password, key_opts, config_opts)
    }

    /// Initialize a new repository with given password and options.
    ///
    /// This returns an open repository which can be directly used.
    ///
    /// # Type Parameters
    ///
    /// * `P` - The type of the progress bar
    ///
    /// # Arguments
    ///
    /// * `pass` - The password to use
    /// * `key_opts` - The options to use for the key
    /// * `config_opts` - The options to use for the config
    ///
    /// # Errors
    ///
    /// * [`RepositoryErrorKind::ConfigFileExists`] - If a config file already exists
    /// * [`RepositoryErrorKind::ListingRepositoryConfigFileFailed`] - If listing the repository config file failed
    /// * [`RepositoryErrorKind::MoreThanOneRepositoryConfig`] - If there is more than one repository config file
    ///
    /// [`RepositoryErrorKind::ConfigFileExists`]: crate::error::RepositoryErrorKind::ConfigFileExists
    /// [`RepositoryErrorKind::ListingRepositoryConfigFileFailed`]: crate::error::RepositoryErrorKind::ListingRepositoryConfigFileFailed
    /// [`RepositoryErrorKind::MoreThanOneRepositoryConfig`]: crate::error::RepositoryErrorKind::MoreThanOneRepositoryConfig
    pub fn init_with_password(
        self,
        pass: &str,
        key_opts: &KeyOptions,
        config_opts: &ConfigOptions,
    ) -> RusticResult<Repository<P, OpenStatus>> {
        if self.config_id()?.is_some() {
            return Err(RepositoryErrorKind::ConfigFileExists.into());
        }
        let (key, config) = commands::init::init(&self, pass, key_opts, config_opts)?;
        self.open_raw(key, config)
    }

    /// Initialize a new repository with given password and a ready [`ConfigFile`].
    ///
    /// This returns an open repository which can be directly used.
    ///
    /// # Type Parameters
    ///
    /// * `P` - The type of the progress bar
    ///
    /// # Arguments
    ///
    /// * `password` - The password to use
    /// * `key_opts` - The options to use for the key
    /// * `config` - The config file to use
    ///
    /// # Errors
    ///
    // TODO: Document errors
    pub fn init_with_config(
        self,
        password: &str,
        key_opts: &KeyOptions,
        config: ConfigFile,
    ) -> RusticResult<Repository<P, OpenStatus>> {
        let key = commands::init::init_with_config(&self, password, key_opts, &config)?;
        info!("repository {} successfully created.", config.id);
        self.open_raw(key, config)
    }

    /// Open the repository with given [`Key`] and [`ConfigFile`].
    ///
    /// # Type Parameters
    ///
    /// * `P` - The type of the progress bar
    ///
    /// # Arguments
    ///
    /// * `key` - The key to use
    /// * `config` - The config file to use
    ///
    /// # Errors
    ///
    /// * [`RepositoryErrorKind::HotRepositoryFlagMissing`] - If the config file has `is_hot` set to `true` but the repository is not hot
    /// * [`RepositoryErrorKind::IsNotHotRepository`] - If the config file has `is_hot` set to `false` but the repository is hot
    ///
    /// [`RepositoryErrorKind::HotRepositoryFlagMissing`]: crate::error::RepositoryErrorKind::HotRepositoryFlagMissing
    /// [`RepositoryErrorKind::IsNotHotRepository`]: crate::error::RepositoryErrorKind::IsNotHotRepository
    fn open_raw(mut self, key: Key, config: ConfigFile) -> RusticResult<Repository<P, OpenStatus>> {
        match (config.is_hot == Some(true), self.be_hot.is_some()) {
            (true, false) => return Err(RepositoryErrorKind::HotRepositoryFlagMissing.into()),
            (false, true) => return Err(RepositoryErrorKind::IsNotHotRepository.into()),
            _ => {}
        }

        let cache = (!self.opts.no_cache)
            .then(|| Cache::new(config.id, self.opts.cache_dir.clone()).ok())
            .flatten();

        if let Some(cache) = &cache {
            self.be = CachedBackend::new_cache(self.be.clone(), cache.clone());
            info!("using cache at {}", cache.location());
        } else {
            info!("using no cache");
        }

        let mut dbe = DecryptBackend::new(self.be.clone(), key);
        dbe.set_zstd(config.zstd()?);
        dbe.set_extra_verify(config.extra_verify());

        let open = OpenStatus { cache, dbe, config };

        Ok(Repository {
            name: self.name,
            be: self.be,
            be_hot: self.be_hot,
            opts: self.opts,
            pb: self.pb,
            status: open,
        })
    }

    /// List all file [`Id`]s of the given [`FileType`] which are present in the repository
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the files to list
    ///
    /// # Errors
    ///
    // TODO: Document errors
    pub fn list(&self, tpe: FileType) -> RusticResult<impl Iterator<Item = Id>> {
        Ok(self
            .be
            .list(tpe)
            .map_err(RusticErrorKind::Backend)?
            .into_iter())
    }
}

impl<P: ProgressBars, S> Repository<P, S> {
    /// Collect information about repository files
    ///
    /// # Errors
    ///
    /// If files could not be listed.
    pub fn infos_files(&self) -> RusticResult<RepoFileInfos> {
        commands::repoinfo::collect_file_infos(self)
    }

    /// Warm up the given pack files without waiting.
    ///
    /// # Arguments
    ///
    /// * `packs` - The pack files to warm up
    ///
    /// # Errors
    ///
    /// * [`RepositoryErrorKind::FromSplitError`] - If the command could not be parsed.
    /// * [`RepositoryErrorKind::FromThreadPoolbilderError`] - If the thread pool could not be created.
    ///
    /// # Returns
    ///
    /// The result of the warm up
    pub fn warm_up(&self, packs: impl ExactSizeIterator<Item = Id>) -> RusticResult<()> {
        warm_up(self, packs)
    }

    /// Warm up the given pack files and wait the configured waiting time.
    ///
    /// # Arguments
    ///
    /// * `packs` - The pack files to warm up
    ///
    /// # Errors
    ///
    /// * [`RepositoryErrorKind::FromSplitError`] - If the command could not be parsed.
    /// * [`RepositoryErrorKind::FromThreadPoolbilderError`] - If the thread pool could not be created.
    ///
    /// [`RepositoryErrorKind::FromSplitError`]: crate::error::RepositoryErrorKind::FromSplitError
    /// [`RepositoryErrorKind::FromThreadPoolbilderError`]: crate::error::RepositoryErrorKind::FromThreadPoolbilderError
    pub fn warm_up_wait(&self, packs: impl ExactSizeIterator<Item = Id>) -> RusticResult<()> {
        warm_up_wait(self, packs)
    }
}

/// A repository which is open, i.e. the password has been checked and the decryption key is available.
pub trait Open {
    /// Get the cache
    fn cache(&self) -> Option<&Cache>;

    /// Get the [`DecryptBackend`]
    fn dbe(&self) -> &DecryptBackend<Key>;

    /// Get the [`ConfigFile`]
    fn config(&self) -> &ConfigFile;
}

impl<P, S: Open> Open for Repository<P, S> {
    /// Get the cache
    fn cache(&self) -> Option<&Cache> {
        self.status.cache()
    }

    /// Get the [`DecryptBackend`]
    fn dbe(&self) -> &DecryptBackend<Key> {
        self.status.dbe()
    }

    /// Get the [`ConfigFile`]
    fn config(&self) -> &ConfigFile {
        self.status.config()
    }
}

/// Open Status: This repository is open, i.e. the password has been checked and the decryption key is available.
#[derive(Debug)]
pub struct OpenStatus {
    /// The cache
    cache: Option<Cache>,
    /// The [`DecryptBackend`]
    dbe: DecryptBackend<Key>,
    /// The [`ConfigFile`]
    config: ConfigFile,
}

impl Open for OpenStatus {
    /// Get the cache
    fn cache(&self) -> Option<&Cache> {
        self.cache.as_ref()
    }

    /// Get the [`DecryptBackend`]
    fn dbe(&self) -> &DecryptBackend<Key> {
        &self.dbe
    }

    /// Get the [`ConfigFile`]
    fn config(&self) -> &ConfigFile {
        &self.config
    }
}

impl<P, S: Open> Repository<P, S> {
    /// Get the content of the decrypted repository file given by id and [`FileType`]
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the file to get
    /// * `id` - The id of the file to get
    ///
    /// # Errors
    ///
    /// * [`IdErrorKind::HexError`] - If the string is not a valid hexadecimal string
    /// * [`BackendAccessErrorKind::NoSuitableIdFound`] - If no id could be found.
    /// * [`BackendAccessErrorKind::IdNotUnique`] - If the id is not unique.
    ///
    /// [`IdErrorKind::HexError`]: crate::error::IdErrorKind::HexError
    /// [`BackendAccessErrorKind::NoSuitableIdFound`]: crate::error::BackendAccessErrorKind::NoSuitableIdFound
    /// [`BackendAccessErrorKind::IdNotUnique`]: crate::error::BackendAccessErrorKind::IdNotUnique
    pub fn cat_file(&self, tpe: FileType, id: &str) -> RusticResult<Bytes> {
        commands::cat::cat_file(self, tpe, id)
    }

    /// Add a new key to the repository
    ///
    /// # Arguments
    ///
    /// * `pass` - The password to use for the new key
    /// * `opts` - The options to use for the new key
    ///
    /// # Errors
    ///
    /// * [`CommandErrorKind::FromJsonError`] - If the key could not be serialized.
    ///
    /// [`CommandErrorKind::FromJsonError`]: crate::error::CommandErrorKind::FromJsonError
    pub fn add_key(&self, pass: &str, opts: &KeyOptions) -> RusticResult<Id> {
        opts.add_key(self, pass)
    }

    /// Update the repository config by applying the given [`ConfigOptions`]
    ///
    /// # Arguments
    ///
    /// * `opts` - The options to apply
    ///
    /// # Errors
    ///
    /// * [`CommandErrorKind::VersionNotSupported`] - If the version is not supported
    /// * [`CommandErrorKind::CannotDowngrade`] - If the version is lower than the current version
    /// * [`CommandErrorKind::NoCompressionV1Repo`] - If compression is set for a v1 repo
    /// * [`CommandErrorKind::CompressionLevelNotSupported`] - If the compression level is not supported
    /// * [`CommandErrorKind::SizeTooLarge`] - If the size is too large
    /// * [`CommandErrorKind::MinPackSizeTolerateWrong`] - If the min packsize tolerance percent is wrong
    /// * [`CommandErrorKind::MaxPackSizeTolerateWrong`] - If the max packsize tolerance percent is wrong
    /// * [`CryptBackendErrorKind::SerializingToJsonByteVectorFailed`] - If the file could not be serialized to json.
    ///
    /// [`CommandErrorKind::VersionNotSupported`]: crate::error::CommandErrorKind::VersionNotSupported
    /// [`CommandErrorKind::CannotDowngrade`]: crate::error::CommandErrorKind::CannotDowngrade
    /// [`CommandErrorKind::NoCompressionV1Repo`]: crate::error::CommandErrorKind::NoCompressionV1Repo
    /// [`CommandErrorKind::CompressionLevelNotSupported`]: crate::error::CommandErrorKind::CompressionLevelNotSupported
    /// [`CommandErrorKind::SizeTooLarge`]: crate::error::CommandErrorKind::SizeTooLarge
    /// [`CommandErrorKind::MinPackSizeTolerateWrong`]: crate::error::CommandErrorKind::MinPackSizeTolerateWrong
    /// [`CommandErrorKind::MaxPackSizeTolerateWrong`]: crate::error::CommandErrorKind::MaxPackSizeTolerateWrong
    /// [`CryptBackendErrorKind::SerializingToJsonByteVectorFailed`]: crate::error::CryptBackendErrorKind::SerializingToJsonByteVectorFailed
    pub fn apply_config(&self, opts: &ConfigOptions) -> RusticResult<bool> {
        commands::config::apply_config(self, opts)
    }

    /// Get the repository configuration
    pub fn config(&self) -> &ConfigFile {
        self.status.config()
    }

    // TODO: add documentation!
    pub(crate) fn dbe(&self) -> &DecryptBackend<Key> {
        self.status.dbe()
    }
}

impl<P: ProgressBars, S: Open> Repository<P, S> {
    /// Get grouped snapshots.
    ///
    /// # Arguments
    ///
    /// * `ids` - The ids of the snapshots to group. If empty, all snapshots are grouped.
    /// * `group_by` - The criterion to group by
    /// * `filter` - The filter to use
    ///
    /// # Errors
    ///
    // TODO: Document errors
    ///
    /// # Returns
    ///
    /// If `ids` are given, this will try to resolve the ids (or `latest` with respect to the given filter) and return a single group
    /// If `ids` is empty, return and group all snapshots respecting the filter.
    pub fn get_snapshot_group(
        &self,
        ids: &[String],
        group_by: SnapshotGroupCriterion,
        filter: impl FnMut(&SnapshotFile) -> bool,
    ) -> RusticResult<Vec<(SnapshotGroup, Vec<SnapshotFile>)>> {
        commands::snapshots::get_snapshot_group(self, ids, group_by, filter)
    }

    /// Get a single snapshot
    ///
    /// # Arguments
    ///
    /// * `id` - The id of the snapshot to get
    /// * `filter` - The filter to use
    ///
    /// # Errors
    ///
    /// * [`IdErrorKind::HexError`] - If the string is not a valid hexadecimal string
    /// * [`BackendAccessErrorKind::NoSuitableIdFound`] - If no id could be found.
    /// * [`BackendAccessErrorKind::IdNotUnique`] - If the id is not unique.
    ///
    /// # Returns
    ///
    /// If `id` is (part of) an `Id`, return this snapshot.
    /// If `id` is "latest", return the latest snapshot respecting the giving filter.
    ///
    /// [`IdErrorKind::HexError`]: crate::error::IdErrorKind::HexError
    /// [`BackendAccessErrorKind::NoSuitableIdFound`]: crate::error::BackendAccessErrorKind::NoSuitableIdFound
    /// [`BackendAccessErrorKind::IdNotUnique`]: crate::error::BackendAccessErrorKind::IdNotUnique
    pub fn get_snapshot_from_str(
        &self,
        id: &str,
        filter: impl FnMut(&SnapshotFile) -> bool + Send + Sync,
    ) -> RusticResult<SnapshotFile> {
        let p = self.pb.progress_counter("getting snapshot...");
        let snap = SnapshotFile::from_str(self.dbe(), id, filter, &p)?;
        p.finish();
        Ok(snap)
    }

    /// Get the given snapshots.
    ///
    /// # Arguments
    ///
    /// * `ids` - The ids of the snapshots to get
    ///
    /// # Notes
    ///
    /// `ids` may contain part of snapshots id which will be resolved.
    /// However, "latest" is not supported in this function.
    ///
    /// # Errors
    ///
    // TODO: Document errors
    pub fn get_snapshots<T: AsRef<str>>(&self, ids: &[T]) -> RusticResult<Vec<SnapshotFile>> {
        let p = self.pb.progress_counter("getting snapshots...");
        let result = SnapshotFile::from_ids(self.dbe(), ids, &p);
        p.finish();
        result
    }

    /// Get all snapshots from the repository
    ///
    /// # Errors
    ///
    // TODO: Document errors
    pub fn get_all_snapshots(&self) -> RusticResult<Vec<SnapshotFile>> {
        self.get_matching_snapshots(|_| true)
    }

    /// Get all snapshots from the repository respecting the given `filter`
    ///
    /// # Arguments
    ///
    /// * `filter` - The filter to use
    ///
    /// # Errors
    ///
    /// # Note
    ///  The result is not sorted and may come in random order!
    ///
    // TODO: Document errors
    pub fn get_matching_snapshots(
        &self,
        filter: impl FnMut(&SnapshotFile) -> bool,
    ) -> RusticResult<Vec<SnapshotFile>> {
        let p = self.pb.progress_counter("getting snapshots...");
        let result = SnapshotFile::all_from_backend(self.dbe(), filter, &p);
        p.finish();
        result
    }

    /// Get snapshots to forget depending on the given [`KeepOptions`]
    ///
    /// # Arguments
    ///
    /// * `keep` - The keep options to use
    /// * `group_by` - The criterion to group by
    /// * `filter` - The filter to use
    ///
    /// # Errors
    ///
    /// If keep options are not valid
    ///
    /// # Returns
    ///
    /// The groups of snapshots to forget
    pub fn get_forget_snapshots(
        &self,
        keep: &KeepOptions,
        group_by: SnapshotGroupCriterion,
        filter: impl FnMut(&SnapshotFile) -> bool,
    ) -> RusticResult<ForgetGroups> {
        commands::forget::get_forget_snapshots(self, keep, group_by, filter)
    }

    /// Get snapshots which are not already present and should be present.
    ///
    /// # Arguments
    ///
    /// * `filter` - The filter to use
    /// * `snaps` - The snapshots to check
    ///
    /// # Errors
    ///
    // TODO: Document errors
    ///
    /// # Note
    ///
    /// This method should be called on the *destination repository*
    pub fn relevant_copy_snapshots(
        &self,
        filter: impl FnMut(&SnapshotFile) -> bool,
        snaps: &[SnapshotFile],
    ) -> RusticResult<Vec<CopySnapshot>> {
        commands::copy::relevant_snapshots(snaps, self, filter)
    }

    // TODO: Maybe only offer a method to remove &[Snapshotfile] and check if they must be kept.
    // See e.g. the merge command of the CLI
    /// Remove the given snapshots from the repository
    ///
    /// # Arguments
    ///
    /// * `ids` - The ids of the snapshots to remove
    ///
    /// # Errors
    ///
    // TODO: Document errors
    ///
    /// # Panics
    ///
    /// If the files could not be deleted.
    pub fn delete_snapshots(&self, ids: &[Id]) -> RusticResult<()> {
        if self.config().append_only == Some(true) {
            return Err(CommandErrorKind::NotAllowedWithAppendOnly(
                "snapshots removal".to_string(),
            )
            .into());
        }
        let p = self.pb.progress_counter("removing snapshots...");
        self.dbe()
            .delete_list(FileType::Snapshot, true, ids.iter(), p)?;
        Ok(())
    }

    /// Save the given snapshots to the repository.
    ///
    /// # Arguments
    ///
    /// * `snaps` - The snapshots to save
    ///
    /// # Errors
    ///
    /// * [`CryptBackendErrorKind::SerializingToJsonByteVectorFailed`] - If the file could not be serialized to json.
    ///
    /// [`CryptBackendErrorKind::SerializingToJsonByteVectorFailed`]: crate::error::CryptBackendErrorKind::SerializingToJsonByteVectorFailed
    pub fn save_snapshots(&self, mut snaps: Vec<SnapshotFile>) -> RusticResult<()> {
        for snap in &mut snaps {
            snap.id = Id::default();
        }
        let p = self.pb.progress_counter("saving snapshots...");
        self.dbe().save_list(snaps.iter(), p)?;
        Ok(())
    }

    /// Check the repository for errors or inconsistencies
    ///
    /// # Arguments
    ///
    /// * `opts` - The options to use
    ///
    /// # Errors
    ///
    // TODO: Document errors
    pub fn check(&self, opts: CheckOptions) -> RusticResult<()> {
        opts.run(self)
    }

    /// Get the plan about what should be pruned and/or repacked.
    ///
    /// # Arguments
    ///
    /// * `opts` - The options to use
    ///
    /// # Errors
    ///
    // TODO: Document errors
    ///
    /// # Returns
    ///
    /// The plan about what should be pruned and/or repacked.
    pub fn prune_plan(&self, opts: &PruneOptions) -> RusticResult<PrunePlan> {
        opts.get_plan(self)
    }

    /// Turn the repository into the `IndexedFull` state by reading and storing the index
    ///
    /// # Errors
    ///
    // TODO: Document errors
    ///
    /// # Note
    ///
    /// This saves the full index in memory which can be quite memory-consuming!
    pub fn to_indexed(self) -> RusticResult<Repository<P, IndexedStatus<FullIndex, S>>> {
        let index = GlobalIndex::new(self.dbe(), &self.pb.progress_counter(""))?;
        Ok(self.into_indexed_with_index(index))
    }

    /// Turn the repository into the `IndexedFull` state by reading and storing the index
    ///
    /// This is similar to `to_indexed()`, but also lists the pack files and reads pack headers
    /// for packs is missing in the index.
    ///
    /// # Errors
    ///
    // TODO: Document errors
    ///
    /// # Note
    ///
    /// This saves the full index in memory which can be quite memory-consuming!
    pub fn to_indexed_checked(self) -> RusticResult<Repository<P, IndexedStatus<FullIndex, S>>> {
        let collector = IndexCollector::new(IndexType::Full);
        let index = index_checked_from_collector(&self, collector)?;
        Ok(self.into_indexed_with_index(index))
    }

    // helper function to deduplicate code
    fn into_indexed_with_index(
        self,
        index: GlobalIndex,
    ) -> Repository<P, IndexedStatus<FullIndex, S>> {
        let status = IndexedStatus {
            open: self.status,
            index,
            index_data: FullIndex {
                // TODO: Make cache size (32MB currently) customizable!
                cache: quick_cache::sync::Cache::with_weighter(
                    constants::ESTIMATED_ITEM_CAPACITY,
                    constants::WEIGHT_CAPACITY,
                    BytesWeighter {},
                ),
            },
        };
        Repository {
            name: self.name,
            be: self.be,
            be_hot: self.be_hot,
            opts: self.opts,
            pb: self.pb,
            status,
        }
    }

    /// Turn the repository into the `IndexedIds` state by reading and storing a size-optimized index
    ///
    /// # Errors
    ///
    // TODO: Document errors
    ///
    /// # Returns
    ///
    /// The repository in the `IndexedIds` state
    ///
    /// # Note
    ///
    /// This saves only the `Id`s for data blobs. Therefore, not all operations are possible on the repository.
    /// However, operations which add data are fully functional.
    pub fn to_indexed_ids(self) -> RusticResult<Repository<P, IndexedStatus<IdIndex, S>>> {
        let index = GlobalIndex::only_full_trees(self.dbe(), &self.pb.progress_counter(""))?;
        Ok(self.into_indexed_ids_with_index(index))
    }

    /// Turn the repository into the `IndexedIds` state by reading and storing a size-optimized index
    ///
    /// This is similar to `to_indexed_ids()`, but also lists the pack files and reads pack headers
    /// for packs is missing in the index.
    ///
    /// # Errors
    ///
    // TODO: Document errors
    ///
    /// # Returns
    ///
    /// The repository in the `IndexedIds` state
    ///
    /// # Note
    ///
    /// This saves only the `Id`s for data blobs. Therefore, not all operations are possible on the repository.
    /// However, operations which add data are fully functional.
    pub fn to_indexed_ids_checked(self) -> RusticResult<Repository<P, IndexedStatus<IdIndex, S>>> {
        let collector = IndexCollector::new(IndexType::DataIds);
        let index = index_checked_from_collector(&self, collector)?;
        Ok(self.into_indexed_ids_with_index(index))
    }

    // helper function to deduplicate code
    fn into_indexed_ids_with_index(
        self,
        index: GlobalIndex,
    ) -> Repository<P, IndexedStatus<IdIndex, S>> {
        let status = IndexedStatus {
            open: self.status,
            index,
            index_data: IdIndex {},
        };
        Repository {
            name: self.name,
            be: self.be,
            be_hot: self.be_hot,
            opts: self.opts,
            pb: self.pb,
            status,
        }
    }

    /// Get statistical information from the index. This method reads all index files,
    /// even if an index is already available in memory.
    ///
    /// # Errors
    ///
    /// If the index could not be read.
    ///
    /// # Returns
    ///
    /// The statistical information from the index.
    pub fn infos_index(&self) -> RusticResult<IndexInfos> {
        commands::repoinfo::collect_index_infos(self)
    }

    /// Read all files of a given [`RepoFile`]
    ///
    /// # Errors
    ///
    // TODO: Document errors
    ///
    /// # Returns
    ///
    /// # Note
    ///  The result is not sorted and may come in random order!
    ///
    /// An iterator over all files of the given type
    pub fn stream_files<F: RepoFile>(
        &self,
    ) -> RusticResult<impl Iterator<Item = RusticResult<(Id, F)>>> {
        Ok(self
            .dbe()
            .stream_all::<F>(&self.pb.progress_hidden())?
            .into_iter())
    }

    /// Repair the index
    ///
    /// This compares the index with existing pack files and reads packfile headers to ensure the index
    /// correctly represents the pack files.
    ///
    /// # Arguments
    ///
    /// * `opts` - The options to use
    /// * `dry_run` - If true, only print what would be done
    ///
    /// # Errors
    ///
    // TODO: Document errors
    pub fn repair_index(&self, opts: &RepairIndexOptions, dry_run: bool) -> RusticResult<()> {
        opts.repair(self, dry_run)
    }
}

/// A repository which is indexed such that all tree blobs are contained in the index.
pub trait IndexedTree: Open {
    /// The used index
    type I: ReadGlobalIndex;

    /// Returns the used indexs
    fn index(&self) -> &Self::I;
}

/// A repository which is indexed such that all tree blobs are contained in the index
/// and additionally the `Id`s of data blobs are also contained in the index.
pub trait IndexedIds: IndexedTree {}

impl<P, S: IndexedTree> IndexedTree for Repository<P, S> {
    type I = S::I;

    fn index(&self) -> &Self::I {
        self.status.index()
    }
}

#[derive(Clone, Copy, Debug)]
/// Defines a weighted cache with weight equal to the length of the blob size
pub(crate) struct BytesWeighter;

impl quick_cache::Weighter<Id, Bytes> for BytesWeighter {
    fn weight(&self, _key: &Id, val: &Bytes) -> u64 {
        u64::try_from(val.len())
            .expect("weight overflow in cache should not happen")
            // Be cautions out about zero weights!
            .max(1)
    }
}

/// A repository which is indexed such that all blob information is fully contained in the index.
pub trait IndexedFull: IndexedIds {
    /// Get a blob from the internal cache blob or insert it with the given function
    ///
    /// # Arguments
    ///
    /// * `id` - The [`Id`] of the blob to get
    /// * `with` - The function which fetches the blob from the repository if it is not contained in the cache
    ///
    /// # Errors
    ///
    /// If the blob could not be fetched from the repository.
    ///
    /// # Returns
    ///
    /// The blob with the given id or the result of the given function if the blob is not contained in the cache
    /// and the function is called.
    fn get_blob_or_insert_with(
        &self,
        id: &Id,
        with: impl FnOnce() -> RusticResult<Bytes>,
    ) -> RusticResult<Bytes>;
}

/// The indexed status of a repository
///
/// # Type Parameters
///
/// * `T` - The type of index
/// * `S` - The type of the open status
#[derive(Debug)]
pub struct IndexedStatus<T, S: Open> {
    /// The index backend
    index: GlobalIndex,
    /// Additional index data used for the specific index status
    index_data: T,
    /// The open status
    open: S,
}

#[derive(Debug, Clone, Copy)]
/// A type of an index, that only contains [`Id`]s.
///
/// Used for the [`IndexedIds`] state of a repository in [`IndexedStatus`].
pub struct IdIndex;

#[derive(Debug)]
/// A full index containing [`Id`]s and locations for tree and data blobs.
///
/// As we usually use this to access data blobs from the repository, we also have defined a blob cache for
/// repositories with full index.
pub struct FullIndex {
    cache: quick_cache::sync::Cache<Id, Bytes, BytesWeighter>,
}

impl<T, S: Open> IndexedTree for IndexedStatus<T, S> {
    type I = GlobalIndex;

    fn index(&self) -> &Self::I {
        &self.index
    }
}

impl<S: Open> IndexedIds for IndexedStatus<IdIndex, S> {}

impl<S: Open> IndexedIds for IndexedStatus<FullIndex, S> {}

impl<P, S: IndexedFull> IndexedIds for Repository<P, S> {}

impl<S: Open> IndexedFull for IndexedStatus<FullIndex, S> {
    fn get_blob_or_insert_with(
        &self,
        id: &Id,
        with: impl FnOnce() -> RusticResult<Bytes>,
    ) -> RusticResult<Bytes> {
        self.index_data.cache.get_or_insert_with(id, with)
    }
}

impl<P, S: IndexedFull> IndexedFull for Repository<P, S> {
    /// Get a blob from the internal cache blob or insert it with the given function
    ///
    /// # Arguments
    ///
    /// * `id` - The [`Id`] of the blob to get
    /// * `with` - The function which fetches the blob from the repository if it is not contained in the cache
    fn get_blob_or_insert_with(
        &self,
        id: &Id,
        with: impl FnOnce() -> RusticResult<Bytes>,
    ) -> RusticResult<Bytes> {
        self.status.get_blob_or_insert_with(id, with)
    }
}

impl<T, S: Open> Open for IndexedStatus<T, S> {
    fn cache(&self) -> Option<&Cache> {
        self.open.cache()
    }

    fn dbe(&self) -> &DecryptBackend<Key> {
        self.open.dbe()
    }

    fn config(&self) -> &ConfigFile {
        self.open.config()
    }
}

impl<P, S: IndexedFull> Repository<P, S> {
    /// Get the [`IndexEntry`] of the given blob
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the blob
    /// * `id` - The id of the blob
    ///
    /// # Errors
    ///
    /// * [`RepositoryErrorKind::IdNotFound`] - If the id is not found in the index
    ///
    /// [`RepositoryErrorKind::IdNotFound`]: crate::error::RepositoryErrorKind::IdNotFound
    pub fn get_index_entry(&self, tpe: BlobType, id: &Id) -> RusticResult<IndexEntry> {
        let ie = self
            .index()
            .get_id(tpe, id)
            .ok_or_else(|| RepositoryErrorKind::IdNotFound(*id))?;
        Ok(ie)
    }

    /// Open a file in the repository for reading
    ///
    /// # Arguments
    ///
    /// * `node` - The node to open
    ///
    /// # Errors
    ///
    // TODO: Document errors
    pub fn open_file(&self, node: &Node) -> RusticResult<OpenFile> {
        Ok(OpenFile::from_node(self, node))
    }

    /// Reads an opened file at the given position
    ///
    /// # Arguments
    ///
    /// * `open_file` - The opened file
    /// * `offset` - The offset to start reading
    /// * `length` - The length to read
    ///
    /// # Errors
    ///
    // TODO: Document errors
    pub fn read_file_at(
        &self,
        open_file: &OpenFile,
        offset: usize,
        length: usize,
    ) -> RusticResult<Bytes> {
        open_file.read_at(self, offset, length)
    }
}

impl<P, S: IndexedTree> Repository<P, S> {
    /// Get a [`Tree`] by [`Id`] from the repository.
    ///
    /// # Arguments
    ///
    /// * `id` - The `Id` of the tree
    // TODO!: This ID should be a tree ID, we should refactor it to wrap it in a TreeId type
    ///
    /// # Errors
    ///
    /// * [`TreeErrorKind::BlobIdNotFound`] - If the tree ID is not found in the backend.
    /// * [`TreeErrorKind::DeserializingTreeFailed`] - If deserialization fails.
    ///
    /// # Returns
    ///
    /// The tree with the given `Id`
    ///
    /// [`TreeErrorKind::BlobIdNotFound`]: crate::error::TreeErrorKind::BlobIdNotFound
    /// [`TreeErrorKind::DeserializingTreeFailed`]: crate::error::TreeErrorKind::DeserializingTreeFailed
    pub fn get_tree(&self, id: &Id) -> RusticResult<Tree> {
        Tree::from_backend(self.dbe(), self.index(), *id)
    }

    /// Get a [`Node`] from a root tree and a path
    ///
    /// This traverses into the path to get the node.
    ///
    /// # Arguments
    ///
    /// * `root_tree` - The `Id` of the root tree
    // TODO!: This ID should be a tree ID, we should refactor it to wrap it in a TreeId type
    /// * `path` - The path
    ///
    /// # Errors
    ///
    /// * [`TreeErrorKind::NotADirectory`] - If the path is not a directory.
    /// * [`TreeErrorKind::PathNotFound`] - If the path is not found.
    /// * [`TreeErrorKind::PathIsNotUtf8Conform`] - If the path is not UTF-8 conform.
    ///
    /// [`TreeErrorKind::NotADirectory`]: crate::error::TreeErrorKind::NotADirectory
    /// [`TreeErrorKind::PathNotFound`]: crate::error::TreeErrorKind::PathNotFound
    /// [`TreeErrorKind::PathIsNotUtf8Conform`]: crate::error::TreeErrorKind::PathIsNotUtf8Conform
    pub fn node_from_path(&self, root_tree: Id, path: &Path) -> RusticResult<Node> {
        Tree::node_from_path(self.dbe(), self.index(), root_tree, Path::new(path))
    }

    /// Get all [`Node`]s from given root trees and a path
    ///
    /// # Arguments
    ///
    /// * `ids` - The tree ids to search in
    /// * `path` - The path
    ///
    /// # Errors
    /// if loading trees from the backend fails
    pub fn find_nodes_from_path(
        &self,
        ids: impl IntoIterator<Item = Id>,
        path: &Path,
    ) -> RusticResult<FindNode> {
        Tree::find_nodes_from_path(self.dbe(), self.index(), ids, path)
    }

    /// Get all [`Node`]s/[`Path`]s from given root trees and a matching criterion
    ///
    /// # Arguments
    ///
    /// * `ids` - The tree ids to search in
    /// * `matches` - The matching criterion
    ///
    /// # Errors
    /// if loading trees from the backend fails
    pub fn find_matching_nodes(
        &self,
        ids: impl IntoIterator<Item = Id>,
        matches: &impl Fn(&Path, &Node) -> bool,
    ) -> RusticResult<FindMatches> {
        Tree::find_matching_nodes(self.dbe(), self.index(), ids, matches)
    }
}

impl<P: ProgressBars, S: IndexedTree> Repository<P, S> {
    /// Get a [`Node`] from a "SNAP\[:PATH\]" syntax
    ///
    /// This parses for a snapshot (using the filter when "latest" is used) and then traverses into the path to get the node.
    ///
    /// # Arguments
    ///
    /// * `snap_path` - The path to the snapshot
    /// * `filter` - The filter to use
    ///
    /// # Errors
    ///
    /// * [`IdErrorKind::HexError`] - If the string is not a valid hexadecimal string
    /// * [`BackendAccessErrorKind::NoSuitableIdFound`] - If no id could be found.
    /// * [`BackendAccessErrorKind::IdNotUnique`] - If the id is not unique.
    ///
    /// [`IdErrorKind::HexError`]: crate::error::IdErrorKind::HexError
    /// [`BackendAccessErrorKind::NoSuitableIdFound`]: crate::error::BackendAccessErrorKind::NoSuitableIdFound
    /// [`BackendAccessErrorKind::IdNotUnique`]: crate::error::BackendAccessErrorKind::IdNotUnique
    pub fn node_from_snapshot_path(
        &self,
        snap_path: &str,
        filter: impl FnMut(&SnapshotFile) -> bool + Send + Sync,
    ) -> RusticResult<Node> {
        let (id, path) = snap_path.split_once(':').unwrap_or((snap_path, ""));

        let p = &self.pb.progress_counter("getting snapshot...");
        let snap = SnapshotFile::from_str(self.dbe(), id, filter, p)?;

        Tree::node_from_path(self.dbe(), self.index(), snap.tree, Path::new(path))
    }

    /// Get a [`Node`] from a [`SnapshotFile`] and a `path`
    ///
    /// This traverses into the path to get the node.
    ///
    /// # Arguments
    ///
    /// * `snap` - The snapshot to use
    /// * `path` - The path to the node
    ///
    /// # Errors
    ///
    // TODO: Document errors
    pub fn node_from_snapshot_and_path(
        &self,
        snap: &SnapshotFile,
        path: &str,
    ) -> RusticResult<Node> {
        Tree::node_from_path(self.dbe(), self.index(), snap.tree, Path::new(path))
    }
    /// Reads a raw tree from a "SNAP\[:PATH\]" syntax
    ///
    /// This parses a snapshot (using the filter when "latest" is used) and then traverses into the path to get the tree.
    ///
    /// # Arguments
    ///
    /// * `snap` - The snapshot to use
    /// * `sn_filter` - The filter to use
    ///
    /// # Errors
    ///
    // TODO: Document errors
    pub fn cat_tree(
        &self,
        snap: &str,
        sn_filter: impl FnMut(&SnapshotFile) -> bool + Send + Sync,
    ) -> RusticResult<Bytes> {
        commands::cat::cat_tree(self, snap, sn_filter)
    }

    /// List the contents of a given [`Node`]
    ///
    /// # Arguments
    ///
    /// * `node` - The node to list
    /// * `ls_opts` - The options to use
    ///
    /// # Returns
    ///
    /// If `node` is a tree node, this will list the content of that tree.
    /// If `node` is a file node, this will only return one element.
    ///
    /// # Note
    ///
    /// The `PathBuf` returned will be relative to the given `node`.
    ///
    /// # Errors
    ///
    // TODO: Document errors
    pub fn ls(
        &self,
        node: &Node,
        ls_opts: &LsOptions,
    ) -> RusticResult<impl Iterator<Item = RusticResult<(PathBuf, Node)>> + Clone + '_> {
        NodeStreamer::new_with_glob(self.dbe().clone(), self.index(), node, ls_opts)
    }

    /// Restore a given [`RestorePlan`] to a local destination
    ///
    /// # Arguments
    ///
    /// * `restore_infos` - The restore plan to use
    /// * `opts` - The options to use
    /// * `node_streamer` - The node streamer to use
    /// * `dest` - The destination to use
    ///
    /// # Errors
    ///
    // TODO: Document errors
    pub fn restore(
        &self,
        restore_infos: RestorePlan,
        opts: &RestoreOptions,
        node_streamer: impl Iterator<Item = RusticResult<(PathBuf, Node)>>,
        dest: &LocalDestination,
    ) -> RusticResult<()> {
        opts.restore(restore_infos, self, node_streamer, dest)
    }

    /// Merge the given trees.
    ///
    /// This method creates needed tree blobs within the repository.
    /// Merge conflicts (identical filenames which do not match) will be resolved using the ordering given by `cmp`.
    ///
    /// # Arguments
    ///
    /// * `trees` - The trees to merge
    /// * `cmp` - The comparison function to use for merge conflicts
    /// * `summary` - The summary to use
    ///
    /// # Errors
    ///
    // TODO: Document errors
    ///
    /// # Returns
    ///
    /// This method returns the blob [`Id`] of the merged tree.
    pub fn merge_trees(
        &self,
        trees: &[Id],
        cmp: &impl Fn(&Node, &Node) -> Ordering,
        summary: &mut SnapshotSummary,
    ) -> RusticResult<Id> {
        commands::merge::merge_trees(self, trees, cmp, summary)
    }

    /// Merge the given snapshots.
    ///
    /// This method will create needed tree blobs within the repository.
    /// Merge conflicts (identical filenames which do not match) will be resolved using the ordering given by `cmp`.
    ///
    /// # Arguments
    ///
    /// * `snaps` - The snapshots to merge
    /// * `cmp` - The comparison function to use for merge conflicts
    /// * `snap` - The snapshot to save
    ///
    /// # Errors
    ///
    // TODO: Document errors
    ///
    /// # Returns
    ///
    /// This method returns the modified and already saved [`SnapshotFile`].
    pub fn merge_snapshots(
        &self,
        snaps: &[SnapshotFile],
        cmp: &impl Fn(&Node, &Node) -> Ordering,
        snap: SnapshotFile,
    ) -> RusticResult<SnapshotFile> {
        commands::merge::merge_snapshots(self, snaps, cmp, snap)
    }
}

impl<P: ProgressBars, S: IndexedIds> Repository<P, S> {
    /// Run a backup of `source` using the given options.
    ///
    /// You have to give a preflled [`SnapshotFile`] which is modified and saved.
    ///
    /// # Arguments
    ///
    /// * `opts` - The options to use
    /// * `source` - The source to backup
    /// * `snap` - The snapshot to modify and save
    ///
    /// # Errors
    ///
    // TODO: Document errors
    ///
    /// # Returns
    ///  
    /// The saved snapshot.
    pub fn backup(
        &self,
        opts: &BackupOptions,
        source: &PathList,
        snap: SnapshotFile,
    ) -> RusticResult<SnapshotFile> {
        commands::backup::backup(self, opts, source, snap)
    }
}

impl<P, S: IndexedFull> Repository<P, S> {
    /// Get a blob utilizing the internal blob cache
    ///
    /// # Arguments
    ///
    /// * `id` - The id of the blob
    /// * `tpe` - The type of the blob
    ///
    /// # Errors
    ///
    /// * [`IndexErrorKind::BlobInIndexNotFound`] - If the blob is not found in the index
    ///
    /// # Returns
    ///
    /// The cached blob in bytes.
    ///
    /// [`IndexErrorKind::BlobInIndexNotFound`]: crate::error::IndexErrorKind::BlobInIndexNotFound
    pub fn get_blob_cached(&self, id: &Id, tpe: BlobType) -> RusticResult<Bytes> {
        self.get_blob_or_insert_with(id, || self.index().blob_from_backend(self.dbe(), tpe, id))
    }
}

impl<P: ProgressBars, S: IndexedFull> Repository<P, S> {
    /// Read a raw blob
    ///
    /// # Arguments
    ///
    /// * `tpe` - The type of the blob
    /// * `id` - The id of the blob
    ///
    /// # Errors
    ///
    /// * [`IdErrorKind::HexError`] - If the string is not a valid hexadecimal string
    ///
    /// # Returns
    ///
    /// The raw blob in bytes.
    ///
    /// [`IdErrorKind::HexError`]: crate::error::IdErrorKind::HexError
    pub fn cat_blob(&self, tpe: BlobType, id: &str) -> RusticResult<Bytes> {
        commands::cat::cat_blob(self, tpe, id)
    }

    /// Dump a [`Node`] using the given writer.
    ///
    /// # Arguments
    ///
    /// * `node` - The node to dump
    /// * `w` - The writer to use
    ///  
    /// # Note
    ///
    /// Currently, only regular file nodes are supported.
    ///
    /// # Errors
    ///
    /// * [`CommandErrorKind::DumpNotSupported`] - If the node is not a file.
    ///
    /// [`CommandErrorKind::DumpNotSupported`]: crate::error::CommandErrorKind::DumpNotSupported
    pub fn dump(&self, node: &Node, w: &mut impl Write) -> RusticResult<()> {
        commands::dump::dump(self, node, w)
    }

    /// Prepare the restore.
    ///
    /// If `dry_run` is set to false, it will also:
    /// - remove existing files from the destination, if `opts.delete` is set to true
    /// - create all dirs for the restore
    ///
    /// # Arguments
    ///
    /// * `opts` - The options to use
    /// * `node_streamer` - The node streamer to use
    /// * `dest` - The destination to use
    /// * `dry_run` - If true, only print what would be done
    ///
    /// # Errors
    ///
    /// * [`CommandErrorKind::ErrorCreating`] - If a directory could not be created.
    /// * [`CommandErrorKind::ErrorCollecting`] - If the restore information could not be collected.
    ///
    /// # Returns
    ///
    /// The restore plan.
    ///
    /// [`CommandErrorKind::ErrorCreating`]: crate::error::CommandErrorKind::ErrorCreating
    /// [`CommandErrorKind::ErrorCollecting`]: crate::error::CommandErrorKind::ErrorCollecting
    pub fn prepare_restore(
        &self,
        opts: &RestoreOptions,
        node_streamer: impl Iterator<Item = RusticResult<(PathBuf, Node)>>,
        dest: &LocalDestination,
        dry_run: bool,
    ) -> RusticResult<RestorePlan> {
        opts.collect_and_prepare(self, node_streamer, dest, dry_run)
    }

    /// Copy the given `snapshots` to `repo_dest`.
    ///
    /// # Type Parameters
    ///
    /// * `Q` - The type of the progress bar
    /// * `R` - The type of the index.
    ///
    /// # Arguments
    ///
    /// * `repo_dest` - The destination repository
    /// * `snapshots` - The snapshots to copy
    ///
    /// # Errors
    ///
    // TODO: Document errors
    ///
    /// # Note
    ///
    /// This command copies snapshots even if they already exist. For already existing snapshots, a
    /// copy will be created in the destination repository.
    ///
    /// To omit already existing snapshots, use `relevant_copy_snapshots` and filter out the non-relevant ones.
    pub fn copy<'a, Q: ProgressBars, R: IndexedIds>(
        &self,
        repo_dest: &Repository<Q, R>,
        snapshots: impl IntoIterator<Item = &'a SnapshotFile>,
    ) -> RusticResult<()> {
        commands::copy::copy(self, repo_dest, snapshots)
    }

    /// Repair snapshots.
    ///
    /// This traverses all trees of all snapshots and repairs defect trees.
    ///
    /// # Arguments
    ///
    /// * `opts` - The options to use
    /// * `snapshots` - The snapshots to repair
    /// * `dry_run` - If true, only print what would be done
    ///  
    /// # Warning
    ///
    /// If you remove the original snapshots, you may loose data!
    ///
    /// # Errors
    ///
    // TODO: Document errors
    pub fn repair_snapshots(
        &self,
        opts: &RepairSnapshotsOptions,
        snapshots: Vec<SnapshotFile>,
        dry_run: bool,
    ) -> RusticResult<()> {
        opts.repair(self, snapshots, dry_run)
    }
}

use std::sync::Arc;

use log::info;

/// A progress used to indicate/update the status of something which is being processed
#[derive(Debug)]
pub struct Progress(Arc<dyn RusticProgress>);

impl Progress {
    /// Create a new `Progress` from a suitable trait implementation
    pub fn new<P: RusticProgress>(p: P) -> Self {
        Self(Arc::new(p))
    }

    /// Create a new hidden `Progress`
    #[must_use]
    pub fn hidden() -> Self {
        Self(Arc::new(HiddenProgress))
    }

    /// Check if progress is hidden
    #[must_use]
    pub fn is_hidden(&self) -> bool {
        self.0.is_hidden()
    }

    /// Set total length for this progress
    ///
    /// # Arguments
    ///
    /// * `len` - The total length of this progress
    pub fn set_length(&self, len: u64) {
        self.0.set_length(len);
    }

    /// Set title for this progress
    ///
    /// # Arguments
    ///
    /// * `title` - The title of this progress
    pub fn set_title(&self, title: &str) {
        self.0.set_title(title);
    }

    /// Advance progress by given increment
    ///
    /// # Arguments
    ///
    /// * `inc` - The increment to advance this progress
    pub fn inc(&self, inc: u64) {
        self.0.inc(inc);
    }

    /// Finish the progress
    pub fn finish(&self) {
        self.0.finish();
    }
}

/// Trait to report progress information for any rustic action which supports that.
///
/// Implement this trait when you want to display this progress to your users.
pub trait RusticProgress: Send + Sync + 'static + std::fmt::Debug {
    /// Check if progress is hidden
    fn is_hidden(&self) -> bool;

    /// Set total length for this progress
    ///
    /// # Arguments
    ///
    /// * `len` - The total length of this progress
    fn set_length(&self, len: u64);

    /// Set title for this progress
    ///
    /// # Arguments
    ///
    /// * `title` - The title of this progress
    fn set_title(&self, title: &str);

    /// Advance progress by given increment
    ///
    /// # Arguments
    ///
    /// * `inc` - The increment to advance this progress
    fn inc(&self, inc: u64);

    /// Finish the progress
    fn finish(&self);
}

/// Type of progress
#[derive(Debug, Clone, Copy)]
pub enum ProgressType {
    /// a progress spinner. Note that this progress doesn't get a length and is not advanced, only finished.
    Spinner,
    /// a progress which counts something
    Counter,
    /// a progress which counts bytes
    Bytes,
}

/// Trait to start progress information report progress information for any rustic action which supports that.
///
/// Implement this trait when you want to display this progress to your users.
pub trait ProgressBars: std::fmt::Debug + Send + Sync + 'static {
    /// Start a new progress.
    ///
    /// # Arguments
    ///
    /// * `progress_type` - The type of the progress
    /// * `prefix` - The prefix of the progress
    fn progress(&self, progress_type: ProgressType, prefix: &str) -> Progress;
}

/// A Progress showing nothing at all
#[derive(Clone, Copy, Debug)]
pub struct HiddenProgress;
impl RusticProgress for HiddenProgress {
    fn is_hidden(&self) -> bool {
        true
    }
    fn set_length(&self, _len: u64) {}
    fn set_title(&self, _title: &str) {}
    fn inc(&self, _inc: u64) {}
    fn finish(&self) {}
}

/// A dummy struct which shows no progress but only logs titles and end of a progress.
#[derive(Clone, Copy, Debug)]
pub struct NoProgress;

impl RusticProgress for NoProgress {
    fn is_hidden(&self) -> bool {
        true
    }
    fn set_length(&self, _len: u64) {}
    fn set_title(&self, title: &str) {
        info!("{title}");
    }
    fn inc(&self, _inc: u64) {}
    fn finish(&self) {
        info!("finished.");
    }
}

/// Don't show progress bars, only log rudimentary progress information.
#[derive(Clone, Copy, Debug)]
pub struct NoProgressBars;

impl ProgressBars for NoProgressBars {
    fn progress(&self, _progress_type: ProgressType, prefix: &str) -> Progress {
        info!("{prefix}");
        Progress::new(NoProgress)
    }
}

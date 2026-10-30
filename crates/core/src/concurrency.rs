//! Concurrency management
//!
//! This module supports concurrency management for Rustic, controlling the
//! number of simultaneously active threads within each of several concurrency
//! classes, as defined by [`ConcurrencyClass`].  Each concurrency class uses a
//! [`ConcurrencyLimiter`] as a gatekeeper for all code wishing to execute
//! within the class.
//!
//! A [`ConcurrencyManager`] holds a collection of class limiters and allows
//! threads to manage their execution as part of a concurrency class. A manager
//! is created for each [`Repository`](../repository/struct.Repository.html) and
//! is accessed through the
//! [`ConcurrentBackend`](../backend/concurrent/trait.ConcurrentBackend.html)
//! trait on the backend associated with an open repository.
//!
//! Configuration is through [`ConcurrencyOptions`], typically as provided
//! within the [`RepositoryOptions`](crate::repository::RepositoryOptions).
//!
//! Assistance for integrating existing code is provided by [`ConcurrencyPool`]
//! for Rayon pools, and the [`ConcurrentIteratorExt`] trait for guarded or
//! parallel map execution within an interator.

use std::collections::HashMap;
use std::iter::Map;
use std::num::NonZero;
use std::str::FromStr;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{Scope, available_parallelism};
use std::time::Duration;

use derive_setters::Setters;
use log::{trace, warn};
use pariter::{ParallelMap, ParallelMapBuilder};
use rayon::{Scope as RayonScope, ThreadPool, ThreadPoolBuilder};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A single concurrency limit (number of threads)
pub type ConcurrencyLimit = NonZero<usize>;

/// An individual concurrency option
///
/// Used to configure an individual concurrency option, either as no limit, a
/// fixed number of threads or a percentage of available CPUs.  No limit options
/// disable any concurrency control for their concurrency class.
///
/// For command line, environment variable or configuration file use, values are
/// unsigned integers with percentages ending with a percent sign ("%").  The
/// value of "disabled" or 0 is used to disable a limit.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ConcurrencyOption {
    /// Limit disabled
    #[default]
    Disabled,
    /// Specific number of threads
    Threads(usize),
    /// Threads as percent of available cores (rounded up to nearest whole number)
    Percent(usize),
}

impl FromStr for ConcurrencyOption {
    type Err = std::num::ParseIntError;

    /// Parse string into a [`ConcurrencyOption`]
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let result = if value == "disabled" {
            ConcurrencyOption::Disabled
        } else {
            let mut percent = false;
            let mut value = value;
            if value.ends_with("%") {
                value = value.get(..value.len() - 1).expect("concurrency value");
                percent = true;
            }
            match value.parse::<usize>()? {
                0 => ConcurrencyOption::Disabled,
                x if percent => ConcurrencyOption::Percent(x),
                x => ConcurrencyOption::Threads(x),
            }
        };
        Ok(result)
    }
}

impl ConcurrencyOption {
    /// Serde serializer for a [`ConcurrencyOption`]
    fn serialize<S>(value: &Option<ConcurrencyOption>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if let Some(value) = value {
            match value {
                ConcurrencyOption::Disabled => serializer.serialize_str("disabled"),
                ConcurrencyOption::Threads(v) => serializer.serialize_u64(*v as u64),
                ConcurrencyOption::Percent(p) => {
                    let output = format!("{}%", p);
                    serializer.serialize_str(&output)
                }
            }
        } else {
            serializer.serialize_none()
        }
    }

    /// Serde deserializer for a [`ConcurrencyOption`]
    fn deserialize<'de, D>(deserializer: D) -> Result<Option<ConcurrencyOption>, D::Error>
    where
        D: Deserializer<'de>,
    {
        // Allow either string or numeric formats (percentages must be strings)
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Option {
            Number(usize),
            String(String),
        }

        let result = match Option::deserialize(deserializer)? {
            Option::Number(v) if v == 0 => ConcurrencyOption::Disabled,
            Option::Number(v) => ConcurrencyOption::Threads(v),
            Option::String(v) => v.parse().map_err(serde::de::Error::custom)?,
        };
        Ok(Some(result))
    }
}

/// Concurrency options
///
/// These options configure concurrency limits for a [`ConcurrencyManager`].
#[cfg_attr(feature = "clap", derive(clap::Parser))]
#[cfg_attr(feature = "merge", derive(conflate::Merge))]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Deserialize, Serialize, Setters)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
#[setters(into, strip_option)]
#[non_exhaustive]
pub struct ConcurrencyOptions {
    /// Threads available for default limits by other settings.  Number,
    /// percentage (#%) of available CPUs (rounded up to nearest whole number),
    /// or disabled/0 for disabled.  (default: 100%)
    #[cfg_attr(
        feature = "clap",
        clap(
            long = "concurrency-available",
            global = true,
            env = "RUSTIC_CONCURRENCY_AVAILABLE",
        )
    )]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    #[serde(deserialize_with = "ConcurrencyOption::deserialize")]
    #[serde(serialize_with = "ConcurrencyOption::serialize")]
    pub available: Option<ConcurrencyOption>,

    /// Threads (#, #% or disabled/0) for CPU-bound operations. This is not a
    /// hard limit on all CPU usage, but covers the most CPU-intensive
    /// operations.  (default: concurrency-available+1)
    #[cfg_attr(
        feature = "clap",
        clap(
            long = "concurrency-cpu",
            global = true,
            env = "RUSTIC_CONCURRENCY_CPU",
        )
    )]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    #[serde(deserialize_with = "ConcurrencyOption::deserialize")]
    #[serde(serialize_with = "ConcurrencyOption::serialize")]
    pub cpu: Option<ConcurrencyOption>,

    /// Threads (#, #% or disabled/0) for retrieving data from the backend,
    /// including cache operations.  (default: concurrency-available*2)
    #[cfg_attr(
        feature = "clap",
        clap(
            long = "concurrency-read",
            global = true,
            env = "RUSTIC_CONCURRENCY_READ",
        )
    )]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    #[serde(deserialize_with = "ConcurrencyOption::deserialize")]
    #[serde(serialize_with = "ConcurrencyOption::serialize")]
    pub read: Option<ConcurrencyOption>,

    /// Threads (#, #% or disabled/0) for writing data to the backend, including
    /// cache operations.  (default: 1)
    ///
    /// Note: Higher values weaken deduplication, trading off multiple parallel
    /// requests against larger snapshots.
    #[cfg_attr(
        feature = "clap",
        clap(
            long = "concurrency-write",
            global = true,
            env = "RUSTIC_CONCURRENCY_WRITE",
        )
    )]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    #[serde(deserialize_with = "ConcurrencyOption::deserialize")]
    #[serde(serialize_with = "ConcurrencyOption::serialize")]
    pub write: Option<ConcurrencyOption>,

    /// Limit (#, #% or disabled/0) for simultaneous backend requests (reads and
    /// writes combined).  (default: disabled)
    #[cfg_attr(
        feature = "clap",
        clap(
            long = "concurrency-backend",
            global = true,
            env = "RUSTIC_CONCURRENCY_BACKEND",
        )
    )]
    #[cfg_attr(feature = "merge", merge(strategy = conflate::option::overwrite_none))]
    #[serde(deserialize_with = "ConcurrencyOption::deserialize")]
    #[serde(serialize_with = "ConcurrencyOption::serialize")]
    pub backend: Option<ConcurrencyOption>,
}

/// Categories for identifying thread classes for concurrency control.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ConcurrencyClass {
    /// Simultaneous requests (read and write combined) to a backend.  This
    /// provides an overall rate limit to the backend, subject to the other
    /// Read or Write class limits.
    Backend,

    /// Threads for CPU-intensive operations.  This is not a hard limit on
    /// all CPU usage but covers the most CPU intensive operations such as
    /// chunking, hashing, encryption and compression.
    Cpu,

    /// Threads for data retrieval from a backend (including from the cache
    /// for the backend).
    Read,

    /// Threads responsible for writing data to a backend (including updates
    /// to the cache for the backend).
    Write,
}

/// Time to wait before warning about possible deadlock (debug mode only)
#[cfg(debug_assertions)]
const DEADLOCK_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(not(debug_assertions))]
const DEADLOCK_TIMEOUT: Duration = Duration::MAX;

/// Concurrency limiter
///
/// Allows threads to acquire [`ConcurrencyLimitGuard`] objects, subject to a
/// maximum limit on how many threads may hold such objects simultaneously.
/// Any additional threads block until an existing guard object drops.
///
/// Reentrancy is supported (requests from threads already holding a guard for
/// the class being requested will return immediately), but deadlock remains a
/// risk.  Threads should avoid holding a guard while blocked waiting on other
/// threads that may execute in the same concurrency class.
///
/// In development mode, a warning will be logged after each [DEADLOCK_TIMEOUT]
/// interval while a thread remains blocked waiting to acquire a guard.
#[derive(Debug, Clone)]
pub struct ConcurrencyLimiter {
    kind: ConcurrencyClass,
    limit: NonZero<usize>,
    lock: Arc<Mutex<Vec<std::thread::ThreadId>>>,
    cond: Arc<Condvar>,
}

impl ConcurrencyLimiter {
    /// Create a new [`ConcurrencyLimiter`]
    pub fn new(kind: ConcurrencyClass, limit: NonZero<usize>) -> Self {
        Self {
            kind,
            limit,
            lock: Arc::new(Mutex::new(Vec::with_capacity(limit.into()))),
            cond: Arc::new(Condvar::new()),
        }
    }

    /// Acquire a [`ConcurrencyLimitGuard`] for the current thread, as long as
    /// the total number of threads with guards are below the limit.
    ///
    /// The function will block until the thread is permitted to execute
    /// within the concurrency class represented by this limiter.
    ///
    /// Supports reentrancy.  If the requesting thread has previously obtained
    /// a guard object for the concurrency class, None is returned.
    pub fn acquire(&self) -> Option<ConcurrencyLimitGuard<'_>> {
        let tid = std::thread::current().id();
        let limit = self.limit.get();

        let mut active = loop {
            let (active, timeout) = self
                .cond
                .wait_timeout_while(self.lock.lock().unwrap(), DEADLOCK_TIMEOUT, |active| {
                    let wait = active.len() >= limit && !active.contains(&tid);

                    #[cfg(debug_assertions)]
                    if wait {
                        trace!(
                            "{:?} {:?} limiter: wait ({} active, limit {})",
                            tid,
                            self.kind,
                            active.len(),
                            limit
                        );
                    }

                    wait
                })
                .unwrap();

            if !timeout.timed_out() {
                break active;
            }

            warn!(
                "{:?} {:?} limiter: possible deadlock, still waiting.",
                tid, self.kind
            );
        };

        let guard = if active.contains(&tid) {
            None
        } else {
            active.push(tid);
            Some(ConcurrencyLimitGuard::new(&self))
        };

        #[cfg(debug_assertions)]
        trace!(
            "{:?} {:?} limiter: {} ({} threads) limit {}",
            tid,
            self.kind,
            if guard.is_some() {
                "acquire"
            } else {
                "reentrant"
            },
            active.len(),
            limit
        );

        guard
    }
}

/// Guard object issued by a [`ConcurrencyLimiter`]
///
/// The thread holding the guard object will continue to count against the
/// limiter limit until the guard object drops.
#[derive(Debug)]
pub struct ConcurrencyLimitGuard<'a>(&'a ConcurrencyLimiter);

impl<'a> ConcurrencyLimitGuard<'a> {
    fn new(limiter: &'a ConcurrencyLimiter) -> Self {
        Self(limiter)
    }
}

impl<'a> Drop for ConcurrencyLimitGuard<'a> {
    fn drop(&mut self) {
        let tid = std::thread::current().id();
        let mut active = self.0.lock.lock().unwrap();
        let idx = active.iter().position(|t| *t == tid);
        match idx {
            None => warn!(
                "{:?} {:?} limiter: guard drop by unknown thread, ignoring.",
                tid, self.0.kind
            ),
            Some(idx) => {
                let _ = active.swap_remove(idx);

                #[cfg(debug_assertions)]
                trace!(
                    "{:?} {:?} limiter: release ({} active)",
                    tid,
                    self.0.kind,
                    active.len()
                );

                drop(active);
                self.0.cond.notify_one();
            }
        }
    }
}

/// Concurrency manager
///
/// Manages [`ConcurrencyLimiter`]s for each [`ConcurrencyClass`].
///
/// Threads wishing to participate in the configured limit for a concurrency
/// class should acquire a [`ConcurrencyLimitGuard`] for that class by calling
/// acquire() and hold it until finished executing.  If too many threads are
/// executing in that class, the calling thread will block until an opening is
/// available.
///
/// Limit functions are available for use when allocating resources (such as
/// thread pools) that use the class limits to calculate their size.  The
/// `available()` function will return the system available parallelism.
///
/// The `pool()` utility function provides for creating a [`ConcurrencyPool`]
/// of the desired size.
#[derive(Clone, Debug)]
pub struct ConcurrencyManager {
    available: ConcurrencyLimit,
    limiters: HashMap<ConcurrencyClass, ConcurrencyLimiter>,
}

impl ConcurrencyManager {
    /// Create a new [`ConcurrencyManager`] using the [`ConcurrencyOptions`].
    ///
    /// Specified options are used directly; those with a percentage of CPUs use
    /// std::thread::available_parallelism, rounded up to the nearest thread.
    ///
    /// Defaults for unspecified options are based on the "available" option
    /// (default 100%), are unlimited if available is disabled, or else:
    ///
    ///   * Cpu = available + 1
    ///   * Read = 2 * available
    ///   * Write = 1
    ///   * Backend = unlimited
    pub fn new(opts: ConcurrencyOptions) -> Self {
        let avail = match available_parallelism() {
            Ok(num) => num.get(),
            Err(_) => {
                warn!("Unable to identify system concurrency value, using 1");
                1
            }
        };

        let threads = |v: ConcurrencyOption| match v {
            ConcurrencyOption::Disabled => 0,
            ConcurrencyOption::Threads(v) => v,
            ConcurrencyOption::Percent(v) => ((v as f32 / 100.0) * avail as f32).ceil() as usize,
        };

        let base = opts.available.map(threads).unwrap_or(avail);
        let limits = [
            (
                ConcurrencyClass::Backend,
                NonZero::new(opts.backend.map(threads).unwrap_or(0)),
            ),
            (
                ConcurrencyClass::Cpu,
                NonZero::new(
                    opts.cpu
                        .map(threads)
                        .unwrap_or(if base == 0 { 0 } else { base + 1 }),
                ),
            ),
            (
                ConcurrencyClass::Read,
                NonZero::new(opts.read.map(threads).unwrap_or(base * 2)),
            ),
            (
                ConcurrencyClass::Write,
                NonZero::new(
                    opts.write
                        .map(threads)
                        .unwrap_or(if base == 0 { 0 } else { 1 }),
                ),
            ),
        ];

        // Create limiters for any classes with an actual limit
        Self {
            available: NonZero::new(avail).unwrap(),
            limiters: limits
                .into_iter()
                .filter(|(_, l)| l.is_some())
                .map(|(c, l)| (c, ConcurrencyLimiter::new(c, l.unwrap())))
                .collect(),
        }
    }

    //
    // Concurrency Guards
    //

    /// Acquire a guard object for a concurrency class for the current thread,
    /// blocking until successful.  If the class is unlimited, or the calling
    /// thread already has a guard for the class, None is returned.
    pub fn acquire(&self, kind: ConcurrencyClass) -> Option<ConcurrencyLimitGuard<'_>> {
        self.limiters.get(&kind).and_then(|l| l.acquire())
    }

    //
    // Limit Management
    //

    /// Return the available parallelism when the manager was created.
    pub fn available(&self) -> ConcurrencyLimit {
        self.available
    }

    /// Return the limit, if configured, for a concurrency class.
    pub fn limit(&self, kind: ConcurrencyClass) -> Option<ConcurrencyLimit> {
        self.limiters.get(&kind).and_then(|l| Some(l.limit))
    }

    /// Return the limit, if configured, for a concurrency class, with a
    /// default value if no limit was defined.
    pub fn limit_or(&self, kind: ConcurrencyClass, or: ConcurrencyLimit) -> ConcurrencyLimit {
        self.limit(kind).unwrap_or(or.into())
    }

    /// Return the limit, if configured, for a concurrency class or the
    /// system available parallelism if no limit was defined.
    pub fn limit_or_available(&self, kind: ConcurrencyClass) -> ConcurrencyLimit {
        self.limit_or(kind, self.available)
    }

    /// Return all limits configured for this manager.
    pub fn limits(&self) -> HashMap<ConcurrencyClass, ConcurrencyLimit> {
        self.limiters.iter().map(|(c, l)| (*c, l.limit)).collect()
    }

    /// Return a ConcurrencyPool with the given thread limit (None for the global pool).
    pub fn pool<S>(&self, name: S, limit: Option<ConcurrencyLimit>) -> ConcurrencyPool
    where
        S: Into<String>,
    {
        ConcurrencyPool::new(name, limit)
    }
}

impl Default for ConcurrencyManager {
    /// Create a new [`ConcurrencyManager`] with default [`ConcurrencyOptions`].
    fn default() -> Self {
        ConcurrencyManager::new(ConcurrencyOptions::default().into())
    }
}

/// Concurrency Rayon pool
///
/// This is a wrapper around a local Rayon pool
/// [`ThreadPool`](https://docs.rs/rayon/latest/rayon/struct.ThreadPool.html)
/// with `install()`, `scope()` and `in_place_scope()` functions mirroring those
/// of `ThreadPool`.
///
/// If there is no pool size supplied when a [`ConcurrencyPool`] is created (or
/// there is a failure creating the pool), operations will use the global pool.
#[derive(Debug)]
pub struct ConcurrencyPool {
    name: String,
    pool: Option<ThreadPool>,
}

impl ConcurrencyPool {
    /// Create a new [`ConcurrencyPool`]
    pub fn new<S>(name: S, size: Option<ConcurrencyLimit>) -> Self
    where
        S: Into<String>,
    {
        let name = name.into();
        let pool = match size {
            None => None,
            Some(threads) => ThreadPoolBuilder::new()
                .num_threads(threads.into())
                .build()
                .map_err(|_err| {
                    warn!(
                        "Unable to create Rayon thread pool with {} threads, using global pool",
                        threads
                    );
                })
                .ok(),
        };

        trace!(
            "{:?} ConcurrencyPool({}) created with {}",
            std::thread::current().id(),
            name,
            match &pool {
                None => String::from("global pool"),
                Some(pool) => format!("{} threads", pool.current_num_threads()),
            }
        );

        Self { name, pool }
    }

    /// Execute a function in the pool
    pub fn install<F, R>(&self, func: F) -> R
    where
        F: FnOnce() -> R + Send,
        R: Send,
    {
        match &self.pool {
            Some(pool) => pool.install(|| func()),
            None => func(),
        }
    }

    /// Execute a function within a scope in the pool
    pub fn scope<'scope, F, R>(&self, func: F) -> R
    where
        F: FnOnce(&RayonScope<'scope>) -> R + Send,
        R: Send,
    {
        match &self.pool {
            Some(pool) => pool.scope(|s| func(s)),
            None => rayon::scope(|s| func(s)),
        }
    }

    /// Execute a function within a scope locally (in the calling thread)
    /// with any Rayon tasks using the pool.  Note that parallel iterations
    /// will still use the caller's pool and not this pool.
    pub fn in_place_scope<'scope, F, R>(&self, func: F) -> R
    where
        F: FnOnce(&RayonScope<'scope>) -> R,
    {
        match &self.pool {
            Some(pool) => pool.in_place_scope(|s| func(s)),
            None => rayon::in_place_scope(|s| func(s)),
        }
    }
}

impl Drop for ConcurrencyPool {
    fn drop(&mut self) {
        trace!(
            "{:?} ConcurrencyPool({}) dropped",
            std::thread::current().id(),
            self.name
        );
    }
}

/// Iterator that guards item retrieval as part of a concurrency class.
#[derive(Debug)]
pub struct ConcurrentIterator<'a, I: Iterator> {
    iter: I,
    limiter: Option<&'a ConcurrencyLimiter>,
}

/// Iterator using a
/// [`ParallelMap`](https://docs.rs/pariter/latest/pariter/struct.ParallelMap.html)
/// for multiple threads, or a standard [`Map`] for a single thread.
pub enum ConcurrentMapIterator<I: Iterator, F, O>
where
    F: FnMut(I::Item) -> O + Send + Clone,
    O: Send,
    I::Item: Send,
{
    Parallel(usize, ParallelMap<I, O>),
    Single(Map<I, F>),
}

impl<I: Iterator, F, O> std::fmt::Debug for ConcurrentMapIterator<I, F, O>
where
    F: FnMut(I::Item) -> O + Send + Clone,
    O: Send,
    I::Item: Send,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConcurrentMapIterator::Parallel(t, _) => write!(f, "Parallel({})", t),
            ConcurrentMapIterator::Single(_) => write!(f, "Single"),
        }
    }
}

/// Trait adding some concurrency functions to [`Iterator`], allowing iteration
/// while holding a concurrency class guard, or creation of parallel map
/// iterators that fall back to a single threaded map when appropriate.
pub trait ConcurrentIteratorExt {
    /// Return a [`ConcurrentIterator`] that will hold a guard for the specified
    /// concurrency class around each request for an item during iteration.
    fn concurrent_iter(
        self,
        manager: &ConcurrencyManager,
        kind: ConcurrencyClass,
    ) -> ConcurrentIterator<'_, Self>
    where
        Self: Sized + Iterator,
    {
        ConcurrentIterator {
            iter: self,
            limiter: manager.limiters.get(&kind),
        }
    }

    /// Create a parallel iterator for concurrent use.  Defaults to available
    /// parallelism if no limit is specified.  A limit of 1 will use a regular
    /// [`Map`] iterator in the calling thread, while multiple threads use
    /// [`ParallelMap`](https://docs.rs/pariter/latest/pariter/struct.ParallelMap.html).
    #[allow(dead_code)]
    fn concurrent_map<F, O>(
        self,
        limit: Option<ConcurrencyLimit>,
        f: F,
    ) -> ConcurrentMapIterator<Self, F, O>
    where
        Self: Sized + Iterator,
        Self::Item: Send + 'static,
        F: FnMut(Self::Item) -> O,
        F: Send + Clone + 'static,
        O: Send + 'static,
    {
        let threads = limit
            .map(|l| l.get())
            .unwrap_or(available_parallelism().map(|l| l.get()).unwrap_or(1));

        let iter = if threads == 1 {
            ConcurrentMapIterator::Single(self.map(f))
        } else {
            ConcurrentMapIterator::Parallel(
                threads,
                ParallelMapBuilder::new(self).threads(threads).with(f),
            )
        };

        trace!("New concurrent iterator: {:?}", iter);
        iter
    }

    /// Create a scoped parallel iterator for concurrent use.  Defaults to
    /// available parallelism if no limit is specified.  A limit of 1 will use
    /// a regular [`Map`] iterator in the calling thread, while multiple threads use
    /// [`ParallelMap`](https://docs.rs/pariter/latest/pariter/struct.ParallelMap.html).
    fn concurrent_map_scoped<'env, 'scope, F, O>(
        self,
        limit: Option<ConcurrencyLimit>,
        scope: &'scope Scope<'scope, 'env>,
        f: F,
    ) -> ConcurrentMapIterator<Self, F, O>
    where
        Self: Sized + Iterator,
        Self::Item: Send + 'env,
        F: FnMut(Self::Item) -> O,
        F: Send + Clone + 'env,
        O: Send + 'env,
    {
        let threads = limit
            .map(|l| l.get())
            .unwrap_or(available_parallelism().map(|l| l.get()).unwrap_or(1));

        let iter = if threads == 1 {
            ConcurrentMapIterator::Single(self.map(f))
        } else {
            ConcurrentMapIterator::Parallel(
                threads,
                ParallelMapBuilder::new(self)
                    .threads(threads)
                    .with_scoped(scope, f),
            )
        };

        trace!("New concurrent iterator: {:?}", iter);
        iter
    }
}

impl<I> ConcurrentIteratorExt for I where I: Iterator {}

impl<I: Iterator> Iterator for ConcurrentIterator<'_, I> {
    type Item = I::Item;

    fn next(&mut self) -> Option<Self::Item> {
        let _guard = self.limiter.map(|l| l.acquire());
        self.iter.next()
    }
}

impl<I: Iterator, F, O> Iterator for ConcurrentMapIterator<I, F, O>
where
    F: FnMut(I::Item) -> O + Send + Clone,
    O: Send,
    I::Item: Send,
{
    type Item = O;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            ConcurrentMapIterator::Single(map) => map.next(),
            ConcurrentMapIterator::Parallel(_, par) => par.next(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::thread::{scope, sleep, spawn};

    use rand::prelude::*;

    use super::*;

    #[test]
    fn test_options_default() {
        let opts = ConcurrencyOptions::default();
        let mgr1 = ConcurrencyManager::default();
        let mgr2 = ConcurrencyManager::new(opts);

        assert_matches!(
            opts,
            ConcurrencyOptions {
                available: None,
                cpu: None,
                read: None,
                write: None,
                backend: None
            }
        );
        assert_eq!(mgr1.limits(), mgr2.limits());

        let a = mgr1.available().get();

        assert_eq!(
            mgr1.limit(ConcurrencyClass::Cpu),
            NonZero::new(a + 1),
            "{}",
            "Cpu"
        );
        assert_eq!(
            mgr1.limit(ConcurrencyClass::Read),
            NonZero::new(a * 2),
            "{}",
            "Read"
        );
        assert_eq!(
            mgr1.limit(ConcurrencyClass::Write),
            NonZero::new(1),
            "{}",
            "Write"
        );
        assert_eq!(mgr1.limit(ConcurrencyClass::Backend), None, "{}", "Backend");
    }

    #[test]
    fn test_options_nolimit() {
        // Globally disabling via available
        let mgr = ConcurrencyManager::new(ConcurrencyOptions {
            available: Some(ConcurrencyOption::Disabled),
            ..ConcurrencyOptions::default()
        });

        assert_eq!(mgr.limit(ConcurrencyClass::Cpu), None);
        assert_eq!(mgr.limit(ConcurrencyClass::Read), None);
        assert_eq!(mgr.limit(ConcurrencyClass::Write), None);
        assert_eq!(mgr.limit(ConcurrencyClass::Backend), None);

        // Individually disabling even if default would assign limits
        let mgr = ConcurrencyManager::new(ConcurrencyOptions {
            available: None,
            cpu: Some(ConcurrencyOption::Disabled),
            read: Some(ConcurrencyOption::Disabled),
            write: Some(ConcurrencyOption::Disabled),
            backend: Some(ConcurrencyOption::Disabled),
        });

        assert_eq!(mgr.limit(ConcurrencyClass::Cpu), None);
        assert_eq!(mgr.limit(ConcurrencyClass::Read), None);
        assert_eq!(mgr.limit(ConcurrencyClass::Write), None);
        assert_eq!(mgr.limit(ConcurrencyClass::Backend), None);
    }

    #[test]
    fn test_options_available_percent() {
        let available = ConcurrencyManager::default().available().get() as f32;
        let values = [25, 50, 75, 99, 100, 101, 150, 200];

        for v in values.into_iter() {
            let mut opts = ConcurrencyOptions::default();
            opts.available = Some(ConcurrencyOption::Percent(v));

            let mgr = ConcurrencyManager::new(opts);
            let expected = (available * (v as f32 / 100.0)).ceil() as usize;

            assert_eq!(mgr.limit(ConcurrencyClass::Cpu), NonZero::new(expected + 1));
            assert_eq!(
                mgr.limit(ConcurrencyClass::Read),
                NonZero::new(expected * 2)
            );
            assert_eq!(mgr.limit(ConcurrencyClass::Write), NonZero::new(1));
            assert_eq!(mgr.limit(ConcurrencyClass::Backend), None);
        }
    }

    #[test]
    fn test_options_available_threads() {
        let values = [1, 2, 4, 8, 16, 32, 64];

        for v in values.into_iter() {
            let mut opts = ConcurrencyOptions::default();
            opts.available = Some(ConcurrencyOption::Threads(v));

            let mgr = ConcurrencyManager::new(opts);

            assert_eq!(mgr.limit(ConcurrencyClass::Cpu), NonZero::new(v + 1));
            assert_eq!(mgr.limit(ConcurrencyClass::Read), NonZero::new(v * 2));
            assert_eq!(mgr.limit(ConcurrencyClass::Write), NonZero::new(1));
            assert_eq!(mgr.limit(ConcurrencyClass::Backend), None);
        }
    }

    #[test]
    fn test_options() {
        let mut rng = rand::rng();

        for _i in 0..5 {
            let cpu = rng.random_range(0..=64);
            let read = rng.random_range(0..=64);
            let write = rng.random_range(0..=64);
            let backend = rng.random_range(0..=64);

            let opts = ConcurrencyOptions {
                cpu: Some(ConcurrencyOption::Threads(cpu)),
                read: Some(ConcurrencyOption::Threads(read)),
                write: Some(ConcurrencyOption::Threads(write)),
                backend: Some(ConcurrencyOption::Threads(backend)),
                ..ConcurrencyOptions::default()
            };

            let mgr = ConcurrencyManager::new(opts);
            assert_eq!(mgr.limit(ConcurrencyClass::Cpu), NonZero::new(cpu));
            assert_eq!(mgr.limit(ConcurrencyClass::Read), NonZero::new(read));
            assert_eq!(mgr.limit(ConcurrencyClass::Write), NonZero::new(write));
            assert_eq!(mgr.limit(ConcurrencyClass::Backend), NonZero::new(backend));
        }
    }

    #[test]
    fn test_options_percent() {
        let opts = ConcurrencyOptions {
            cpu: Some(ConcurrencyOption::Percent(100)),
            read: Some(ConcurrencyOption::Percent(200)),
            write: Some(ConcurrencyOption::Percent(300)),
            backend: Some(ConcurrencyOption::Percent(400)),
            ..ConcurrencyOptions::default()
        };

        let mgr = ConcurrencyManager::new(opts);
        let available = mgr.available().get();

        assert_eq!(mgr.limit(ConcurrencyClass::Cpu), NonZero::new(available));
        assert_eq!(
            mgr.limit(ConcurrencyClass::Read),
            NonZero::new(available * 2)
        );
        assert_eq!(
            mgr.limit(ConcurrencyClass::Write),
            NonZero::new(available * 3)
        );
        assert_eq!(
            mgr.limit(ConcurrencyClass::Backend),
            NonZero::new(available * 4)
        );
    }

    #[test]
    fn test_options_fromstr() {
        assert!(
            ConcurrencyOption::from_str("").is_err(),
            "parsing empty concurrency option"
        );
        assert!(
            ConcurrencyOption::from_str("%").is_err(),
            "parsing empty percentage concurrency option"
        );
        assert!(
            ConcurrencyOption::from_str("-1").is_err(),
            "parsing negative concurrency option"
        );
        assert!(
            ConcurrencyOption::from_str("\"5\"").is_err(),
            "parsing quoted value (serde only)"
        );

        assert_eq!(
            ConcurrencyOption::from_str("disabled"),
            Ok(ConcurrencyOption::Disabled)
        );
        assert_eq!(
            ConcurrencyOption::from_str("0"),
            Ok(ConcurrencyOption::Disabled)
        );
        assert_eq!(
            ConcurrencyOption::from_str("5"),
            Ok(ConcurrencyOption::Threads(5))
        );
        assert_eq!(
            ConcurrencyOption::from_str("10%"),
            Ok(ConcurrencyOption::Percent(10))
        );
    }

    #[test]
    fn test_options_serde() {
        let tests = [
            // ( name, config, options, serialize )
            (
                "Defaults are all unspecified",
                String::new(),
                ConcurrencyOptions {
                    available: None,
                    cpu: None,
                    read: None,
                    write: None,
                    backend: None,
                },
                true,
            ),
            (
                "Disabled limits",
                [
                    "available = \"disabled\"",
                    "cpu = \"disabled\"",
                    "read = \"disabled\"",
                    "write = \"disabled\"",
                    "backend = \"disabled\"",
                    "",
                ]
                .join("\n"),
                ConcurrencyOptions {
                    available: Some(ConcurrencyOption::Disabled),
                    cpu: Some(ConcurrencyOption::Disabled),
                    read: Some(ConcurrencyOption::Disabled),
                    write: Some(ConcurrencyOption::Disabled),
                    backend: Some(ConcurrencyOption::Disabled),
                },
                true,
            ),
            (
                "Explicit 0 values are also disabled",
                [
                    "available = 0",
                    "cpu = 0",
                    "read = 0",
                    "write = 0",
                    "backend = 0",
                    "",
                ]
                .join("\n"),
                ConcurrencyOptions {
                    available: Some(ConcurrencyOption::Disabled),
                    cpu: Some(ConcurrencyOption::Disabled),
                    read: Some(ConcurrencyOption::Disabled),
                    write: Some(ConcurrencyOption::Disabled),
                    backend: Some(ConcurrencyOption::Disabled),
                },
                false, // Serializes as disabled
            ),
            (
                "Direct thread counts",
                [
                    "available = 1",
                    "cpu = 2",
                    "read = 3",
                    "write = 4",
                    "backend = 5",
                    "",
                ]
                .join("\n"),
                ConcurrencyOptions {
                    available: Some(ConcurrencyOption::Threads(1)),
                    cpu: Some(ConcurrencyOption::Threads(2)),
                    read: Some(ConcurrencyOption::Threads(3)),
                    write: Some(ConcurrencyOption::Threads(4)),
                    backend: Some(ConcurrencyOption::Threads(5)),
                },
                true,
            ),
            (
                "Direct thread counts (as strings)",
                [
                    "available = \"1\"",
                    "cpu = \"2\"",
                    "read = \"3\"",
                    "write = \"4\"",
                    "backend = \"5\"",
                    "",
                ]
                .join("\n"),
                ConcurrencyOptions {
                    available: Some(ConcurrencyOption::Threads(1)),
                    cpu: Some(ConcurrencyOption::Threads(2)),
                    read: Some(ConcurrencyOption::Threads(3)),
                    write: Some(ConcurrencyOption::Threads(4)),
                    backend: Some(ConcurrencyOption::Threads(5)),
                },
                false, // Serializes as numbers (no quotes)
            ),
            (
                "Percentages",
                [
                    "available = \"1%\"",
                    "cpu = \"2%\"",
                    "read = \"3%\"",
                    "write = \"4%\"",
                    "backend = \"5%\"",
                    "",
                ]
                .join("\n"),
                ConcurrencyOptions {
                    available: Some(ConcurrencyOption::Percent(1)),
                    cpu: Some(ConcurrencyOption::Percent(2)),
                    read: Some(ConcurrencyOption::Percent(3)),
                    write: Some(ConcurrencyOption::Percent(4)),
                    backend: Some(ConcurrencyOption::Percent(5)),
                },
                true,
            ),
        ];

        for (name, config, expected, test_serialize) in tests {
            let parse: Result<ConcurrencyOptions, _> = toml::from_str(&config);
            assert!(parse.is_ok(), "Deserialization failed: {:?}", parse);
            assert_eq!(parse.unwrap(), expected, "Deserialize: {}", name);

            if test_serialize {
                let output = toml::to_string(&expected);
                assert!(output.is_ok(), "Serialization failed: {:?}", output);
                assert_eq!(output.unwrap(), config, "Serialize: {}", name);
            }
        }
    }

    #[test]
    fn test_limits() {
        let cpu = 10;
        let read = 15;
        let write = 20;
        let backend = 25;

        let opts = ConcurrencyOptions {
            cpu: Some(ConcurrencyOption::Threads(cpu)),
            read: Some(ConcurrencyOption::Threads(read)),
            write: Some(ConcurrencyOption::Threads(write)),
            backend: Some(ConcurrencyOption::Threads(backend)),
            ..ConcurrencyOptions::default()
        };
        let limits: HashMap<ConcurrencyClass, ConcurrencyLimit> = [
            (ConcurrencyClass::Cpu, NonZero::new(cpu).unwrap()),
            (ConcurrencyClass::Read, NonZero::new(read).unwrap()),
            (ConcurrencyClass::Write, NonZero::new(write).unwrap()),
            (ConcurrencyClass::Backend, NonZero::new(backend).unwrap()),
        ]
        .into_iter()
        .collect();

        let mgr = ConcurrencyManager::new(opts);
        assert_eq!(mgr.limits(), limits);
    }

    #[test]
    fn test_limit_functions() {
        let limit = NonZero::new(12345).unwrap();
        let limit_or = NonZero::new(54321).unwrap();

        let mut opts = ConcurrencyOptions::default();
        opts.available = Some(ConcurrencyOption::Disabled);
        opts.cpu = Some(ConcurrencyOption::Threads(limit.get()));

        let mgr = ConcurrencyManager::new(opts);

        // Ensure assigned limit always respected
        assert_eq!(mgr.limit(ConcurrencyClass::Cpu), Some(limit));
        assert_eq!(mgr.limit_or(ConcurrencyClass::Cpu, limit_or), limit);
        assert_eq!(mgr.limit_or_available(ConcurrencyClass::Cpu), limit);

        // Check fallback when no limit assigned
        assert_eq!(mgr.limit(ConcurrencyClass::Read), None);
        assert_eq!(mgr.limit_or(ConcurrencyClass::Read, limit_or), limit_or);
        assert_eq!(
            mgr.limit_or_available(ConcurrencyClass::Read),
            mgr.available()
        );
    }

    #[test]
    fn test_acquire() {
        // TODO: Consider adding test entry points for acquire with timeout to
        // avoid risk of future failures blocking a test run

        let inner = |mgr: ConcurrencyManager, threads| {
            let start = Arc::new(AtomicBool::new(false));
            let active = Arc::new(AtomicUsize::new(0));
            let max = Arc::new(AtomicUsize::new(0));

            let handles: Vec<_> = (0..threads)
                .into_iter()
                .map(|_| {
                    let start = start.clone();
                    let active = active.clone();
                    let max = max.clone();
                    let mgr = mgr.clone();

                    // Each thread holds guard for about 10ms in total
                    spawn(move || {
                        while !start.load(Ordering::Relaxed) {
                            sleep(Duration::from_millis(1));
                        }
                        for _i in 0..1000 {
                            // Track peak active thread count
                            let _guard = mgr.acquire(ConcurrencyClass::Cpu);
                            let last_active = active.fetch_add(1, Ordering::Relaxed);
                            let _ = max.fetch_max(last_active + 1, Ordering::Relaxed);
                            sleep(Duration::from_micros(10));
                            let _ = active.fetch_sub(1, Ordering::Relaxed);
                        }
                    })
                })
                .collect();

            start.store(true, Ordering::Relaxed);
            for handle in handles {
                let _ = handle.join();
            }

            max.load(Ordering::Relaxed)
        };

        let opts = ConcurrencyOptions::default();

        let mgr = ConcurrencyManager::new(ConcurrencyOptions {
            cpu: Some(ConcurrencyOption::Threads(1)),
            ..opts
        });
        assert_eq!(inner(mgr, 5), 1);

        let mgr = ConcurrencyManager::new(ConcurrencyOptions {
            cpu: Some(ConcurrencyOption::Threads(5)),
            ..opts
        });
        assert_eq!(inner(mgr, 5), 5);

        let mgr = ConcurrencyManager::new(ConcurrencyOptions {
            cpu: Some(ConcurrencyOption::Threads(5)),
            ..opts
        });
        assert_eq!(inner(mgr, 100), 5);
    }

    #[test]
    fn test_acquire_reentrant() {
        let mgr = ConcurrencyManager::new(ConcurrencyOptions {
            cpu: Some(ConcurrencyOption::Threads(1)),
            ..ConcurrencyOptions::default()
        });

        // Acquire beneath limit
        let guard0 = mgr.acquire(ConcurrencyClass::Cpu);
        assert!(guard0.is_some());

        // Reentrant acquisition
        let guard1 = mgr.acquire(ConcurrencyClass::Cpu);
        assert!(guard1.is_none());

        // Acquire after release gives new guard
        drop(guard0);
        let guard2 = mgr.acquire(ConcurrencyClass::Cpu);
        assert!(guard2.is_some());
    }

    #[test]
    fn test_pool() {
        let mgr = ConcurrencyManager::default();
        for threads in [0, 1, 5, 10, 20] {
            let pool = mgr.pool("test", NonZero::new(threads));
            pool.install(|| {
                assert_eq!(
                    if threads > 0 {
                        threads
                    } else {
                        mgr.available().get()
                    },
                    rayon::current_num_threads()
                );
            });
        }
    }

    #[test]
    fn test_concurrent_map_fallback() {
        let parallel = NonZero::new(10);
        let single = NonZero::new(1);
        let iter = (0..1).into_iter();

        let map = iter.clone().concurrent_map(parallel, |_| {});
        assert_matches!(map, ConcurrentMapIterator::Parallel(10, _));

        let map = iter.clone().concurrent_map(single, |_| {});
        assert_matches!(map, ConcurrentMapIterator::Single(_));

        scope(|s| {
            let map = iter.clone().concurrent_map_scoped(parallel, s, |_| {});
            assert_matches!(map, ConcurrentMapIterator::Parallel(10, _));

            let map = iter.clone().concurrent_map_scoped(single, s, |_| {});
            assert_matches!(map, ConcurrentMapIterator::Single(_));
        });
    }
}

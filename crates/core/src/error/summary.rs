//! An informative summary system for aggregating and condensing data collected
//! from runtime checks, including warnings, issues, and operational metrics.
//!
//! This system should provide end-users with a clear, concise summary of command
//! execution results without conflicting with existing error-handling standards.
//! In scenarios where execution cannot proceed due to a critical error, a
//! `RusticError` will be raised instead, and no summary will be provided.
//!
//! # Separation of Concerns
//!
//! Critical runtime errors that prevent further execution are handled through the
//! existing `RusticError` system. The `Summary` will only collect information for
//! non-fatal events.
//!
//! # Compatibility with Existing Error Handling
//!
//! Summaries must coexist with error propagation rules. They will not replace
//! the core behavior of error propagation but act as a complementary mechanism
//! for presenting non-fatal feedback.
//!
//! # User-Friendly Reporting
//!
//! Summaries should aggregate detailed runtime information—such as warnings,
//! issues, and metrics — in a clear and condensed format for the end-user.
//!
//! # Aggregation & Condensation
//!
//! Similar or repeated errors should be aggregated to avoid redundant information,
//! presenting users with a high-level overview.

use std::{
    collections::{BTreeMap, HashSet},
    fmt::{self, Display},
    time::Instant,
};

use ecow::EcoString;

pub type IssueIdentifier = EcoString;

pub type Issues = BTreeMap<IssueScope, BTreeMap<IssueIdentifier, CondensedIssue>>;
pub type Metrics = BTreeMap<EcoString, EcoString>;

#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    derive_more::Display,
    serde::Serialize,
)]
pub enum IssueScope {
    #[default]
    Internal,
    Unknown,
    UserInput,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct CondensedIssue {
    /// High-level description of the problem
    message: EcoString,

    /// Number of occurrences
    count: usize,

    /// Optional diagnostic information, e.g. an error message
    root_cause: Option<EcoString>,
}

#[derive(
    Debug, Clone, Copy, Default, Hash, PartialEq, Eq, derive_more::Display, serde::Serialize,
)]
pub enum DisplayOptionKind {
    #[default]
    Issues,
    Timing,
    Metrics,
    All,
}

#[derive(Debug, Clone)]
pub struct Summary {
    /// Name of the active context, e.g. a command or operation
    context: EcoString,

    /// Start time of the collection
    // Instant cannot be (de-)serialized, for an implementation see:
    // https://github.com/serde-rs/serde/issues/1375#issuecomment-419688068
    start_time: Instant,

    /// End time, when the collection is completed
    // Serialization: See note above
    end_time: Option<Instant>,

    /// Collection of non-critical warnings   
    issues: Issues,

    /// Optional custom metrics collected during execution
    metrics: Metrics,

    /// Display this data
    display: HashSet<DisplayOptionKind>,
}

impl Summary {
    /// Constructor to create an initial empty Summary
    pub fn new(context: &str) -> Self {
        Self {
            context: context.into(),
            start_time: Instant::now(),
            end_time: None,
            issues: Issues::default(),
            metrics: BTreeMap::default(),
            display: HashSet::from([DisplayOptionKind::default()]),
        }
    }

    /// Marks the summary as completed, capturing the end time.
    pub fn complete(&mut self) {
        self.end_time = Some(Instant::now());
    }

    /// Adds a new issue to the summary, condensing similar issues
    pub fn add_issue(&mut self, scope: IssueScope, message: &str, root_cause: Option<&str>) {
        _ = self
            .issues
            .entry(scope)
            .or_default()
            .entry(message.into())
            .and_modify(|val| {
                val.count += 1;
                if val.root_cause.is_none() {
                    val.root_cause = root_cause.map(Into::into);
                }
            })
            .or_insert(CondensedIssue {
                message: message.into(),
                count: 1,
                root_cause: root_cause.map(Into::into),
            });
    }

    /// Adds a custom metric
    pub fn add_metric(&mut self, key: &str, value: &str) {
        _ = self
            .metrics
            .entry(key.into())
            .and_modify(|val| *val = value.into())
            .or_insert_with(|| value.into());
    }

    pub fn export_issues(&mut self) -> bool {
        self.display.insert(DisplayOptionKind::Issues)
    }

    pub fn export_timing(&mut self) -> bool {
        self.display.insert(DisplayOptionKind::Timing)
    }

    pub fn export_metrics(&mut self) -> bool {
        self.display.insert(DisplayOptionKind::Metrics)
    }

    pub fn export_all(&mut self) -> bool {
        self.display.insert(DisplayOptionKind::All)
    }

    pub fn export_none(&mut self) {
        self.display.clear();
    }

    pub fn set_export(&mut self, option: DisplayOptionKind) -> bool {
        self.display.clear();
        self.display.insert(option)
    }
}

// Display Helpers
impl Summary {
    fn should_display_timing(&self) -> bool {
        !self.display.is_disjoint(&HashSet::from([
            DisplayOptionKind::Timing,
            DisplayOptionKind::All,
        ]))
    }

    fn should_display_issues(&self) -> bool {
        !self.display.is_disjoint(&HashSet::from([
            DisplayOptionKind::Issues,
            DisplayOptionKind::All,
        ]))
    }

    fn should_display_metrics(&self) -> bool {
        !self.display.is_disjoint(&HashSet::from([
            DisplayOptionKind::Metrics,
            DisplayOptionKind::All,
        ]))
    }

    fn display_timing(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f)?;
        if let Some(end_time) = self.end_time {
            let duration = end_time.duration_since(self.start_time);
            let human_duration = humantime::format_duration(duration);

            writeln!(f, "Execution Time: {human_duration}")?;
        }

        Ok(())
    }

    fn display_issues(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f)?;
        writeln!(f, "Issues Encountered:")?;
        for (scope, scoped_issues) in &self.issues {
            writeln!(f, "  Scope: {scope}")?;
            for (message, issue) in scoped_issues {
                let root_cause_info = issue
                    .root_cause
                    .as_ref()
                    .map_or_else(String::new, |root| format!(" (Root Cause: {root})"));

                writeln!(
                    f,
                    "    {} - Occurrences: {}{}",
                    message, issue.count, root_cause_info
                )?;
            }
        }

        Ok(())
    }

    fn display_metrics(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f)?;
        writeln!(f, "Metrics:")?;
        for (key, value) in &self.metrics {
            writeln!(f, "  {key}: {value}")?;
        }

        Ok(())
    }
}

impl Display for Summary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // General context information
        writeln!(f, "Context: {}", self.context)?;

        if self.should_display_timing() {
            self.display_timing(f)?;
        }

        if !self.issues.is_empty() && self.should_display_issues() {
            self.display_issues(f)?;
        }

        if !self.metrics.is_empty() && self.should_display_metrics() {
            self.display_metrics(f)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use insta::assert_snapshot;
    use rstest::rstest;

    use super::*;

    #[test]
    fn test_summary_completion_and_display_passes() {
        let mut summary = Summary::new("test_command");

        summary.complete();

        assert!(summary.end_time.is_some());
    }

    #[test]
    fn test_add_issue_passes() {
        let mut summary = Summary::new("test_command");

        summary.add_issue(
            IssueScope::UserInput,
            "Invalid input",
            Some("Missing field"),
        );

        assert_eq!(summary.issues.len(), 1);

        let user_input_issues = summary.issues.get(&IssueScope::UserInput).unwrap();

        let issue = user_input_issues.get("Invalid input").unwrap();

        assert_eq!(issue.count, 1);

        assert_eq!(issue.root_cause.as_deref(), Some("Missing field"));
    }

    #[test]
    fn test_add_issue_aggregation() {
        let mut summary = Summary::new("test_command");

        summary.add_issue(
            IssueScope::UserInput,
            "Invalid input",
            Some("Missing field"),
        );

        summary.add_issue(
            IssueScope::UserInput,
            "Invalid input",
            Some("Missing field"),
        );

        assert_eq!(summary.issues.len(), 1);

        let user_input_issues = summary.issues.get(&IssueScope::UserInput).unwrap();

        let issue = user_input_issues.get("Invalid input").unwrap();

        assert_eq!(issue.count, 2);
    }

    #[test]
    fn test_add_metric() {
        let mut summary = Summary::new("test_command");

        summary.add_metric("execution_time", "5s");

        assert_eq!(summary.metrics.len(), 1);

        assert_eq!(summary.metrics.get("execution_time").unwrap(), "5s");
    }

    #[rstest]
    #[case(DisplayOptionKind::Issues)]
    #[case(DisplayOptionKind::Timing)]
    #[case(DisplayOptionKind::Metrics)]
    #[case(DisplayOptionKind::All)]
    fn test_summary_display(#[case] display: DisplayOptionKind) {
        let mut summary = Summary::new("Check");
        _ = summary.set_export(display);

        summary.add_issue(
            IssueScope::UserInput,
            "Invalid input",
            Some("Missing field"),
        );

        summary.add_issue(
            IssueScope::UserInput,
            "Invalid input",
            Some("Missing field"),
        );

        summary.add_issue(
            IssueScope::Internal,
            "Pack not found",
            Some("Inconsistent state on disk"),
        );

        summary.add_metric("execution_time", "5s");

        summary.complete();

        let display_output = format!("{summary}");

        assert!(display_output.contains("Context: Check"));

        match display {
            DisplayOptionKind::Issues => {
                assert!(display_output.contains("Issues Encountered:"));
                assert!(display_output.contains("Scope: UserInput"));

                assert!(display_output
                    .contains("Invalid input - Occurrences: 2 (Root Cause: Missing field)"));

                assert_snapshot!(display.to_string(), display_output);
            }
            DisplayOptionKind::Timing => {
                assert!(display_output.contains("Execution Time:"));
            }
            DisplayOptionKind::Metrics => {
                assert!(display_output.contains("Metrics:"));

                assert!(display_output.contains("execution_time: 5s"));

                assert_snapshot!(display.to_string(), display_output);
            }
            DisplayOptionKind::All => {
                assert!(display_output.contains("Issues Encountered:"));
                assert!(display_output.contains("Scope: UserInput"));

                assert!(display_output
                    .contains("Invalid input - Occurrences: 2 (Root Cause: Missing field)"));

                assert!(display_output.contains("Execution Time:"));

                assert!(display_output.contains("Metrics:"));

                assert!(display_output.contains("execution_time: 5s"));
            }
        }
    }
}

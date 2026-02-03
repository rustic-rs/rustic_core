//! Tests for warm-up batch functionality

use std::{
    fs::{self, File},
    io::Read,
    path::Path,
    sync::Arc,
};

use anyhow::Result;
use rstest::rstest;
use serde::{Deserialize, Serialize};
use tempfile::tempdir;

use rustic_core::{
    CommandInput, Id, NoProgressBars, RepositoryBackends, RepositoryOptions, repofile::PackId,
};
use rustic_testing::backend::in_memory_backend::InMemoryBackend;

type RepoOpen = rustic_core::Repository<NoProgressBars, rustic_core::OpenStatus>;

// Test constants
const DEFAULT_BATCH_SIZE: usize = 1;
const PACK_ID_HEX_LENGTH: usize = 64;

/// Helper to create a test script that logs invocations and arguments
/// Returns the tempdir (to keep it alive) and the command
///
/// # Arguments
/// * `log_dir` - Directory where log files will be recorded (each process writes to its own file)
/// * `exit_code` - Exit code the script should return (0 for success, non-zero for failure)
#[cfg(not(windows))]
fn create_test_script_with_exit_code(
    log_dir: &Path,
    exit_code: i32,
) -> Result<(tempfile::TempDir, CommandInput)> {
    let dir = tempdir()?;
    let script_name = if exit_code == 0 {
        "test_warm_up.sh"
    } else {
        "test_warm_up_fail.sh"
    };
    let script_path = dir.path().join(script_name);
    let log_dir_path = log_dir.to_string_lossy();

    let exit_line = if exit_code == 0 {
        String::new()
    } else {
        format!("# Exit with error\nexit {exit_code}\n")
    };

    let script_content = format!(
        r#"#!/usr/bin/env bash
# Log that the script was called (each process writes to its own file to avoid interleaving)
LOG_FILE="{log_dir_path}/warmup_$$.log"
echo "CALL" >> "$LOG_FILE"
# Log the number of arguments
echo "ARGC:$#" >> "$LOG_FILE"
# Log each argument
for arg in "$@"; do
  echo "ARG:$arg" >> "$LOG_FILE"
done
{exit_line}
"#,
    );

    // Write the script and sync to disk to avoid "Text file busy" errors
    {
        use std::io::Write;
        let mut file = File::create(&script_path)?;
        file.write_all(script_content.as_bytes())?;
        file.sync_all()?;
        // Explicitly drop the file handle before setting permissions
        drop(file);
    }

    // Make script executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script_path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms)?;

        // Sync the parent directory to ensure metadata changes are committed
        if let Some(parent) = script_path.parent() {
            if let Ok(dir_file) = File::open(parent) {
                let _ = dir_file.sync_all();
            }
        }
    }

    let command: CommandInput = script_path.to_string_lossy().to_string().parse()?;

    Ok((dir, command))
}

/// Helper to create a test script that succeeds
#[cfg(not(windows))]
fn create_test_script(log_dir: &Path) -> Result<(tempfile::TempDir, CommandInput)> {
    create_test_script_with_exit_code(log_dir, 0)
}

/// Helper to create a test script that fails (exits with non-zero status)
#[cfg(not(windows))]
fn create_failing_script(log_dir: &Path) -> Result<(tempfile::TempDir, CommandInput)> {
    create_test_script_with_exit_code(log_dir, 1)
}

/// Helper to parse log files in a directory and extract call count and arguments
#[cfg(not(windows))]
fn parse_log_files(log_dir: &Path) -> Result<(usize, Vec<Vec<String>>)> {
    let mut all_args = Vec::new();

    // Read all log files matching the pattern warmup_*.log
    for entry in fs::read_dir(log_dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_file() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with("warmup_")
                    && Path::new(name)
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("log"))
                {
                    let mut content = String::new();
                    let _ = File::open(&path)?.read_to_string(&mut content)?;

                    let lines: Vec<&str> = content.lines().collect();
                    let mut current_args = Vec::new();

                    for line in lines {
                        if line == "CALL" {
                            if !current_args.is_empty() {
                                all_args.push(current_args.clone());
                                current_args.clear();
                            }
                        } else if let Some(arg) = line.strip_prefix("ARG:") {
                            current_args.push(arg.to_string());
                        }
                    }

                    if !current_args.is_empty() {
                        all_args.push(current_args);
                    }
                }
            }
        }
    }

    let call_count = all_args.len();

    Ok((call_count, all_args))
}

/// Helper to create a list of mock `PackId`s
#[allow(clippy::cast_possible_truncation)]
fn create_test_ids(count: usize) -> Vec<PackId> {
    (0..count)
        .map(|i| {
            let mut bytes = [0u8; 32];
            bytes[0] = (i >> 24) as u8;
            bytes[1] = (i >> 16) as u8;
            bytes[2] = (i >> 8) as u8;
            bytes[3] = i as u8;
            PackId::from(Id::new(bytes))
        })
        .collect()
}

/// Helper to create a test repository with warm-up configuration
fn create_test_repo(
    command: CommandInput,
    batch_size: usize,
) -> Result<rustic_core::Repository<NoProgressBars, ()>> {
    let be = InMemoryBackend::new();
    let be = RepositoryBackends::new(Arc::new(be), None);
    let options = RepositoryOptions::default()
        .warm_up_command(command)
        .warm_up_batch(batch_size);

    rustic_core::Repository::new(&options, &be).map_err(Into::into)
}

/// Helper to parse log files and assert call count
/// Returns the parsed arguments for further verification
#[cfg(not(windows))]
fn assert_call_count(log_dir: &Path, expected: usize, context: &str) -> Result<Vec<Vec<String>>> {
    let (call_count, all_args) = parse_log_files(log_dir)?;
    assert_eq!(call_count, expected, "{context}");
    Ok(all_args)
}

/// Helper to verify batch distribution across multiple calls (order-independent)
#[cfg(not(windows))]
fn verify_batch_distribution(all_args: &[Vec<String>], num_packs: usize, batch_size: usize) {
    // Verify total arguments across all calls equals number of packs
    let total_args: usize = all_args.iter().map(Vec::len).sum();
    assert_eq!(
        total_args, num_packs,
        "Total arguments should equal number of packs"
    );

    // Calculate expected batch sizes
    let num_full_batches = num_packs / batch_size;
    let remainder = num_packs % batch_size;

    // Count actual batch sizes
    let mut size_counts: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    for args in all_args {
        *size_counts.entry(args.len()).or_insert(0) += 1;
    }

    // Verify full batch count
    if num_full_batches > 0 {
        assert_eq!(
            size_counts.get(&batch_size).copied().unwrap_or(0),
            num_full_batches,
            "Should have {num_full_batches} batches of size {batch_size}, got {size_counts:?}"
        );
    }

    // Verify remainder batch (if any)
    if remainder > 0 {
        assert_eq!(
            size_counts.get(&remainder).copied().unwrap_or(0),
            1,
            "Should have 1 batch of size {remainder}, got {size_counts:?}"
        );
    } else {
        assert!(
            !size_counts.contains_key(&batch_size) || size_counts.len() == 1,
            "No remainder expected, but found multiple batch sizes: {size_counts:?}"
        );
    }
}

#[cfg(not(windows))]
#[rstest]
#[case(1, 10, 10)] // batch_size=1, num_packs=10, expected_calls=10
#[case(5, 10, 2)] // batch_size=5, num_packs=10, expected_calls=2
#[case(10, 10, 1)] // batch_size=10, num_packs=10, expected_calls=1
#[case(20, 10, 1)] // batch_size=20 (larger than pack count), expected_calls=1
#[case(1, 1, 1)] // edge case: single pack
#[case(3, 7, 3)] // non-even division: 3+3+1
fn test_warm_up_batch_args_mode(
    #[case] batch_size: usize,
    #[case] num_packs: usize,
    #[case] expected_calls: usize,
) -> Result<()> {
    let log_dir = tempdir()?;
    let (_script_dir, mut command) = create_test_script(log_dir.path())?;

    // Modify command to include %ids placeholder for plural mode
    let cmd_str = format!("{} %ids", command.command());
    command = cmd_str.parse()?;

    let repo = create_test_repo(command, batch_size)?;
    let pack_ids = create_test_ids(num_packs);

    repo.warm_up(pack_ids.iter().copied())?;

    let all_args = assert_call_count(
        log_dir.path(),
        expected_calls,
        &format!("Command should be called {expected_calls} times"),
    )?;
    verify_batch_distribution(&all_args, num_packs, batch_size);

    Ok(())
}

#[cfg(not(windows))]
#[rstest]
#[case(1, 10, 10)] // batch_size=1, num_packs=10, expected_calls=10
#[case(5, 10, 10)] // batch_size=5, num_packs=10, expected_calls=10 (still one per pack in singular mode)
#[case(10, 1, 1)] // batch_size=10, num_packs=1, expected_calls=1
fn test_warm_up_batch_variable_mode(
    #[case] batch_size: usize,
    #[case] num_packs: usize,
    #[case] expected_calls: usize,
) -> Result<()> {
    let log_dir = tempdir()?;
    let (_script_dir, mut command) = create_test_script(log_dir.path())?;

    // Modify command to include %id placeholder for singular mode
    let cmd_str = format!("{} %id", command.command());
    command = cmd_str.parse()?;

    let repo = create_test_repo(command, batch_size)?;
    let pack_ids = create_test_ids(num_packs);

    repo.warm_up(pack_ids.iter().copied())?;

    let all_args = assert_call_count(
        log_dir.path(),
        expected_calls,
        &format!("Command should be called {expected_calls} times in singular mode"),
    )?;

    // Each call should have exactly 1 argument (the pack ID)
    for (i, args) in all_args.iter().enumerate() {
        assert_eq!(
            args.len(),
            1,
            "Call {} should have exactly 1 argument in singular mode",
            i + 1
        );
    }

    Ok(())
}

#[cfg(not(windows))]
#[rstest]
#[case(1, 10, 2000, 3500)] // batch_size=1, num_packs=10, sequential: ~2000ms
#[case(10, 10, 200, 1000)] // batch_size=10, num_packs=10, parallel: ~200ms
fn test_warm_up_parallel_singular_mode(
    #[case] batch_size: usize,
    #[case] num_packs: usize,
    #[case] min_ms: u128,
    #[case] max_ms: u128,
) -> Result<()> {
    let log_dir = tempdir()?;
    let log_dir_path = log_dir.path().to_string_lossy();

    // Script that sleeps for 200ms and logs calls
    let dir = tempdir()?;
    let script_path = dir.path().join("sleep_warm_up.sh");

    let script_content = format!(
        r#"#!/usr/bin/env bash
LOG_FILE="{log_dir_path}/warmup_$$.log"
echo "CALL:$1:$(date +%s%3N)" >> "$LOG_FILE"
sleep 0.2
"#,
    );

    // Write the script and sync to disk to avoid "Text file busy" errors
    {
        use std::io::Write;
        let mut file = File::create(&script_path)?;
        file.write_all(script_content.as_bytes())?;
        file.sync_all()?;
        // Explicitly drop the file handle before setting permissions
        drop(file);
    }

    // Make script executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script_path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms)?;

        // Sync the parent directory to ensure metadata changes are committed
        if let Some(parent) = script_path.parent() {
            if let Ok(dir_file) = File::open(parent) {
                let _ = dir_file.sync_all();
            }
        }
    }

    let cmd_str = format!("{} %id", script_path.to_string_lossy());
    let command: CommandInput = cmd_str.parse()?;

    let repo = create_test_repo(command, batch_size)?;
    let pack_ids = create_test_ids(num_packs);

    let start_time = std::time::Instant::now();
    repo.warm_up(pack_ids.iter().copied())?;
    let elapsed = start_time.elapsed();

    let elapsed_ms = elapsed.as_millis();
    assert!(
        elapsed_ms >= min_ms && elapsed_ms <= max_ms,
        "Expected {min_ms}-{max_ms}ms for batch_size={batch_size}, num_packs={num_packs}, got {elapsed_ms}ms"
    );

    Ok(())
}

#[test]
fn test_warm_up_batch_default_value() {
    let options = RepositoryOptions::default();
    assert_eq!(
        options.warm_up_batch, None,
        "warm_up_batch should default to None"
    );
}

#[test]
fn test_validation_requires_placeholder() -> Result<()> {
    let be = InMemoryBackend::new();
    let be = RepositoryBackends::new(Arc::new(be), None);

    // Command without any placeholder should fail
    let command: CommandInput = "echo test".parse()?;
    let options = RepositoryOptions::default().warm_up_command(command);

    let result = rustic_core::Repository::<NoProgressBars, ()>::new(&options, &be);

    assert!(
        result.is_err(),
        "Should fail when no placeholder is present"
    );
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("placeholder"),
        "Error should mention placeholder"
    );

    Ok(())
}

#[test]
fn test_validation_mixing_singular_and_plural() -> Result<()> {
    let be = InMemoryBackend::new();
    let be = RepositoryBackends::new(Arc::new(be), None);

    // Command mixing singular and plural placeholders should fail
    let command: CommandInput = "echo %ids %path".parse()?;
    let options = RepositoryOptions::default().warm_up_command(command);

    let result = rustic_core::Repository::<NoProgressBars, ()>::new(&options, &be);

    assert!(
        result.is_err(),
        "Should fail when mixing singular and plural placeholders"
    );
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("Cannot mix")
            || err.to_string().contains("singular")
            || err.to_string().contains("plural"),
        "Error should mention not mixing singular and plural placeholders"
    );

    Ok(())
}

#[test]
fn test_validation_valid_singular_placeholders() -> Result<()> {
    let be = InMemoryBackend::new();
    let be = RepositoryBackends::new(Arc::new(be), None);

    // Valid singular placeholders should succeed
    let valid_commands = vec!["echo %id", "echo %path", "echo %id %path", "echo %id %id"];
    for cmd_str in valid_commands {
        let command: CommandInput = cmd_str.parse()?;
        let options = RepositoryOptions::default().warm_up_command(command);

        let result = rustic_core::Repository::<NoProgressBars, ()>::new(&options, &be);
        assert!(result.is_ok(), "Command with '{cmd_str}' should succeed");
    }

    Ok(())
}

#[test]
fn test_validation_valid_plural_placeholders() -> Result<()> {
    let be = InMemoryBackend::new();
    let be = RepositoryBackends::new(Arc::new(be), None);

    // Valid plural placeholders should succeed
    let valid_commands = vec![
        "echo %ids",
        "echo %paths",
        "echo %ids %paths",
        "echo %ids %ids",
    ];
    for cmd_str in valid_commands {
        let command: CommandInput = cmd_str.parse()?;
        let options = RepositoryOptions::default().warm_up_command(command);

        let result = rustic_core::Repository::<NoProgressBars, ()>::new(&options, &be);
        assert!(result.is_ok(), "Command with '{cmd_str}' should succeed");
    }

    Ok(())
}

#[cfg(not(windows))]
#[test]
fn test_warm_up_argv_mode_with_existing_args() -> Result<()> {
    let log_dir = tempdir()?;
    let (_script_dir, command) = create_test_script(log_dir.path())?;

    // Add extra arguments and %ids placeholder to the command
    let cmd_str = format!("{} --flag value %ids", command.command());
    let command: CommandInput = cmd_str.parse()?;

    let repo = create_test_repo(command, 5)?;
    let pack_ids = create_test_ids(3);

    repo.warm_up(pack_ids.iter().copied())?;

    let all_args = assert_call_count(
        log_dir.path(),
        1,
        "Should be called once with batch_size=5 and 3 packs",
    )?;
    assert_eq!(
        all_args[0].len(),
        5,
        "Should have 2 existing args + 3 pack IDs"
    );

    // First two args should be existing arguments
    assert_eq!(all_args[0][0], "--flag");
    assert_eq!(all_args[0][1], "value");

    // Remaining args should be pack IDs (hex strings)
    for arg in &all_args[0][2..] {
        assert_eq!(
            arg.len(),
            PACK_ID_HEX_LENGTH,
            "Pack IDs should be {PACK_ID_HEX_LENGTH}-character hex strings"
        );
    }

    Ok(())
}

#[test]
fn test_toml_serialization_with_warm_up_batch() -> Result<()> {
    #[derive(Deserialize, Serialize, Debug)]
    struct TestConfig {
        #[serde(flatten)]
        repo: RepositoryOptions,
    }

    let toml_str = r#"
 warm-up-command = "echo %id"
 warm-up-batch = 100
 "#;

    let config: TestConfig = toml::from_str(toml_str)?;

    assert_eq!(config.repo.warm_up_batch, Some(100));

    // Test round-trip serialization
    let serialized = toml::to_string(&config)?;
    let deserialized: TestConfig = toml::from_str(&serialized)?;

    assert_eq!(config.repo.warm_up_batch, deserialized.repo.warm_up_batch);

    Ok(())
}

#[test]
fn test_toml_deserialization_with_defaults() -> Result<()> {
    #[derive(Deserialize, Serialize)]
    struct TestConfig {
        #[serde(flatten)]
        repo: RepositoryOptions,
    }

    // TOML without warm-up-batch should use default value
    let toml_str = r#"
 warm-up-command = "echo %id"
 "#;

    let config: TestConfig = toml::from_str(toml_str)?;

    assert_eq!(
        config.repo.warm_up_batch, None,
        "Should default to None when not specified"
    );

    Ok(())
}

#[test]
fn test_toml_placeholder_variations() -> Result<()> {
    #[derive(Deserialize, Serialize)]
    struct TestConfig {
        #[serde(flatten)]
        repo: RepositoryOptions,
    }

    // Test singular placeholder variations
    let toml_singular = r#"
  warm-up-command = "echo %id %path"
  "#;

    let config: TestConfig = toml::from_str(toml_singular)?;
    let cmd_str = config.repo.warm_up_command.unwrap().to_string();
    assert!(
        cmd_str.contains("echo") && cmd_str.contains("%id") && cmd_str.contains("%path"),
        "Command should contain echo %id %path, got: {cmd_str}"
    );

    // Test plural placeholder variations
    let toml_plural = r#"
 warm-up-command = "echo %ids %paths"
  "#;

    let config: TestConfig = toml::from_str(toml_plural)?;
    let cmd_str = config.repo.warm_up_command.unwrap().to_string();
    assert!(
        cmd_str.contains("echo") && cmd_str.contains("%ids") && cmd_str.contains("%paths"),
        "Command should contain echo %ids %paths, got: {cmd_str}"
    );

    Ok(())
}

#[cfg(not(windows))]
#[rstest]
#[case("%id", 5, 3, 3, true)] // singular mode: all 3 spawned, all fail
#[case("%ids", 5, 3, 1, true)] // plural mode: fails on first (only) batch
#[case("%id", 1, 10, 1, true)] // singular mode with many packs: fails on first batch
fn test_warm_up_command_failure(
    #[case] placeholder: &str,
    #[case] batch_size: usize,
    #[case] num_packs: usize,
    #[case] expected_calls: usize,
    #[case] needs_id_placeholder: bool,
) -> Result<()> {
    let log_dir = tempdir()?;
    let (_script_dir, mut command) = create_failing_script(log_dir.path())?;

    if needs_id_placeholder {
        let cmd_str = format!("{} {}", command.command(), placeholder);
        command = cmd_str.parse()?;
    }

    let repo = create_test_repo(command, batch_size)?;
    let pack_ids = create_test_ids(num_packs);

    let result = repo.warm_up(pack_ids.iter().copied());

    assert!(
        result.is_err(),
        "warm_up should return error when command fails"
    );
    let err_str = result.unwrap_err().to_string();
    assert!(
        err_str.contains("command failed")
            || err_str.contains("ExternalCommand")
            || err_str.contains("Error in executing"),
        "Error should indicate command failure: {err_str}"
    );

    let (call_count, _) = parse_log_files(log_dir.path())?;
    assert_eq!(
        call_count, expected_calls,
        "Script should have been called {expected_calls} times before failing"
    );

    Ok(())
}

#[cfg(not(windows))]
#[test]
fn test_warm_up_backend_path_mode() -> Result<()> {
    let log_dir = tempdir()?;
    let (_script_dir, mut command) = create_test_script(log_dir.path())?;

    // Add %paths placeholder to use backend paths instead of pack IDs
    let cmd_str = format!("{} %paths", command.command());
    command = cmd_str.parse()?;

    // Test with backend paths
    let repo = create_test_repo(command, 3)?;

    // Create test pack IDs
    let pack_ids = create_test_ids(2); // 2 packs for easier testing

    repo.warm_up(pack_ids.iter().copied())?;

    // Verify the warmup command was called once with backend paths
    let all_args = assert_call_count(
        log_dir.path(),
        1,
        "Command should be called once with backend paths",
    )?;

    // Verify that backend paths were passed instead of pack IDs
    assert_eq!(all_args.len(), 1, "Should have one call with backend paths");
    let args = &all_args[0];
    assert_eq!(args.len(), 2, "Should have 2 backend path arguments");

    // The backend paths should follow the pattern: "data/XX/fullpackid"
    for (i, arg) in args.iter().enumerate() {
        // Verify it's a backend path, not a pack ID
        assert!(
            arg.starts_with("data/"),
            "Argument {} should be a backend path starting with 'data/', got: {arg}",
            i + 1
        );

        // Verify the format: data/XX/full_hex_id
        assert!(
            arg.len() >= 4 + 2 + 64, // "data/" + "XX" + 64-char hex ID
            "Backend path should have sufficient length: {arg}"
        );

        // Verify the hex ID part is valid hex
        let parts: Vec<&str> = arg.split('/').collect();
        assert_eq!(
            parts.len(),
            3,
            "Backend path should have 3 parts: data, XX, hex_id"
        );
        assert_eq!(parts[0], "data", "First part should be 'data'");
        assert_eq!(
            parts[1].len(),
            2,
            "Second part should be 2-character prefix"
        );
        assert_eq!(
            parts[2].len(),
            64,
            "Third part should be 64-character hex ID"
        );
    }

    Ok(())
}

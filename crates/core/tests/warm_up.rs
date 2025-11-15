//! Tests for warm-up batch functionality

use std::{
    fs::{self, File},
    io::Read,
    path::PathBuf,
    sync::Arc,
};

use anyhow::Result;
use rstest::rstest;
use serde::{Deserialize, Serialize};
use tempfile::tempdir;

use rustic_core::{
    Id,
    repofile::PackId,
    CommandInput, NoProgressBars, RepositoryBackends, RepositoryOptions, WarmUpPackIdInput, WarmUpInputType,
};
use rustic_testing::backend::in_memory_backend::InMemoryBackend;

type RepoOpen = rustic_core::Repository<NoProgressBars, rustic_core::OpenStatus>;

// Test constants
const DEFAULT_BATCH_SIZE: usize = 1;
const PACK_ID_HEX_LENGTH: usize = 64;
const TEST_PASSWORD: &str = "test";

/// Helper to create a test script that logs invocations and arguments
/// Returns the tempdir (to keep it alive) and the command
///
/// # Arguments
/// * `log_file` - Path to log file where invocations will be recorded
/// * `exit_code` - Exit code the script should return (0 for success, non-zero for failure)
#[cfg(not(windows))]
fn create_test_script_with_exit_code(log_file: &PathBuf, exit_code: i32) -> Result<(tempfile::TempDir, CommandInput)> {
    let dir = tempdir()?;
    let script_name = if exit_code == 0 { "test_warm_up.sh" } else { "test_warm_up_fail.sh" };
    let script_path = dir.path().join(script_name);
    let log_path = log_file.to_string_lossy();

    let exit_line = if exit_code == 0 { String::new() } else { format!("# Exit with error\nexit {}\n", exit_code) };

    let script_content = format!(
        r#"#!/usr/bin/env bash
# Log that the script was called
echo "CALL" >> {}
# Log the number of arguments
echo "ARGC:$#" >> {}
# Log each argument
for arg in "$@"; do
    echo "ARG:$arg" >> {}
done
{}
"#,
        log_path, log_path, log_path, exit_line
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
fn create_test_script(log_file: &PathBuf) -> Result<(tempfile::TempDir, CommandInput)> {
    create_test_script_with_exit_code(log_file, 0)
}

/// Helper to create a test script that fails (exits with non-zero status)
#[cfg(not(windows))]
fn create_failing_script(log_file: &PathBuf) -> Result<(tempfile::TempDir, CommandInput)> {
    create_test_script_with_exit_code(log_file, 1)
}

/// Helper to parse log file and extract call count and arguments
#[cfg(not(windows))]
fn parse_log_file(log_file: &PathBuf) -> Result<(usize, Vec<Vec<String>>)> {
    let mut content = String::new();
    let _ = File::open(log_file)?.read_to_string(&mut content)?;

    let lines: Vec<&str> = content.lines().collect();
    let call_count = lines.iter().filter(|line| **line == "CALL").count();

    let mut all_args = Vec::new();
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

    Ok((call_count, all_args))
}

/// Helper to create a list of mock PackIds
fn create_test_pack_ids(count: usize) -> Vec<PackId> {
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
    input_mode: WarmUpPackIdInput,
) -> Result<rustic_core::Repository<NoProgressBars, ()>> {
    create_test_repo_with_input_type(command, batch_size, input_mode, WarmUpInputType::PackId)
}

/// Helper to create a test repository with warm-up configuration and specific input type
fn create_test_repo_with_input_type(
    command: CommandInput,
    batch_size: usize,
    input_mode: WarmUpPackIdInput,
    input_type: WarmUpInputType,
) -> Result<rustic_core::Repository<NoProgressBars, ()>> {
    let be = InMemoryBackend::new();
    let be = RepositoryBackends::new(Arc::new(be), None);
    let options = RepositoryOptions::default()
        .password(TEST_PASSWORD)
        .warm_up_command(command)
        .warm_up_batch(batch_size)
        .warm_up_pack_id_input(input_mode)
        .warm_up_input_type(input_type);

    rustic_core::Repository::new(&options, &be).map_err(|e| e.into())
}

/// Helper to parse log file and assert call count
/// Returns the parsed arguments for further verification
#[cfg(not(windows))]
fn assert_call_count(log_file: &PathBuf, expected: usize, context: &str) -> Result<Vec<Vec<String>>> {
    let (call_count, all_args) = parse_log_file(log_file)?;
    assert_eq!(call_count, expected, "{}", context);
    Ok(all_args)
}

/// Helper to verify batch distribution across multiple calls
#[cfg(not(windows))]
fn verify_batch_distribution(all_args: &[Vec<String>], num_packs: usize, batch_size: usize) -> Result<()> {
    // Verify total arguments across all calls equals number of packs
    let total_args: usize = all_args.iter().map(|args| args.len()).sum();
    assert_eq!(total_args, num_packs, "Total arguments should equal number of packs");

    // Verify each call has the expected batch size (except possibly the last one)
    for (i, args) in all_args.iter().enumerate() {
        let expected_batch = if i == all_args.len() - 1 {
            // Last batch might be smaller
            let remainder = num_packs % batch_size;
            if remainder == 0 { batch_size } else { remainder }
        } else {
            batch_size
        };
        assert_eq!(args.len(), expected_batch, "Call {} should have {} arguments", i + 1, expected_batch);
    }

    Ok(())
}

#[cfg(not(windows))]
#[rstest]
#[case(1, 10, 10)]  // batch_size=1, num_packs=10, expected_calls=10
#[case(5, 10, 2)]   // batch_size=5, num_packs=10, expected_calls=2
#[case(10, 10, 1)]  // batch_size=10, num_packs=10, expected_calls=1
#[case(20, 10, 1)]  // batch_size=20 (larger than pack count), expected_calls=1
#[case(1, 1, 1)]    // edge case: single pack
#[case(3, 7, 3)]    // non-even division: 3+3+1
fn test_warm_up_batch_argv_mode(
    #[case] batch_size: usize,
    #[case] num_packs: usize,
    #[case] expected_calls: usize,
) -> Result<()> {
    let log_dir = tempdir()?;
    let log_file = log_dir.path().join("warmup.log");
    let (_script_dir, command) = create_test_script(&log_file)?;

    let repo = create_test_repo(command, batch_size, WarmUpPackIdInput::Argv)?;
    let pack_ids = create_test_pack_ids(num_packs);

    repo.warm_up(pack_ids.iter().copied())?;

    let all_args = assert_call_count(&log_file, expected_calls,
        &format!("Command should be called {} times", expected_calls))?;
    verify_batch_distribution(&all_args, num_packs, batch_size)?;

    Ok(())
}

#[cfg(not(windows))]
#[rstest]
#[case(1, 10, 10)]  // batch_size=1, num_packs=10, expected_calls=10
#[case(5, 10, 10)]  // batch_size=5, num_packs=10, expected_calls=10 (still one per pack in anchor mode)
#[case(10, 1, 1)]   // batch_size=10, num_packs=1, expected_calls=1
fn test_warm_up_batch_anchor_mode(
    #[case] batch_size: usize,
    #[case] num_packs: usize,
    #[case] expected_calls: usize,
) -> Result<()> {
    let log_dir = tempdir()?;
    let log_file = log_dir.path().join("warmup.log");
    let (_script_dir, mut command) = create_test_script(&log_file)?;

    // Modify command to include %id placeholder
    let cmd_str = format!("{} %id", command.command());
    command = cmd_str.parse()?;

    let repo = create_test_repo(command, batch_size, WarmUpPackIdInput::Anchor)?;
    let pack_ids = create_test_pack_ids(num_packs);

    repo.warm_up(pack_ids.iter().copied())?;

    let all_args = assert_call_count(&log_file, expected_calls,
        &format!("Command should be called {} times in anchor mode", expected_calls))?;

    // Each call should have exactly 1 argument (the pack ID)
    for (i, args) in all_args.iter().enumerate() {
        assert_eq!(args.len(), 1, "Call {} should have exactly 1 argument in anchor mode", i + 1);
    }

    Ok(())
}

#[test]
fn test_warm_up_batch_default_value() -> Result<()> {
    let options = RepositoryOptions::default();
    assert_eq!(options.warm_up_batch, DEFAULT_BATCH_SIZE, "warm_up_batch should default to {}", DEFAULT_BATCH_SIZE);
    Ok(())
}

#[test]
fn test_warm_up_pack_id_input_default_value() -> Result<()> {
    let options = RepositoryOptions::default();
    assert_eq!(options.warm_up_pack_id_input, None, "warm_up_pack_id_input should default to None");

    // When None, it should be treated as Anchor
    let input_mode = options.warm_up_pack_id_input.unwrap_or_default();
    assert_eq!(input_mode, WarmUpPackIdInput::Anchor, "Default should be Anchor mode");
    Ok(())
}

#[test]
fn test_validation_anchor_mode_requires_id() -> Result<()> {
    let be = InMemoryBackend::new();
    let be = RepositoryBackends::new(Arc::new(be), None);

    // Command without %id in anchor mode should fail
    let command: CommandInput = "echo test".parse()?;
    let options = RepositoryOptions::default()
        .password(TEST_PASSWORD)
        .warm_up_command(command)
        .warm_up_pack_id_input(WarmUpPackIdInput::Anchor);

    let result = rustic_core::Repository::<NoProgressBars, ()>::new(&options, &be);

    assert!(result.is_err(), "Should fail when %id is missing in anchor mode");
    let err = result.unwrap_err();
    assert!(err.to_string().contains("%id") || err.to_string().contains("anchor"),
            "Error should mention %id or anchor mode");

    Ok(())
}

#[test]
fn test_validation_argv_mode_no_id_required() -> Result<()> {
    let be = InMemoryBackend::new();
    let be = RepositoryBackends::new(Arc::new(be), None);

    // Command without %id in argv mode should succeed
    let command: CommandInput = "echo test".parse()?;
    let options = RepositoryOptions::default()
        .password(TEST_PASSWORD)
        .warm_up_command(command)
        .warm_up_pack_id_input(WarmUpPackIdInput::Argv);

    let result = rustic_core::Repository::<NoProgressBars, ()>::new(&options, &be);

    assert!(result.is_ok(), "Should succeed when %id is missing in argv mode");

    Ok(())
}

#[cfg(not(windows))]
#[test]
fn test_warm_up_argv_mode_with_existing_args() -> Result<()> {
    let log_dir = tempdir()?;
    let log_file = log_dir.path().join("warmup.log");
    let (_script_dir, command) = create_test_script(&log_file)?;

    // Add extra arguments to the command
    let cmd_str = format!("{} --flag value", command.command());
    let command: CommandInput = cmd_str.parse()?;

    let repo = create_test_repo(command, 5, WarmUpPackIdInput::Argv)?;
    let pack_ids = create_test_pack_ids(3);

    repo.warm_up(pack_ids.iter().copied())?;

    let all_args = assert_call_count(&log_file, 1, "Should be called once with batch_size=5 and 3 packs")?;
    assert_eq!(all_args[0].len(), 5, "Should have 2 existing args + 3 pack IDs");

    // First two args should be the existing arguments
    assert_eq!(all_args[0][0], "--flag");
    assert_eq!(all_args[0][1], "value");

    // Remaining args should be pack IDs (hex strings)
    for arg in &all_args[0][2..] {
        assert_eq!(arg.len(), PACK_ID_HEX_LENGTH, "Pack IDs should be {}-character hex strings", PACK_ID_HEX_LENGTH);
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
password = "test"
warm-up-command = "echo %id"
warm-up-batch = 100
warm-up-pack-id-input = "argv"
"#;

    let config: TestConfig = toml::from_str(toml_str)?;

    assert_eq!(config.repo.password, Some("test".to_string()));
    assert_eq!(config.repo.warm_up_batch, 100);
    assert_eq!(config.repo.warm_up_pack_id_input, Some(WarmUpPackIdInput::Argv));

    // Test round-trip serialization
    let serialized = toml::to_string(&config)?;
    let deserialized: TestConfig = toml::from_str(&serialized)?;

    assert_eq!(config.repo.warm_up_batch, deserialized.repo.warm_up_batch);
    assert_eq!(config.repo.warm_up_pack_id_input, deserialized.repo.warm_up_pack_id_input);

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
password = "test"
warm-up-command = "echo %id"
"#;

    let config: TestConfig = toml::from_str(toml_str)?;

    assert_eq!(config.repo.warm_up_batch, DEFAULT_BATCH_SIZE, "Should default to {} when not specified", DEFAULT_BATCH_SIZE);
    assert_eq!(config.repo.warm_up_pack_id_input, None, "Should be None when not specified");

    Ok(())
}

#[test]
fn test_toml_warm_up_pack_id_input_values() -> Result<()> {
    #[derive(Deserialize, Serialize)]
    struct TestConfig {
        #[serde(flatten)]
        repo: RepositoryOptions,
    }

    // Test "anchor" value
    let toml_anchor = r#"
password = "test"
warm-up-pack-id-input = "anchor"
"#;

    let config: TestConfig = toml::from_str(toml_anchor)?;
    assert_eq!(config.repo.warm_up_pack_id_input, Some(WarmUpPackIdInput::Anchor));

    // Test "argv" value
    let toml_argv = r#"
password = "test"
warm-up-pack-id-input = "argv"
"#;

    let config: TestConfig = toml::from_str(toml_argv)?;
    assert_eq!(config.repo.warm_up_pack_id_input, Some(WarmUpPackIdInput::Argv));

    Ok(())
}

#[cfg(not(windows))]
#[rstest]
#[case(WarmUpPackIdInput::Anchor, 5, 3, 1, true)]  // anchor mode: fails on first call
#[case(WarmUpPackIdInput::Argv, 5, 3, 1, false)]   // argv mode: fails on first (only) batch
#[case(WarmUpPackIdInput::Anchor, 1, 10, 1, true)] // anchor mode with many packs: aborts early
fn test_warm_up_command_failure(
    #[case] input_mode: WarmUpPackIdInput,
    #[case] batch_size: usize,
    #[case] num_packs: usize,
    #[case] expected_calls: usize,
    #[case] needs_id_placeholder: bool,
) -> Result<()> {
    let log_dir = tempdir()?;
    let log_file = log_dir.path().join("warmup.log");
    let (_script_dir, mut command) = create_failing_script(&log_file)?;

    if needs_id_placeholder {
        let cmd_str = format!("{} %id", command.command());
        command = cmd_str.parse()?;
    }

    let repo = create_test_repo(command, batch_size, input_mode)?;
    let pack_ids = create_test_pack_ids(num_packs);

    let result = repo.warm_up(pack_ids.iter().copied());

    assert!(result.is_err(), "warm_up should return error when command fails");
    let err_str = result.unwrap_err().to_string();
    assert!(
        err_str.contains("command failed") || err_str.contains("ExternalCommand") || err_str.contains("Error in executing"),
        "Error should indicate command failure: {}",
        err_str
    );

    let (call_count, _) = parse_log_file(&log_file)?;
    assert_eq!(call_count, expected_calls, "Script should have been called {} times before failing", expected_calls);

    Ok(())
}

#[cfg(not(windows))]
#[test]
fn test_warm_up_backend_path_mode() -> Result<()> {
    let log_dir = tempdir()?;
    let log_file = log_dir.path().join("warmup.log");
    let (_script_dir, command) = create_test_script(&log_file)?;

    // Test with BackendPath input type
    let repo = create_test_repo_with_input_type(
        command,
        3, // batch_size
        WarmUpPackIdInput::Argv,
        WarmUpInputType::BackendPath
    )?;

    // Create test pack IDs
    let pack_ids = create_test_pack_ids(2); // 2 packs for easier testing

    repo.warm_up(pack_ids.iter().copied())?;

    // Verify the warmup command was called once with backend paths
    let all_args = assert_call_count(&log_file, 1, "Command should be called once with backend paths")?;

    // Verify that backend paths were passed instead of pack IDs
    assert_eq!(all_args.len(), 1, "Should have one call with backend paths");
    let args = &all_args[0];
    assert_eq!(args.len(), 2, "Should have 2 backend path arguments");

    // The backend paths should follow the pattern: "data/XX/fullpackid"
    for (i, arg) in args.iter().enumerate() {
        // Verify it's a backend path, not a pack ID
        assert!(
            arg.starts_with("data/"),
            "Argument {} should be a backend path starting with 'data/', got: {}",
            i + 1,
            arg
        );

        // Verify the format: data/XX/full_hex_id
        assert!(
            arg.len() >= 4 + 2 + 64, // "data/" + "XX" + 64-char hex ID
            "Backend path should have sufficient length: {}",
            arg
        );

        // Verify the hex ID part is valid hex
        let parts: Vec<&str> = arg.split('/').collect();
        assert_eq!(parts.len(), 3, "Backend path should have 3 parts: data, XX, hex_id");
        assert_eq!(parts[0], "data", "First part should be 'data'");
        assert_eq!(parts[1].len(), 2, "Second part should be 2-character prefix");
        assert_eq!(parts[2].len(), 64, "Third part should be 64-character hex ID");
    }

    Ok(())
}

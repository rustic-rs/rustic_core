#![allow(missing_docs)]
#[cfg(not(windows))]
use std::fs::File;

use anyhow::Result;
use rustic_core::CommandInput;
use serde::{Deserialize, Serialize};

#[cfg(not(windows))]
use tempfile::tempdir;

#[test]
fn from_str() -> Result<()> {
    let cmd: CommandInput = "echo test".parse()?;
    assert_eq!(cmd.command(), "echo");
    assert_eq!(cmd.args(), ["test"]);

    let cmd: CommandInput = r#"echo "test test" test"#.parse()?;
    assert_eq!(cmd.command(), "echo");
    assert_eq!(cmd.args(), ["test test", "test"]);
    Ok(())
}

#[cfg(not(windows))]
#[test]
fn from_str_failed() {
    let failed_cmd: std::result::Result<CommandInput, _> = "echo \"test test".parse();
    assert!(failed_cmd.is_err());
}

#[test]
fn toml() -> Result<()> {
    #[derive(Deserialize, Serialize)]
    struct Test {
        command1: CommandInput,
        command2: CommandInput,
        command3: CommandInput,
        command4: CommandInput,
    }

    let test = toml::from_str::<Test>(
        r#"
            command1 = "echo test"
            command2 = {command = "echo", args = ["test test"], on-failure = "error"}
            command3 = {command = "echo", args = ["test test", "test"]}
            command4 = {command = "echo", args = ["test test", "test"], on-failure = "warn"}
        "#,
    )?;

    assert_eq!(test.command1.command(), "echo");
    assert_eq!(test.command1.args(), ["test"]);
    assert_eq!(test.command2.command(), "echo");
    assert_eq!(test.command2.args(), ["test test"]);
    assert_eq!(test.command3.command(), "echo");
    assert_eq!(test.command3.args(), ["test test", "test"]);

    let test_ser = toml::to_string(&test)?;
    assert_eq!(
        test_ser,
        r#"command1 = "echo test"
command2 = "echo 'test test'"
command3 = "echo 'test test' test"

[command4]
command = "echo"
args = ["test test", "test"]
on-failure = "warn"
"#
    );
    Ok(())
}

#[test]
fn run_empty() -> Result<()> {
    // empty command
    let command: CommandInput = "".parse()?;
    dbg!(&command);
    assert!(!command.is_set());
    command.run("test", "empty")?;
    Ok(())
}

#[cfg(not(windows))]
#[test]
fn run_deletey() -> Result<()> {
    // create a tmp file which will be removed by
    let dir = tempdir()?;
    let filename = dir.path().join("file");
    let _ = File::create(&filename)?;
    assert!(filename.exists());

    let command: CommandInput = format!("rm {}", filename.to_str().unwrap()).parse()?;
    assert!(command.is_set());
    command.run("test", "test-call")?;
    assert!(!filename.exists());

    Ok(())
}

use anyhow::Result;
use assert_cmd::Command;
use insta::assert_yaml_snapshot;
use lintrunner::lint_message::{LintMessage, LintSeverity};
use regex::Regex;
use serde_json;
use std::io::Write;

fn assert_output_snapshot(name: &str, cmd: &mut Command) -> Result<()> {
    let re = Regex::new("'.*test-lintrunner-config.*toml'").unwrap();
    let output = cmd.output()?;

    let output_string = format!(
        "STDOUT:\n{}\n\nSTDERR:\n{}",
        std::str::from_utf8(&output.stdout)?,
        std::str::from_utf8(&output.stderr)?,
    );
    let output_lines = output_string.lines().collect::<Vec<_>>();

    assert_yaml_snapshot!(
        name,
        output_lines,
        // Define a dynamic redaction on all lines. This will replace the config
        // name (which is a tempfile that changes from run to run) with a fixed value.
        // Everything else is passed through normally.
        {
            "[]" => insta::dynamic_redaction(move |value, _path|
                {
                    re.replace(value.as_str().unwrap(), "<temp-config>").to_string()
                }
            ),
        }
    );
    Ok(())
}

fn temp_config(contents: &str) -> Result<tempfile::NamedTempFile> {
    let mut config = tempfile::Builder::new()
        .prefix("test-lintrunner-config")
        .suffix(".toml")
        .tempfile()?;
    config.write_all(contents.as_bytes())?;
    Ok(config)
}

fn temp_config_returning_msg(lint_message: LintMessage) -> Result<tempfile::NamedTempFile> {
    let serialized = serde_json::to_string(&lint_message)?;
    let config = temp_config(&format!(
        "\
            [[linter]]
            code = 'TESTLINTER'
            include_patterns = ['**']
            command = ['echo', '{}']
        ",
        serialized
    ))?;

    Ok(config)
}

#[test]
fn unknown_config_fails() -> Result<()> {
    let mut cmd = Command::cargo_bin("lintrunner")?;
    cmd.arg("--config=asdfasdfasdf");
    cmd.assert().failure();
    assert_output_snapshot("unknown_config_fails", &mut cmd)?;

    Ok(())
}

#[test]
fn invalid_config_fails() -> Result<()> {
    let config = temp_config("asdf = 'lol'\n")?;

    let mut cmd = Command::cargo_bin("lintrunner")?;
    cmd.arg(format!("--config={}", config.path().to_str().unwrap()));
    cmd.assert().failure();
    assert_output_snapshot("invalid_config_fails", &mut cmd)?;

    Ok(())
}

#[test]
fn no_op_config_succeeds() -> Result<()> {
    let config = temp_config(
        "\
            [[linter]]
            code = 'TESTLINTER'
            include_patterns = []
            command = ['echo', 'foo']
        ",
    )?;

    let mut cmd = Command::cargo_bin("lintrunner")?;
    cmd.arg(format!("--config={}", config.path().to_str().unwrap()));
    cmd.assert().success();
    assert_output_snapshot("no_op_config_succeeds", &mut cmd)?;

    Ok(())
}

#[test]
fn empty_command_fails() -> Result<()> {
    let config = temp_config(
        "\
            [[linter]]
            code = 'TESTLINTER'
            include_patterns = ['**']
            command = []
        ",
    )?;

    let mut cmd = Command::cargo_bin("lintrunner")?;
    cmd.arg(format!("--config={}", config.path().to_str().unwrap()));
    cmd.assert().failure();
    assert_output_snapshot("empty_command_fails", &mut cmd)?;

    Ok(())
}

#[test]
fn simple_linter() -> Result<()> {
    let lint_message = LintMessage {
        path: Some("tests/fixtures/fake_source_file.rs".to_string()),
        line: Some(9),
        char: Some(1),
        code: "DUMMY".to_string(),
        name: "dummy failure".to_string(),
        severity: LintSeverity::Advice,
        original: None,
        replacement: None,
        description: Some("A dummy linter failure".to_string()),
    };
    let config = temp_config_returning_msg(lint_message)?;

    let mut cmd = Command::cargo_bin("lintrunner")?;
    cmd.arg(format!("--config={}", config.path().to_str().unwrap()));
    // Run on a file to ensure that the linter is run.
    cmd.arg("README.md");
    cmd.assert().failure();
    assert_output_snapshot("simple_linter", &mut cmd)?;

    Ok(())
}

#[test]
fn simple_linter_fails_on_nonexistent_file() -> Result<()> {
    let config = temp_config(
        "\
            [[linter]]
            code = 'TESTLINTER'
            include_patterns = ['**']
            command = ['wont_be_run']
        ",
    )?;

    let mut cmd = Command::cargo_bin("lintrunner")?;
    cmd.arg(format!("--config={}", config.path().to_str().unwrap()));
    cmd.arg("blahblahblah");
    cmd.assert().failure();
    assert_output_snapshot("simple_linter_fails_on_nonexistent_file", &mut cmd)?;

    Ok(())
}

#[test]
fn linter_providing_nonexistent_path_degrades_gracefully() -> Result<()> {
    let lint_message = LintMessage {
        path: Some("i_dont_exist_wow".to_string()),
        line: Some(3),
        char: Some(1),
        code: "DUMMY".to_string(),
        name: "dummy failure".to_string(),
        severity: LintSeverity::Advice,
        original: None,
        replacement: None,
        description: Some("A dummy linter failure".to_string()),
    };
    let config = temp_config_returning_msg(lint_message)?;

    let mut cmd = Command::cargo_bin("lintrunner")?;
    cmd.arg(format!("--config={}", config.path().to_str().unwrap()));

    // Run the linter on this file.
    cmd.arg("tests/integration_test.rs");
    cmd.assert().failure();
    assert_output_snapshot(
        "linter_providing_nonexistent_path_degrades_gracefully",
        &mut cmd,
    )?;

    Ok(())
}

#[test]
fn linter_hard_failure_is_caught() -> Result<()> {
    let config = temp_config(
        "\
            [[linter]]
            code = 'TESTLINTER'
            include_patterns = ['**']
            command = ['false']
        ",
    )?;
    let mut cmd = Command::cargo_bin("lintrunner")?;
    cmd.arg(format!("--config={}", config.path().to_str().unwrap()));

    // Run the linter on this file.
    cmd.arg("tests/integration_test.rs");
    cmd.assert().failure();
    assert_output_snapshot("linter_hard_failure_is_caught", &mut cmd)?;

    Ok(())
}

#[test]
fn linter_nonexistent_command() -> Result<()> {
    let config = temp_config(
        "\
            [[linter]]
            code = 'TESTLINTER'
            include_patterns = ['**']
            command = ['idonotexist']
        ",
    )?;
    let mut cmd = Command::cargo_bin("lintrunner")?;
    cmd.arg(format!("--config={}", config.path().to_str().unwrap()));

    // Run the linter on this file.
    cmd.arg("tests/integration_test.rs");
    cmd.assert().failure();
    assert_output_snapshot("linter_nonexistent_command", &mut cmd)?;

    Ok(())
}

#[test]
fn simple_linter_replacement_message() -> Result<()> {
    let lint_message = LintMessage {
        path: Some("tests/fixtures/fake_source_file.rs".to_string()),
        line: Some(9),
        char: Some(1),
        code: "DUMMY".to_string(),
        name: "dummy failure".to_string(),
        severity: LintSeverity::Advice,
        original: Some(
            "\
            foo\n\
            bar\n\
            baz\n\
            foo\n\
            bar\n\
            baz\n\
            foo\n\
            bar\n\
            baz\n\
            foo\n\
            bar\n\
            baz\n\
        "
            .to_string(),
        ),
        replacement: Some(
            "\
            foo\n\
            bar\n\
            bat\n\
            foo\n\
            bar\n\
            bat\n\
            foo\n\
            bar\n\
            bat\n\
            foo\n\
            bar\n\
            bat\n\
        "
            .to_string(),
        ),
        description: Some("A dummy linter failure".to_string()),
    };
    let config = temp_config_returning_msg(lint_message)?;

    let mut cmd = Command::cargo_bin("lintrunner")?;
    cmd.arg(format!("--config={}", config.path().to_str().unwrap()));
    // Run on a file to ensure that the linter is run.
    cmd.arg("README.md");
    cmd.assert().failure();
    assert_output_snapshot("simple_linter_replacement_message", &mut cmd)?;

    Ok(())
}

#[test]
fn take_nonexistent_linter() -> Result<()> {
    let config = temp_config(
        "\
            [[linter]]
            code = 'TESTLINTER'
            include_patterns = ['**']
            command = ['wont_be_run']
        ",
    )?;

    let mut cmd = Command::cargo_bin("lintrunner")?;
    cmd.arg(format!("--config={}", config.path().to_str().unwrap()));
    cmd.arg("--take=MENOEXIST");

    cmd.assert().failure();
    assert_output_snapshot("take_nonexistent_linter", &mut cmd)?;

    Ok(())
}

#[test]
fn skip_nonexistent_linter() -> Result<()> {
    let config = temp_config(
        "\
            [[linter]]
            code = 'TESTLINTER'
            include_patterns = ['**']
            command = ['wont_be_run']
        ",
    )?;

    let mut cmd = Command::cargo_bin("lintrunner")?;
    cmd.arg(format!("--config={}", config.path().to_str().unwrap()));
    cmd.arg("--skip=MENOEXIST");

    cmd.assert().failure();
    assert_output_snapshot("skip_nonexistent_linter", &mut cmd)?;

    Ok(())
}

#[test]
fn invalid_paths_cmd_and_from() -> Result<()> {
    let config = temp_config(
        "\
            [[linter]]
            code = 'TESTLINTER'
            include_patterns = []
            command = ['wont_be_run']
        ",
    )?;

    let mut cmd = Command::cargo_bin("lintrunner")?;
    cmd.arg(format!("--config={}", config.path().to_str().unwrap()));
    cmd.args(["--paths-cmd", "echo foo", "--paths-from", "foo"]);
    cmd.assert().failure();
    assert_output_snapshot("invalid_paths_cmd_and_from", &mut cmd)?;

    Ok(())
}

#[test]
fn invalid_paths_cmd_and_specified_paths() -> Result<()> {
    let config = temp_config(
        "\
            [[linter]]
            code = 'TESTLINTER'
            include_patterns = []
            command = ['wont_be_run']
        ",
    )?;

    let mut cmd = Command::cargo_bin("lintrunner")?;
    cmd.arg(format!("--config={}", config.path().to_str().unwrap()));
    cmd.args(["--paths-cmd", "echo foo", "bar", "foo"]);
    cmd.assert().failure();
    assert_output_snapshot("invalid_paths_cmd_and_specified_paths", &mut cmd)?;

    Ok(())
}

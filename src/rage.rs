use crate::persistent_data::{PersistentDataStore, RunInfo};
use anyhow::{Context, Result};
use console::style;
use dialoguer::{theme::ColorfulTheme, Select};
use std::io::Write;
use std::process::Command;
use std::process::Stdio;

fn select_past_runs(persistent_data_store: &PersistentDataStore) -> Result<Option<RunInfo>> {
    let runs = persistent_data_store.past_runs()?;
    if runs.is_empty() {
        return Ok(None);
    }
    let items: Vec<String> = runs
        .iter()
        .map(|(run_info, exit_info)| {
            let starting_glyph = if exit_info.code == 0 {
                style("✓").green()
            } else {
                style("✕").red()
            };
            format!(
                "{} {}: {}",
                starting_glyph,
                run_info.timestamp,
                run_info.args.join(" "),
            )
        })
        .collect();

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Select a past invocation to report")
        .items(&items)
        .default(0)
        .interact_opt()?;

    Ok(selection.map(|i| runs.into_iter().nth(i).unwrap().0))
}

fn upload(report: String, cmd: &mut Command) -> Result<()> {
    let mut child = cmd.stdin(Stdio::piped()).spawn()?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(report.as_bytes())?;
    }

    child.wait()?;
    Ok(())
}

pub fn do_rage(
    persistent_data_store: &PersistentDataStore,
    invocation: Option<usize>,
    gist: bool,
    pastry: bool,
) -> Result<i32> {
    let run = match invocation {
        Some(invocation) => Some(persistent_data_store.past_run(invocation)?),
        None => select_past_runs(persistent_data_store)?,
    };

    match run {
        Some(run) => {
            let report = persistent_data_store
                .get_run_report(&run)
                .context("getting selected run report")?;
            if gist {
                upload(
                    report.clone(),
                    Command::new("gh").args(["gist", "create", "-"]),
                )?;
            } else if pastry {
                upload(report.clone(), &mut Command::new("pastry"))?;
            } else {
                print!("{}", report);
            }
        }
        None => {
            println!("{}", style("Nothing selected, exiting.").yellow());
        }
    }
    Ok(0)
}

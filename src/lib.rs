use anyhow::{bail, Context, Result};
use clap::ArgEnum;
use console::{style, Term};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use linter::Linter;
use log::debug;
use path::AbsPath;
use persistent_data::PersistentDataStore;
use render::{render_lint_messages, render_lint_messages_json};
use std::collections::HashMap;
use std::collections::HashSet;
use std::convert::TryFrom;
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use version_control::VersionControl;

const MAX_VISIBLE_LINTERS: usize = 8;

pub mod git;
pub mod init;
pub mod lint_config;
pub mod lint_message;
pub mod linter;
pub mod log_utils;
pub mod path;
pub mod persistent_data;
pub mod rage;
pub mod render;
pub mod sapling;
pub mod version_control;

#[cfg(test)]
pub mod testing;

use git::get_paths_from_cmd;
use lint_message::LintMessage;
use render::PrintedLintErrors;

use crate::render::render_lint_messages_oneline;

fn group_lints_by_file(
    all_lints: &mut HashMap<Option<String>, Vec<LintMessage>>,
    lints: Vec<LintMessage>,
) {
    lints.into_iter().fold(all_lints, |acc, lint| {
        acc.entry(lint.path.clone()).or_default().push(lint);
        acc
    });
}

fn merge_lints_by_file(
    all_lints: &mut HashMap<Option<String>, Vec<LintMessage>>,
    lints_by_file: HashMap<Option<String>, Vec<LintMessage>>,
) {
    for (path, mut lints) in lints_by_file {
        all_lints.entry(path).or_default().append(&mut lints);
    }
}

fn apply_patches(lint_messages: &[LintMessage]) -> Result<()> {
    let mut patched_paths = HashSet::new();
    for lint_message in lint_messages {
        if let (Some(replacement), Some(path)) = (&lint_message.replacement, &lint_message.path) {
            let path = AbsPath::try_from(path)?;
            if patched_paths.contains(&path) {
                bail!(
                    "Two different linters proposed changes for the same file:
                    {}.\n This is not yet supported, file an issue if you want it.",
                    path.display()
                );
            }
            patched_paths.insert(path.clone());

            std::fs::write(&path, replacement).context(format!(
                "Failed to write apply patch to file: '{}'",
                path.display()
            ))?;
        }
    }
    Ok(())
}

pub fn do_init(
    linters: Vec<Linter>,
    dry_run: bool,
    persistent_data_store: &PersistentDataStore,
    config_paths: &Vec<std::string::String>,
) -> Result<i32> {
    debug!(
        "Initializing linters: {:?}",
        linters.iter().map(|l| &l.code).collect::<Vec<_>>()
    );

    for linter in linters {
        linter.init(dry_run)?;
    }
    persistent_data_store.update_last_init(config_paths)?;
    Ok(0)
}

fn remove_patchable_lints(lints: Vec<LintMessage>) -> Vec<LintMessage> {
    lints
        .into_iter()
        .filter(|lint| lint.replacement.is_none())
        .collect()
}

fn get_paths_from_input(paths: Vec<String>) -> Result<Vec<AbsPath>> {
    let mut ret = Vec::new();
    for path in &paths {
        let path = AbsPath::try_from(path)
            .with_context(|| format!("Failed to find provided file: '{}'", path))?;
        ret.push(path);
    }
    Ok(ret)
}

fn get_paths_from_file(file: AbsPath) -> Result<Vec<AbsPath>> {
    let file = std::fs::read_to_string(&file).with_context(|| {
        format!(
            "Failed to read file specified in `--paths-from`: '{}'",
            file.display()
        )
    })?;
    let files = file
        .trim()
        .lines()
        .map(|l| l.to_string())
        .collect::<Vec<String>>();
    get_paths_from_input(files)
}

/// Represents the set of paths the user wants to lint.
pub enum PathsOpt {
    /// The user didn't specify any paths, so we'll automatically determine
    /// which paths to check.
    Auto,
    AllFiles,
    PathsFile(AbsPath),
    PathsCmd(String),
    Paths(Vec<String>),
}

/// Represents the scope of revisions that the auto paths finder will look at to
/// determine which paths to lint.
pub enum RevisionOpt {
    /// Look at changes in HEAD and changes in the working tree.
    Head,
    /// Look at changes from revision..HEAD and changes in the working tree.
    Revision(String),
    /// Look at changes from merge_base(revision, HEAD)..HEAD and changes in the working tree.
    MergeBaseWith(String),
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ArgEnum)]
pub enum RenderOpt {
    Default,
    Json,
    Oneline,
}

pub fn get_version_control() -> Result<Box<dyn VersionControl>> {
    let repo = git::Repo::new();
    if let Ok(repo) = repo {
        return Ok(Box::new(repo));
    }

    Ok(Box::new(sapling::Repo::new()?))
}

struct LintProgress {
    progress: MultiProgress,
    summary: ProgressBar,
    rows: Vec<ProgressBar>,
    state: Mutex<LintProgressState>,
    total: usize,
}

#[derive(Default)]
struct LintProgressState {
    active: Vec<String>,
    completed: usize,
    failed: usize,
    streamed_lints: bool,
}

impl LintProgress {
    fn new(total: usize) -> Arc<Self> {
        let progress = MultiProgress::new();
        let summary = progress.add(ProgressBar::new(total as u64));
        summary.set_style(
            ProgressStyle::with_template("{wide_msg}")
                .expect("static progress style template should be valid"),
        );

        let mut rows = Vec::new();
        for _ in 0..MAX_VISIBLE_LINTERS.min(total) {
            let row = progress.add(ProgressBar::new_spinner());
            row.set_style(Self::blank_row_style());
            rows.push(row);
        }

        let lint_progress = Arc::new(Self {
            progress,
            summary,
            rows,
            state: Mutex::new(LintProgressState::default()),
            total,
        });
        lint_progress.render();
        lint_progress
    }

    fn active_row_style() -> ProgressStyle {
        ProgressStyle::with_template("{spinner} {wide_msg}")
            .expect("static progress style template should be valid")
    }

    fn blank_row_style() -> ProgressStyle {
        ProgressStyle::with_template("{wide_msg}")
            .expect("static progress style template should be valid")
    }

    fn start_linter(&self, code: &str) {
        {
            let mut state = self.state.lock().unwrap();
            state.active.push(code.to_string());
        }
        self.render();
    }

    fn finish_linter(&self, code: &str, is_success: bool) {
        let completed = {
            let mut state = self.state.lock().unwrap();
            state.active.retain(|active_code| active_code != code);
            state.completed += 1;
            if !is_success {
                state.failed += 1;
            }
            state.completed
        };
        self.summary.set_position(completed as u64);
        self.render();
    }

    fn stream_lints(
        &self,
        lints_by_file: &HashMap<Option<String>, Vec<LintMessage>>,
    ) -> Result<()> {
        if lints_by_file.is_empty() || self.progress.is_hidden() {
            return Ok(());
        }

        let mut output = Vec::new();
        render_lint_messages(&mut output, lints_by_file)?;

        self.progress.suspend(|| {
            let stdout = std::io::stdout();
            let mut stdout = stdout.lock();
            stdout.write_all(&output)
        })?;

        self.state.lock().unwrap().streamed_lints = true;
        Ok(())
    }

    fn streamed_lints(&self) -> bool {
        self.state.lock().unwrap().streamed_lints
    }

    fn finish(&self) {
        for row in &self.rows {
            row.finish_and_clear();
        }
        let state = self.state.lock().unwrap();
        self.summary
            .finish_with_message(self.summary_message(&state));
    }

    fn summary_message(&self, state: &LintProgressState) -> String {
        let running = state.active.len();
        let hidden = running.saturating_sub(MAX_VISIBLE_LINTERS);
        let completed = format!("{}/{} completed", state.completed, self.total);
        let completed = if state.completed == self.total && state.failed == 0 {
            format!("{}", style(completed).green())
        } else {
            completed
        };
        let failure_msg = if state.failed == 0 {
            String::new()
        } else {
            format!(
                ", {}",
                style(format!("{} failed", state.failed)).red().bold()
            )
        };

        format!(
            "Linters: {completed}, {running} running{failure_msg}{}",
            if hidden == 0 {
                String::new()
            } else {
                format!(", {} hidden", hidden)
            }
        )
    }

    fn render(&self) {
        let state = self.state.lock().unwrap();
        self.summary.set_message(self.summary_message(&state));

        for (index, row) in self.rows.iter().enumerate() {
            if let Some(code) = state.active.get(index) {
                row.set_style(Self::active_row_style());
                row.set_message(format!("{} running...", code));
                row.enable_steady_tick(Duration::from_millis(100));
                row.tick();
            } else {
                row.disable_steady_tick();
                row.set_style(Self::blank_row_style());
                row.set_message("");
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn do_lint(
    linters: Vec<Linter>,
    paths_opt: PathsOpt,
    should_apply_patches: bool,
    render_opt: RenderOpt,
    enable_spinners: bool,
    revision_opt: RevisionOpt,
    tee_json: Option<String>,
    only_lint_under_config_dir: bool,
) -> Result<i32> {
    debug!(
        "Running linters: {:?}",
        linters.iter().map(|l| &l.code).collect::<Vec<_>>()
    );
    let repo = get_version_control()?;
    let mut stdout = Term::stdout();
    if linters.is_empty() {
        stdout.write_line("No linters ran.")?;
        return Ok(0);
    }

    let config_dir = if only_lint_under_config_dir {
        Some(AbsPath::try_from(linters[0].get_config_dir())?)
    } else {
        None
    };

    let mut files = match paths_opt {
        PathsOpt::Auto => {
            let relative_to = match revision_opt {
                RevisionOpt::Head => None,
                RevisionOpt::Revision(revision) => Some(revision),
                RevisionOpt::MergeBaseWith(merge_base_with) => {
                    Some(repo.get_merge_base_with(&merge_base_with)?)
                }
            };
            debug!("Relative to: {:?}", relative_to);
            repo.get_changed_files(relative_to.as_deref())?
        }
        PathsOpt::PathsCmd(paths_cmd) => get_paths_from_cmd(&paths_cmd)?,
        PathsOpt::Paths(paths) => get_paths_from_input(paths)?,
        PathsOpt::PathsFile(file) => get_paths_from_file(file)?,
        PathsOpt::AllFiles => repo.get_all_files(config_dir.as_ref())?,
    };

    // Sort and unique the files so we pass a consistent ordering to linters
    if let Some(config_dir) = config_dir {
        files.retain(|path| path.starts_with(&config_dir));
    }
    files.sort();
    files.dedup();

    let files = Arc::new(files);

    log_utils::log_files("Linting files: ", &files);

    let total_linters = linters.len();
    let mut thread_handles = Vec::new();
    let progress = if enable_spinners {
        Some(LintProgress::new(total_linters))
    } else {
        None
    };

    // Too lazy to learn rust's fancy concurrent programming stuff, just spawn a thread per linter and join them.
    let all_lints = Arc::new(Mutex::new(HashMap::new()));

    for linter in linters {
        let all_lints = Arc::clone(&all_lints);
        let files = Arc::clone(&files);
        let progress = progress.as_ref().map(Arc::clone);

        let handle = thread::spawn(move || -> Result<()> {
            if let Some(progress) = &progress {
                progress.start_linter(&linter.code);
            }

            let lints = linter.run(&files);

            // If we're applying patches later, don't consider lints that would
            // be fixed by that.
            let lints = if should_apply_patches {
                if let Err(err) = apply_patches(&lints) {
                    if let Some(progress) = &progress {
                        progress.finish_linter(&linter.code, false);
                    }
                    return Err(err);
                }
                remove_patchable_lints(lints)
            } else {
                lints
            };

            let is_success = lints.is_empty();
            let mut lints_by_file = HashMap::new();
            group_lints_by_file(&mut lints_by_file, lints);

            if let Some(progress) = &progress {
                progress.finish_linter(&linter.code, is_success);
                progress.stream_lints(&lints_by_file)?;
            }

            let mut all_lints = all_lints.lock().unwrap();
            merge_lints_by_file(&mut all_lints, lints_by_file);
            Ok(())
        });
        thread_handles.push(handle);
    }

    for handle in thread_handles {
        if let Err(err) = handle.join().unwrap() {
            if let Some(progress) = &progress {
                progress.finish();
            }
            return Err(err);
        }
    }

    if let Some(progress) = &progress {
        progress.finish();
    }

    // Unwrap is fine because all other owners hsould have been joined.
    let all_lints = all_lints.lock().unwrap();

    // Flush the logger before rendering results.
    log::logger().flush();

    let did_print = match render_opt {
        RenderOpt::Default
            if progress
                .as_ref()
                .is_some_and(|progress| progress.streamed_lints()) =>
        {
            PrintedLintErrors::Yes
        }
        RenderOpt::Default => render_lint_messages(&mut stdout, &all_lints)?,
        RenderOpt::Json => render_lint_messages_json(&mut stdout, &all_lints)?,
        RenderOpt::Oneline => render_lint_messages_oneline(&mut stdout, &all_lints)?,
    };

    if let Some(tee_json) = tee_json {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(tee_json)
            .context("Couldn't open file for --tee-json")?;
        render_lint_messages_json(&mut file, &all_lints)?;
    }

    if should_apply_patches {
        stdout.write_line("Successfully applied all patches.")?;
    }

    match did_print {
        PrintedLintErrors::No => Ok(0),
        PrintedLintErrors::Yes => Ok(1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{convert::TryFrom, io::Write};
    use tempfile::NamedTempFile;

    #[test]
    fn test_paths_file() -> Result<()> {
        let file1 = NamedTempFile::new()?;
        let file2 = NamedTempFile::new()?;

        let mut paths_file = NamedTempFile::new()?;

        writeln!(paths_file, "{}", file1.path().display())?;
        writeln!(paths_file, "{}", file2.path().display())?;

        let paths_file = AbsPath::try_from(paths_file.path())?;
        let paths = get_paths_from_file(paths_file)?;

        let file1_abspath = AbsPath::try_from(file1.path())?;
        let file2_abspath = AbsPath::try_from(file2.path())?;

        assert!(paths.contains(&file1_abspath));
        assert!(paths.contains(&file2_abspath));

        Ok(())
    }
}

use std::collections::HashMap;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use console::style;
use crossterm::{
    cursor::{Hide, MoveTo, Show},
    execute,
    terminal::{Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};

/// Manages the terminal alternate screen for progress display
pub struct TerminalManager {
    active_linters: Arc<Mutex<HashMap<String, LinterStatus>>>,
    in_alternate_screen: bool,
}

#[derive(Clone)]
pub struct LinterStatus {
    pub message: String,
    pub completed: bool,
    pub success: bool,
}

impl TerminalManager {
    pub fn new() -> Self {
        Self {
            active_linters: Arc::new(Mutex::new(HashMap::new())),
            in_alternate_screen: false,
        }
    }

    /// Enter alternate screen buffer and start progress display
    pub fn enter_progress_mode(&mut self) -> Result<()> {
        if !self.in_alternate_screen {
            execute!(io::stdout(), EnterAlternateScreen, Hide)?;
            self.in_alternate_screen = true;
        }
        Ok(())
    }

    /// Exit alternate screen buffer and return to normal terminal
    pub fn exit_progress_mode(&mut self) -> Result<()> {
        if self.in_alternate_screen {
            execute!(io::stdout(), Show, LeaveAlternateScreen)?;
            self.in_alternate_screen = false;
        }
        Ok(())
    }

    /// Add a new linter to track
    pub fn add_linter(&self, code: String, message: String) {
        let mut linters = self.active_linters.lock().unwrap();
        linters.insert(
            code,
            LinterStatus {
                message,
                completed: false,
                success: false,
            },
        );
        drop(linters);
        self.refresh_display().ok();
    }

    /// Update a linter's status
    pub fn update_linter(&self, code: &str, message: String, completed: bool, success: bool) {
        let mut linters = self.active_linters.lock().unwrap();
        if let Some(status) = linters.get_mut(code) {
            status.message = message;
            status.completed = completed;
            status.success = success;
        }
    }

    /// Update a linter's status (non-blocking version)
    pub fn update_linter_nonblocking(
        &self,
        code: &str,
        message: String,
        completed: bool,
        success: bool,
    ) {
        if let Ok(mut linters) = self.active_linters.try_lock() {
            if let Some(status) = linters.get_mut(code) {
                status.message = message;
                status.completed = completed;
                status.success = success;
            }
        }
    }

    /// Refresh the progress display
    pub fn refresh_display(&self) -> Result<()> {
        if !self.in_alternate_screen {
            return Ok(());
        }

        let all_linters = self.active_linters.lock().unwrap();

        // Calculate counts
        let total_count = all_linters.len();
        let completed_count = all_linters.values().filter(|s| s.completed).count();
        let success_count = all_linters
            .values()
            .filter(|s| s.completed && s.success)
            .count();
        let failed_count = all_linters
            .values()
            .filter(|s| s.completed && !s.success)
            .count();
        let running_count = total_count - completed_count;

        // Get terminal height for truncation
        let terminal_height = crossterm::terminal::size()
            .map(|(_, h)| h as usize)
            .unwrap_or(24);

        // Calculate how much space we need for header (4 lines) and potential truncation message (1 line)
        let header_lines = 4;
        let truncation_reserve = 1;
        let available_lines = if terminal_height > header_lines + truncation_reserve + 2 {
            terminal_height - header_lines - truncation_reserve
        } else {
            terminal_height.saturating_sub(header_lines)
        };

        // Filter linters to display (hide successful completed ones)
        let display_linters: Vec<(String, LinterStatus)> = all_linters
            .iter()
            .filter(|(_, status)| !status.completed || !status.success)
            .map(|(code, status)| (code.clone(), status.clone()))
            .collect();

        drop(all_linters);

        // Clear screen and move to top
        execute!(io::stdout(), Clear(ClearType::All), MoveTo(0, 0))?;

        if total_count == 0 {
            println!("{}", style("No linters to run").dim());
        } else if completed_count == total_count && success_count == total_count {
            println!("{}", style("All linters completed successfully!").green());
        } else {
            // Header with progress summary
            println!("{}", style("Running linters...").bold());

            let progress_parts = vec![
                if running_count > 0 {
                    Some(format!("{} running", style(running_count).yellow()))
                } else {
                    None
                },
                if success_count > 0 {
                    Some(format!("{} done", style(success_count).green()))
                } else {
                    None
                },
                if failed_count > 0 {
                    Some(format!("{} failed", style(failed_count).red()))
                } else {
                    None
                },
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();

            if !progress_parts.is_empty() {
                println!("({} of {})", progress_parts.join(", "), total_count);
            } else {
                println!("(0 of {})", total_count);
            }
            println!();

            // Sort linters by code for consistent display
            let mut sorted_linters = display_linters;
            sorted_linters.sort_by(|a, b| a.0.cmp(&b.0));

            // Determine if we need to truncate
            let (linters_to_show, truncated_count) = if sorted_linters.len() <= available_lines {
                (sorted_linters, 0)
            } else {
                let truncated = sorted_linters.len() - available_lines;
                (
                    sorted_linters.into_iter().take(available_lines).collect(),
                    truncated,
                )
            };

            // Display visible linters
            for (code, status) in linters_to_show {
                let status_symbol = if status.completed {
                    if status.success {
                        style("✓").green()
                    } else {
                        style("✗").red()
                    }
                } else {
                    style("●").yellow()
                };

                let linter_name = style(&code).bold();
                let message = if status.completed && !status.success {
                    style(&status.message).red()
                } else if status.completed && status.success {
                    style(&status.message).green()
                } else {
                    style(&status.message).dim()
                };

                println!("  {} {} {}", status_symbol, linter_name, message);
            }

            // Show truncation message if needed
            if truncated_count > 0 {
                println!();
                println!(
                    "{} {} more linter{} running...",
                    style("...").dim(),
                    style(truncated_count).bold(),
                    if truncated_count == 1 { "" } else { "s" }
                );
            }
        }

        io::stdout().flush()?;
        Ok(())
    }

    /// Get a handle for updating this linter's status
    pub fn get_linter_handle(&self, code: String) -> LinterHandle {
        LinterHandle {
            code,
            manager: Arc::downgrade(&self.active_linters),
        }
    }
}

impl Drop for TerminalManager {
    fn drop(&mut self) {
        // Ensure we exit alternate screen if we're still in it
        if self.in_alternate_screen {
            let _ = self.exit_progress_mode();
        }
    }
}

/// Handle for updating individual linter status
pub struct LinterHandle {
    code: String,
    manager: std::sync::Weak<Mutex<HashMap<String, LinterStatus>>>,
}

impl LinterHandle {
    pub fn update(&self, message: String, completed: bool, success: bool) {
        if let Some(manager) = self.manager.upgrade() {
            let mut linters = manager.lock().unwrap();
            if let Some(status) = linters.get_mut(&self.code) {
                status.message = message;
                status.completed = completed;
                status.success = success;
            }
        }
    }
}

use anyhow;

use crate::path::AbsPath;

// Trait describing the operations we need in lintrunner for a version
// control system.
pub trait VersionControl {
    // Creates a new instance, trying the different implementations we
    // have available.
    fn new() -> anyhow::Result<Self>
    where
        Self: Sized;

    // Gets the tip of the repository.
    fn get_head(&self) -> anyhow::Result<String>;

    // Gets the most recent common ancestor between the tip and the
    // given commit.
    fn get_merge_base_with(&self, merge_base_with: &str) -> anyhow::Result<String>;

    // Gets the files that have changed relative to the given commit.
    fn get_changed_files(&self, relative_to: Option<&str>) -> anyhow::Result<Vec<AbsPath>>;

    // Get all files in the repo.
    fn get_all_files(&self, under: Option<&AbsPath>) -> anyhow::Result<Vec<AbsPath>>;
}

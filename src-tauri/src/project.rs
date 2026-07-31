//! What a project remembers between sessions.
//!
//! A project is a folder, and until now that was the whole of it: a tree in the
//! panel and a `patterns.scree` beside the files. But some of what you set while
//! working belongs to the piece rather than to the app — the tempo it is played
//! at, how loud it sits, what it is called — and all of it was lost the moment
//! the window closed.
//!
//! This is the file that keeps it: a `scree-project.json` in the project folder,
//! read when the project opens and written as any of it changes. JSON rather
//! than scree, because none of it is a program: the project's other file,
//! `patterns.scree`, is folded into every run, and settings must never be.
//!
//! Nothing here is required. A folder with no such file is still a project —
//! that is every folder until it is opened once — and reading one says so
//! rather than failing, so the transport is left exactly where the performer
//! put it. A file that is *there* but unreadable is a different matter, and is
//! reported: the next thing the editor would do is write over it.

use crate::diagnostic::{Diagnostic, Stage};
use crate::engine::DEFAULT_MASTER_VOLUME;
use crate::scheduler::clock::{bpm_from_cps, DEFAULT_CPS};

use std::path::{Path, PathBuf};

/// What the settings file is called, in every project.
pub const FILE: &str = "scree-project.json";

/// The tempo a project plays at when its file does not say — the clock's own
/// default, so a project that predates this file sounds as it always did.
fn default_bpm() -> f64 {
    bpm_from_cps(DEFAULT_CPS)
}

/// The same for the master, which defaults to unity: a program is heard as
/// written until somebody turns it down.
fn default_volume() -> f64 {
    DEFAULT_MASTER_VOLUME as f64
}

/// The name a folder's project has when nothing else names it: the folder's.
fn folder_name(root: &Path) -> String {
    root.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("project")
        .to_string()
}

/// What a project remembers.
///
/// Every field has a default, so a hand-written file may say only the one thing
/// it means to change and the rest is what the engine would have done anyway.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq)]
pub struct Project {
    /// What the project is called, which is the editor's to show and nothing
    /// else's. It starts as the folder's name and is free to diverge: a folder
    /// is named for where it sits on a disk, a piece for what it is.
    #[serde(default)]
    pub name: String,
    /// Beats per minute, as the transport reads it.
    #[serde(default = "default_bpm")]
    pub bpm: f64,
    /// Master output, as a linear amplitude between silence and unity.
    #[serde(default = "default_volume")]
    pub volume: f64,
}

impl Project {
    /// A project for a folder that has none saved, taking the transport as it
    /// stands. Opening a folder must not move a fader, so what is playing now
    /// *is* the answer until the file says otherwise.
    pub fn new(root: &Path, bpm: f64, volume: f64) -> Project {
        Project { name: folder_name(root), bpm, volume }.sanitized(root)
    }

    /// Make a file safe to hand the engine.
    ///
    /// This one is edited by hand — it is JSON in the project the tree is
    /// showing — so a zero tempo or a volume of 40 is a thing that can arrive
    /// here. None of it is worth refusing a project over: a nonsense number is
    /// replaced by the default, a volume outside the fader's travel is clamped
    /// to it, and a blank name falls back to the folder's.
    fn sanitized(mut self, root: &Path) -> Project {
        self.name = self.name.trim().to_string();
        if self.name.is_empty() {
            self.name = folder_name(root);
        }
        if !self.bpm.is_finite() || self.bpm <= 0.0 {
            self.bpm = default_bpm();
        }
        self.volume = if self.volume.is_finite() {
            self.volume.clamp(0.0, 1.0)
        } else {
            default_volume()
        };
        self
    }
}

/// Where a project's settings live.
pub fn file_path(root: &Path) -> PathBuf {
    root.join(FILE)
}

/// Read a project's settings.
///
/// `None` is a folder that has never been saved as a project, which is not a
/// failure — it is every folder the first time it is opened, and every project
/// made before this file existed. An unparseable file is a failure, and is
/// named: the editor is about to start writing this file, and it must not do
/// that over something it could not read.
pub fn read(root: &Path) -> Result<Option<Project>, Diagnostic> {
    let path = file_path(root);
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            let project: Project = serde_json::from_str(&text).map_err(|e| {
                Diagnostic::message(
                    Stage::Import,
                    format!("could not read {}: {e}", path.display()),
                )
            })?;
            Ok(Some(project.sanitized(root)))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Diagnostic::message(
            Stage::Import,
            format!("could not read {}: {e}", path.display()),
        )),
    }
}

/// Write a project's settings out, and say where they went.
///
/// Sanitised on the way, for the same reason as on the way in: what lands on
/// disk is a file this app can open again, whatever the editor was holding.
pub fn write(root: &Path, project: &Project) -> Result<PathBuf, Diagnostic> {
    let path = file_path(root);
    let project = project.clone().sanitized(root);
    // Pretty, and with the newline every other tool expects at the end of a
    // text file: this sits in the project tree and is meant to be opened.
    let mut text = serde_json::to_string_pretty(&project).map_err(|e| {
        Diagnostic::message(Stage::Import, format!("could not write {}: {e}", path.display()))
    })?;
    text.push('\n');

    std::fs::write(&path, text).map_err(|e| {
        Diagnostic::message(Stage::Import, format!("could not save {}: {e}", path.display()))
    })?;

    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_project(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "scree-settings-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).expect("should create a temp project");
        dir
    }

    /// The whole point of the file: set a tempo, close, come back to it.
    #[test]
    fn settings_survive_a_project_being_reopened() {
        let root = temp_project("roundtrip");
        let saved = Project { name: "Nocturne".into(), bpm: 96.0, volume: 0.7 };

        let path = write(&root, &saved).expect("should write");
        assert_eq!(path, file_path(&root));

        let read = read(&root).expect("should read").expect("should be a project");
        assert_eq!(read, saved);

        std::fs::remove_dir_all(&root).ok();
    }

    /// A folder nobody has saved settings for is not a failure — it is every
    /// folder until it is opened once.
    #[test]
    fn a_folder_with_no_file_has_no_settings() {
        let root = temp_project("none");
        assert_eq!(read(&root).expect("a missing file is not an error"), None);
        std::fs::remove_dir_all(&root).ok();
    }

    /// One that does not parse is, because writing over it is what happens next.
    #[test]
    fn an_unreadable_file_is_reported_with_its_path() {
        let root = temp_project("broken");
        std::fs::write(file_path(&root), "{ not json at all").expect("should write");

        let err = read(&root).expect_err("should not read");
        assert!(
            err.message.contains(FILE),
            "the message should name the file, got {err}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// A file may say only what it means to change.
    #[test]
    fn missing_fields_fall_back_to_the_defaults() {
        let root = temp_project("partial");
        std::fs::write(file_path(&root), r#"{ "bpm": 140 }"#).expect("should write");

        let read = read(&root).expect("should read").expect("should be a project");
        assert_eq!(read.bpm, 140.0);
        assert_eq!(read.volume, default_volume());
        // Unnamed is named for its folder rather than blank.
        assert_eq!(read.name, folder_name(&root));

        std::fs::remove_dir_all(&root).ok();
    }

    /// Hand-edited nonsense is corrected rather than refused: none of it is
    /// worth failing to open a project over.
    #[test]
    fn unusable_values_are_replaced_or_clamped() {
        let root = temp_project("nonsense");
        std::fs::write(
            file_path(&root),
            r#"{ "name": "   ", "bpm": 0, "volume": 40 }"#,
        )
        .expect("should write");

        let read = read(&root).expect("should read").expect("should be a project");
        assert_eq!(read.name, folder_name(&root));
        assert_eq!(read.bpm, default_bpm());
        assert_eq!(read.volume, 1.0);

        std::fs::remove_dir_all(&root).ok();
    }

    /// And is corrected on the way out too, so what lands on disk always opens.
    #[test]
    fn what_is_written_can_always_be_read_back() {
        let root = temp_project("sanitised");
        let wild = Project { name: String::new(), bpm: f64::NAN, volume: -3.0 };
        write(&root, &wild).expect("should write");

        let read = read(&root).expect("should read").expect("should be a project");
        assert_eq!(read.name, folder_name(&root));
        assert_eq!(read.bpm, default_bpm());
        assert_eq!(read.volume, 0.0);

        std::fs::remove_dir_all(&root).ok();
    }

    /// A folder with no settings takes the transport as it stands: picking a
    /// project must not move a fader.
    #[test]
    fn a_new_project_takes_the_transport_as_it_is() {
        let root = temp_project("fresh");
        let project = Project::new(&root, 132.0, 0.5);
        assert_eq!(project.name, folder_name(&root));
        assert_eq!(project.bpm, 132.0);
        assert_eq!(project.volume, 0.5);
        std::fs::remove_dir_all(&root).ok();
    }
}

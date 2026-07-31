// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/

use crate::engine::{stop as stop_graph, swap_program};
use crate::{
    scree_graph::realizer::realize,
    lowerer::lower::lower_with_samples,
    parser::parser::parse,
};
use crate::diagnostic::{Diagnostic, Stage};
use crate::engine::AudioEngine;
use crate::imports::Workspace;
use crate::pattern::graphical::{self, GraphicalPattern};
use crate::pattern::patterns::Patterns;
use crate::scheduler::clock::{bpm_from_cps, cps_from_bpm};
use crate::scheduler::scheduler::SchedulerState;
use crate::scheduler::voice::Instruments;

use std::sync::Mutex;
use tauri::menu::{IsMenuItem, Menu, MenuItem, MenuItemKind, PredefinedMenuItem, Submenu};
use tauri::{Emitter, Manager};

mod pattern;
mod scheduler;
mod diagnostic;
mod engine;
mod imports;
mod parser;
mod scree_graph;
mod lowerer;
mod lang;
mod samples;

/// Backend hook for the editor's "play" button.
///
/// Produces two artifacts from one program: the persistent graph, which is
/// crossfaded into the engine's slot, and the pattern bindings, which go to
/// the scheduler along with the instrument definitions it builds voices from.
///
/// `patterns` are the ones drawn in the side panel. They arrive with the code
/// rather than being held here, because they are part of what is being
/// evaluated: the editor is the only thing that knows them, and an eval is the
/// only moment they matter.
///
/// They reach the program as the project's `patterns.scree`, laid over
/// whatever is on disk: the panel writes that file as you draw, and this makes
/// an eval independent of whether the write has landed — or is even possible,
/// in a folder that cannot be written to. `imports::expand` folds the file into
/// the program, which is the whole of how a drawn pattern gets a name.
///
/// `None` is the panel saying it has nothing to add — it has not read this
/// project yet, or could not — as against `Some([])`, which is a panel that
/// has read the project and found no patterns in it. Only the second one is
/// allowed to hide a file that is on disk.
///
/// `workspace` is what the editor knows and this side cannot: where the file
/// being run lives, so a `use` in it has somewhere to look. Everything a `use`
/// names is then read from disk — a module open in another tab included, which
/// means a change to one is heard once it has been saved.
///
/// A refusal from any of the passes goes back as a `Diagnostic` — which pass,
/// what it said, and where — for the editor's problems panel. Nothing is
/// swapped into the engine until all of them have agreed, so a program that does
/// not compile leaves whatever is playing alone.
#[tauri::command]
fn run_code(
    code: String,
    patterns: Option<Vec<GraphicalPattern>>,
    workspace: Workspace,
    engine: tauri::State<Mutex<AudioEngine>>,
    sched: tauri::State<SchedulerState>,
    samples: tauri::State<samples::Cache>,
) -> Result<(), Diagnostic> {
    let mut workspace = workspace;
    if let Some(patterns) = &patterns {
        graphical::check_names(patterns)?;
        workspace.set_patterns(graphical::to_source(patterns));
    }

    // Imports are resolved into the program before anything else sees it, so
    // the lowerer, the realizer and the scheduler all compile one flat file.
    let ast = imports::expand(parse(code.clone())?, &code, &workspace)?;

    // Files before lowering, and before the scheduler is told anything. A
    // `load` names a path relative to this file, exactly as a `use` does, and
    // this is the only thread allowed to read a disk — a voice is built per
    // note on the scheduler's.
    let loaded = samples::Samples::load(&ast, workspace.dir().as_deref(), &samples)
        .map_err(|e| Diagnostic::message(Stage::Lower, e))?;

    let lowered = lower_with_samples(&ast, loaded.clone())
        .map_err(|e| Diagnostic::message(Stage::Lower, e))?;
    let audio_graph = realize(&lowered.graph)
        .map_err(|e| Diagnostic::message(Stage::Realize, e))?;

    // Instruments before patterns: the scheduler must be able to build a voice
    // for any binding it can see.
    *sched
        .instruments
        .lock()
        .map_err(|_| Diagnostic::message(Stage::Engine, "instruments lock poisoned"))? =
        Instruments::from_program(&ast).with_samples(loaded);

    let starting_from_silence = sched
        .patterns
        .lock()
        .map_err(|_| Diagnostic::message(Stage::Engine, "patterns lock poisoned"))?
        .is_empty();

    // Starting from silence should begin a pattern at its first step, but a
    // re-eval of something already playing must not jolt the groove. The clock
    // moves *before* the patterns are published, so the scheduler can never
    // see the new bindings against the old origin.
    //
    // Whatever cycle that leaves us on is the origin the bounded bindings —
    // `play_once`, `playn` — count from, read under the same lock as the reset
    // so the two can never disagree.
    let origin = {
        let eng = engine
            .lock()
            .map_err(|_| Diagnostic::message(Stage::Engine, "audio engine poisoned"))?;
        if starting_from_silence && !lowered.bindings.is_empty() {
            eng.clock.reset();
        }
        eng.clock.now_cycles()
    };

    *sched
        .patterns
        .lock()
        .map_err(|_| Diagnostic::message(Stage::Engine, "patterns lock poisoned"))? =
        Patterns { bindings: lowered.bindings, origin };

    let mut eng = engine
        .lock()
        .map_err(|_| Diagnostic::message(Stage::Engine, "audio engine poisoned"))?;
    swap_program(&mut eng, audio_graph);

    Ok(())
}

/// Backend hook for the editor's "stop" button.
///
/// Silence has three parts, because a program has three places sound can come
/// from: the persistent graph in the engine's slot, the pattern bindings the
/// scheduler keeps turning into voices, and the voices it has already pushed
/// into the lookahead window. Clearing only the first two leaves the last few
/// notes ringing out.
#[tauri::command]
fn stop_audio(
    engine: tauri::State<Mutex<AudioEngine>>,
    sched: tauri::State<SchedulerState>,
) -> Result<(), String> {
    // Bindings first, so the next pass has nothing to schedule, then the flag
    // the scheduler thread reads to cut what it already pushed.
    *sched.patterns.lock().map_err(|_| "patterns lock poisoned")? = Patterns::default();
    sched.request_stop();

    let mut eng = engine.lock().map_err(|_| "audio engine poisoned")?;
    stop_graph(&mut eng);
    // The clock keeps counting through the silence, so without this the next
    // play would rejoin the pattern a little past where it stopped.
    eng.clock.reset();

    Ok(())
}

/// The transport panel's controls, as the frontend shows them: tempo in beats
/// per minute, volume as a linear amplitude between silence and unity.
#[derive(serde::Serialize)]
struct Transport {
    bpm: f64,
    volume: f64,
}

/// What the transport is set to right now, so the panel can open showing the
/// engine's own defaults rather than a guess that drifts away from them.
#[tauri::command]
fn transport(engine: tauri::State<Mutex<AudioEngine>>) -> Result<Transport, String> {
    let eng = engine.lock().map_err(|_| "audio engine poisoned")?;
    Ok(Transport {
        bpm: bpm_from_cps(eng.clock.cps()),
        volume: eng.master.value() as f64,
    })
}

/// Set the global tempo.
///
/// The clock holds the current beat across the change, so this is safe to call
/// while something is playing — including on every frame of a drag.
#[tauri::command]
fn set_tempo(bpm: f64, engine: tauri::State<Mutex<AudioEngine>>) -> Result<(), String> {
    if !bpm.is_finite() || bpm <= 0.0 {
        return Err(format!("tempo must be above zero, got {bpm}"));
    }
    engine
        .lock()
        .map_err(|_| "audio engine poisoned")?
        .clock
        .set_cps(cps_from_bpm(bpm));
    Ok(())
}

/// Set the master output volume, as a linear amplitude.
///
/// Out-of-range values are clamped rather than refused: a control surface
/// sending 1.0000001 is not an error worth interrupting a performance for.
#[tauri::command]
fn set_master_volume(volume: f64, engine: tauri::State<Mutex<AudioEngine>>) -> Result<(), String> {
    if !volume.is_finite() {
        return Err(format!("volume must be a number, got {volume}"));
    }
    engine
        .lock()
        .map_err(|_| "audio engine poisoned")?
        .master
        .set(volume.clamp(0.0, 1.0) as f32);
    Ok(())
}

/// Write a tab's contents to disk. The frontend picks the path via the save
/// dialog; here we just persist the bytes.
#[tauri::command]
fn save_file(path: String, content: String) -> Result<(), String> {
    std::fs::write(&path, content).map_err(|e| e.to_string())
}

/// Read a file's contents from disk. The frontend picks the path via the open
/// dialog; here we just return the text.
#[tauri::command]
fn read_file(path: String) -> Result<String, String> {
    std::fs::read_to_string(&path).map_err(|e| e.to_string())
}

/// A project's drawn patterns, and the file they came from.
///
/// The path is returned rather than composed on the frontend so there is one
/// answer to where the file is, and the editor can recognise the file when it
/// is opened as an ordinary tab.
#[derive(serde::Serialize, Debug)]
struct PatternsFile {
    path: String,
    patterns: Vec<GraphicalPattern>,
}

/// Read a project's patterns into the panel.
///
/// A project with no such file has no drawn patterns, which is not a failure —
/// it is every new project. A file that does not parse *is* one: the panel must
/// not show an empty grid over a file it could not read, because the next thing
/// it would do is write that empty grid back over it.
#[tauri::command]
fn read_patterns(root: String) -> Result<PatternsFile, Diagnostic> {
    let path = std::path::Path::new(&root).join(graphical::FILE);
    let display = path.display().to_string();

    let patterns = match std::fs::read_to_string(&path) {
        Ok(code) => graphical::from_source(&code).map_err(|e| e.or_in(Some(&path)))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => {
            return Err(Diagnostic::message(
                Stage::Import,
                format!("could not read {display}: {e}"),
            ))
        }
    };

    Ok(PatternsFile { path: display, patterns })
}

/// Write a project's patterns out, and say where they went.
///
/// Called as the panel is drawn in, so the file is what the panel holds. It is
/// only persistence: an eval sends the panel's rows along with the code, so
/// music does not stop for a folder that cannot be written to — the failure
/// shows up in the problems panel instead.
#[tauri::command]
fn write_patterns(root: String, patterns: Vec<GraphicalPattern>) -> Result<String, Diagnostic> {
    graphical::check_names(&patterns)?;

    let path = std::path::Path::new(&root).join(graphical::FILE);
    std::fs::write(&path, graphical::to_source(&patterns)).map_err(|e| {
        Diagnostic::message(
            Stage::Import,
            format!("could not save {}: {e}", path.display()),
        )
    })?;

    Ok(path.display().to_string())
}

/// One entry in a project directory, as the project panel draws it.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    name: String,
    /// Absolute, so a click can open it without knowing where it came from.
    path: String,
    is_dir: bool,
}

/// What the app remembers the last project in, inside its own config folder.
const RECENT_PROJECT: &str = "recent-project";

/// Whether a remembered path is still worth opening.
///
/// A project is nothing more than a folder, so the only thing that can go
/// wrong is that it stopped being one — moved, deleted, or on a volume that is
/// not mounted this time. None of those deserve an error: the app simply opens
/// with no project, which is the same state it is in on a first run.
fn usable_project(saved: &str) -> Option<String> {
    let saved = saved.trim();
    if saved.is_empty() {
        return None;
    }
    std::path::Path::new(saved).is_dir().then(|| saved.to_string())
}

fn recent_project_file(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    app.path()
        .app_config_dir()
        .map(|dir| dir.join(RECENT_PROJECT))
        .map_err(|e| e.to_string())
}

/// The project to open on launch: whichever folder was last chosen, if it is
/// still there.
///
/// Deliberately *not* the working directory. A launched binary's cwd says
/// nothing about what someone is working on — under `tauri dev` it is the Rust
/// crate, and a bundled app opened from the desktop gets `/` — so using it
/// meant the app wrote the panel's `patterns.scree` into the source tree in
/// development, and somewhere unwritable in a real build. `None` is a real
/// answer here: the app is allowed to have no project open.
#[tauri::command]
fn recent_project(app: tauri::AppHandle) -> Result<Option<String>, String> {
    let path = recent_project_file(&app)?;
    // An unreadable file is a first run, not a failure.
    let Ok(saved) = std::fs::read_to_string(&path) else {
        return Ok(None);
    };
    Ok(usable_project(&saved))
}

/// Remember the project just opened, for the next launch.
#[tauri::command]
fn set_recent_project(app: tauri::AppHandle, root: String) -> Result<(), String> {
    let path = recent_project_file(&app)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, root).map_err(|e| e.to_string())
}

/// List one directory, for the project tree.
///
/// One level deep on purpose: the tree asks again when a folder is opened, so
/// a project with a `target/` or `node_modules/` in it costs nothing until
/// somebody actually looks inside one.
#[tauri::command]
fn list_dir(path: String) -> Result<Vec<Entry>, String> {
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(&path).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let full = entry.path();
        // Follows symlinks, so a link to a folder opens like the folder.
        let is_dir = full.is_dir();
        // A name or path that isn't text is one the editor could not open
        // either, so it is left out rather than drawn as mojibake.
        if let (Some(name), Some(path)) = (entry.file_name().to_str(), full.to_str()) {
            entries.push(Entry {
                name: name.to_string(),
                path: path.to_string(),
                is_dir,
            });
        }
    }

    // Folders first, then by name, the way a file browser reads.
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    Ok(entries)
}

/// The language's callable surface, for the editor's highlighting, completion
/// and signature help.
///
/// Served from the same tables the lowerer dispatches on, so the editor can
/// never offer a name the language does not have.
#[tauri::command]
fn language_metadata() -> lang::LanguageMetadata {
    lang::metadata()
}

/// Menu item ids, which are also the events the frontend listens for. One
/// constant each so the two can't drift apart.
const MENU_NEW: &str = "file-new";
const MENU_OPEN: &str = "file-open";
const MENU_SAVE: &str = "file-save";
const MENU_NEW_PROJECT: &str = "project-new";

/// Every id the File menu can raise. The event handler forwards these and
/// ignores anything else, so the platform's own items keep working.
const FILE_ITEMS: [&str; 4] = [MENU_NEW, MENU_OPEN, MENU_SAVE, MENU_NEW_PROJECT];

/// The platform's default menu with New, Open and Save added to the top of
/// File. Each one carries the accelerator the toolbar buttons used to advertise.
///
/// Editing the default rather than rebuilding it keeps every other item —
/// Quit, Copy, Minimise, the lot — exactly where the platform puts it.
fn build_menu(app: &tauri::AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let menu = Menu::default(app)?;

    let new_item = MenuItem::with_id(app, MENU_NEW, "New", true, Some("CmdOrCtrl+N"))?;
    let open_item = MenuItem::with_id(app, MENU_OPEN, "Open…", true, Some("CmdOrCtrl+O"))?;
    let save_item = MenuItem::with_id(app, MENU_SAVE, "Save", true, Some("CmdOrCtrl+S"))?;
    // No accelerator: picking a project is a once-a-session act, and the
    // obvious shortcuts are all spoken for by the file items above.
    let new_project_item =
        MenuItem::with_id(app, MENU_NEW_PROJECT, "New Project…", true, None::<&str>)?;

    // Three groups, because there are three kinds of act here: making or
    // fetching a file, choosing which folder the project panel shows, and
    // saving. The trailing rule keeps all of it off whatever the platform put
    // in File.
    let items: [&dyn IsMenuItem<tauri::Wry>; 7] = [
        &new_item,
        &open_item,
        &PredefinedMenuItem::separator(app)?,
        &new_project_item,
        &PredefinedMenuItem::separator(app)?,
        &save_item,
        &PredefinedMenuItem::separator(app)?,
    ];

    let file = menu.items()?.into_iter().find_map(|item| match item {
        MenuItemKind::Submenu(s) if s.text().map(|t| t == "File").unwrap_or(false) => Some(s),
        _ => None,
    });

    match file {
        Some(file) => file.insert_items(&items, 0)?,
        // No File submenu on this platform's default: make one. Nothing to sit
        // below it, so the trailing separator goes.
        None => menu.append(&Submenu::with_items(app, "File", true, &items[..6])?)?,
    }

    Ok(menu)
}

#[cfg(test)]
mod pattern_file_tests {
    use super::*;
    use crate::pattern::graphical::{GraphicalStep, StepKind};

    fn temp_project(name: &str) -> String {
        let dir = std::env::temp_dir().join(format!(
            "scree-project-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).expect("should create a temp project");
        dir.display().to_string()
    }

    fn rows() -> Vec<GraphicalPattern> {
        vec![GraphicalPattern {
            name: "hats".to_string(),
            steps: vec![
                GraphicalStep::new(StepKind::Trigger),
                GraphicalStep::new(StepKind::Rest),
                GraphicalStep::new(StepKind::Pitch { note: 60.0 }),
            ],
            mode: graphical::Mode::Roll,
        }]
    }

    /// The panel's whole round trip, through the two commands it calls: draw,
    /// save, reopen the project, and find the same rows.
    #[test]
    fn patterns_survive_a_project_being_reopened() {
        let root = temp_project("roundtrip");

        let path = write_patterns(root.clone(), rows()).expect("should write");
        assert!(path.ends_with(graphical::FILE), "wrote {path}");

        let read = read_patterns(root.clone()).expect("should read");
        assert_eq!(read.path, path);
        assert_eq!(read.patterns, rows());

        std::fs::remove_dir_all(&root).ok();
    }

    /// A project nobody has drawn in yet has no patterns, which is not a
    /// failure — it is every new project.
    #[test]
    fn a_project_with_no_file_has_no_patterns() {
        let root = temp_project("empty");
        let read = read_patterns(root.clone()).expect("a missing file is not an error");
        assert!(read.patterns.is_empty());
        std::fs::remove_dir_all(&root).ok();
    }

    /// One that does not parse is a failure, and says where: the panel needs
    /// to know, because what it does on a successful read is start writing.
    #[test]
    fn an_unreadable_file_is_reported_with_its_path() {
        let root = temp_project("broken");
        std::fs::write(
            std::path::Path::new(&root).join(graphical::FILE),
            "let hats = [\\, `\nfn oops(\n",
        )
        .expect("should write");

        let err = read_patterns(root.clone()).expect_err("should not read");
        assert_eq!(err.file.as_deref(), Some(format!("{root}/{}", graphical::FILE).as_str()));

        std::fs::remove_dir_all(&root).ok();
    }

    /// The file is written as scree, and reads as scree: this is the property
    /// that lets it be included like any other file in the project.
    #[test]
    fn the_written_file_is_a_program() {
        let root = temp_project("program");
        let path = write_patterns(root.clone(), rows()).expect("should write");
        let text = std::fs::read_to_string(&path).expect("should read back");

        let items = parse(text).expect("the panel's file must parse");
        assert!(matches!(items.first(), Some(crate::parser::parser::ScreeItem::Let { .. })));

        std::fs::remove_dir_all(&root).ok();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .menu(build_menu)
        .on_menu_event(|app, event| {
            // The work is the frontend's: it owns the tabs, and the dialogs
            // that pick a path. All this side does is relay the click.
            if let Some(id) = FILE_ITEMS.iter().find(|id| event.id() == **id) {
                let _ = app.emit(id, ());
            }
        })
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            run_code,
            stop_audio,
            transport,
            set_tempo,
            set_master_volume,
            save_file,
            read_file,
            read_patterns,
            write_patterns,
            recent_project,
            set_recent_project,
            list_dir,
            language_metadata
        ])
        .setup(|app| {
            let (engine, seq) = engine::start()?;
            let clock = engine.clock.clone();
            let sched = SchedulerState::new();

            // Free-runs for the life of the app; evals only swap what it reads.
            scheduler::scheduler::start(seq, clock, sched.clone());

            app.manage(Mutex::new(engine));
            app.manage(sched);
            // Outlives every eval: a break named by a program being edited is
            // decoded on the first run and shared by every run after it.
            app.manage(samples::Cache::default());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod recent_project_tests {
    use super::usable_project;

    /// A remembered folder that is still a folder is the one to open.
    #[test]
    fn an_existing_folder_is_usable() {
        let dir = std::env::temp_dir();
        let saved = dir.to_string_lossy().to_string();
        assert_eq!(usable_project(&saved), Some(saved.clone()));
    }

    /// Trailing whitespace is the file's, not the path's — it is written with
    /// whatever the OS left on the line.
    #[test]
    fn surrounding_whitespace_is_ignored() {
        let dir = std::env::temp_dir();
        let saved = format!("  {}\n", dir.to_string_lossy());
        assert_eq!(usable_project(&saved), Some(dir.to_string_lossy().to_string()));
    }

    /// A folder that has moved, been deleted, or is on an unmounted volume is
    /// not an error — the app just opens with no project, as on a first run.
    #[test]
    fn a_missing_folder_is_no_project() {
        assert_eq!(usable_project("/nowhere/that/exists/at/all"), None);
    }

    /// A file is not a project. Only folders are.
    #[test]
    fn a_file_is_not_a_project() {
        let file = std::env::temp_dir().join("scree-usable-project-test");
        std::fs::write(&file, "x").expect("should write");
        assert_eq!(usable_project(&file.to_string_lossy()), None);
        std::fs::remove_file(&file).ok();
    }

    #[test]
    fn an_empty_record_is_no_project() {
        assert_eq!(usable_project(""), None);
        assert_eq!(usable_project("   \n"), None);
    }
}

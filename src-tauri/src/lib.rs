// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/

use crate::engine::{stop as stop_graph, swap_program};
use crate::{scree_graph::realizer::realize, lowerer::lower::lower_with_patterns, parser::parser::parse};
use crate::engine::AudioEngine;
use crate::pattern::graphical::GraphicalPattern;
use crate::pattern::patterns::Patterns;
use crate::scheduler::clock::{bpm_from_cps, cps_from_bpm};
use crate::scheduler::scheduler::SchedulerState;
use crate::scheduler::voice::Instruments;

use std::sync::Mutex;
use tauri::menu::{IsMenuItem, Menu, MenuItem, MenuItemKind, PredefinedMenuItem, Submenu};
use tauri::{Emitter, Manager};

mod pattern;
mod scheduler;
mod engine;
mod parser;
mod scree_graph;
mod lowerer;
mod lang;

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
#[tauri::command]
fn run_code(
    code: String,
    patterns: Vec<GraphicalPattern>,
    engine: tauri::State<Mutex<AudioEngine>>,
    sched: tauri::State<SchedulerState>,
) -> Result<(), String> {
    let ast = parse(code)?;
    let lowered = lower_with_patterns(&ast, &patterns)?;
    let audio_graph = realize(&lowered.graph)?;

    // Instruments before patterns: the scheduler must be able to build a voice
    // for any binding it can see.
    *sched
        .instruments
        .lock()
        .map_err(|_| "instruments lock poisoned")? = Instruments::from_program(&ast);

    let starting_from_silence = sched
        .patterns
        .lock()
        .map_err(|_| "patterns lock poisoned")?
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
        let eng = engine.lock().map_err(|_| "audio engine poisoned")?;
        if starting_from_silence && !lowered.bindings.is_empty() {
            eng.clock.reset();
        }
        eng.clock.now_cycles()
    };

    *sched.patterns.lock().map_err(|_| "patterns lock poisoned")? =
        Patterns { bindings: lowered.bindings, origin };

    let mut eng = engine.lock().map_err(|_| "audio engine poisoned")?;
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

/// Every id the File menu can raise. The event handler forwards these and
/// ignores anything else, so the platform's own items keep working.
const FILE_ITEMS: [&str; 3] = [MENU_NEW, MENU_OPEN, MENU_SAVE];

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

    // Saving is a different kind of act from starting or fetching a file, and
    // the trailing rule keeps all three off whatever the platform put in File.
    let items: [&dyn IsMenuItem<tauri::Wry>; 5] = [
        &new_item,
        &open_item,
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
        None => menu.append(&Submenu::with_items(app, "File", true, &items[..4])?)?,
    }

    Ok(menu)
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
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

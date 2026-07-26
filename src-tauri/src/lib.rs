// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/

use crate::{brap_graph::realizer::realize, engine::swap_program, lowerer::lower::lower, parser::parser::parse};
use crate::engine::AudioEngine;
use crate::pattern::patterns::Patterns;
use crate::scheduler::scheduler::SchedulerState;
use crate::scheduler::voice::Instruments;

use std::sync::Mutex;
use tauri::Manager;

mod pattern;
mod scheduler;
mod engine;
mod parser;
mod brap_graph;
mod lowerer;

/// Backend hook for the editor's "play" button.
///
/// Produces two artifacts from one program: the persistent graph, which is
/// crossfaded into the engine's slot, and the pattern bindings, which go to
/// the scheduler along with the instrument definitions it builds voices from.
#[tauri::command]
fn run_code(
    code: String,
    engine: tauri::State<Mutex<AudioEngine>>,
    sched: tauri::State<SchedulerState>,
) -> Result<(), String> {
    let ast = parse(code)?;
    let lowered = lower(&ast)?;
    let audio_graph = realize(&lowered.graph)?;

    // Instruments before patterns: the scheduler must be able to build a voice
    // for any binding it can see.
    *sched
        .instruments
        .lock()
        .map_err(|_| "instruments lock poisoned")? = Instruments::from_program(&ast);

    *sched.patterns.lock().map_err(|_| "patterns lock poisoned")? =
        Patterns { bindings: lowered.bindings };

    let mut eng = engine.lock().map_err(|_| "audio engine poisoned")?;
    swap_program(&mut eng, audio_graph);

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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            run_code,
            save_file,
            read_file
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

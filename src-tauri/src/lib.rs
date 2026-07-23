// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/

use crate::{brap_graph::realizer::{self, realize}, engine::swap_program, lowerer::lower::lower, parser::parser::parse};
use crate::engine::AudioEngine;

use std::sync::Mutex;
use tauri::Manager;

mod engine;
mod parser;
mod brap_graph;
mod lowerer;

/// Backend hook for the editor's "play" button.
///
/// Intentionally empty for now — it just receives the editor contents so the
/// frontend has something real to call. Wire up actual execution here later.
#[tauri::command]
fn run_code(code: String, engine: tauri::State<Mutex<AudioEngine>>) -> Result<(), String> {
    let ast = parse(code)?;
    let graph_ir = lower(&ast)?;
    let audio_graph = realize(&graph_ir);

    let mut eng = engine.lock().map_err(|_| "audio engine poisoned")?;
    swap_program(&mut eng, audio_graph);

    Ok(())
}

/// Backend hook for the editor's "stop" button. Fades the audio engine back
/// to silence without tearing down the stream.
#[tauri::command]
fn stop_audio(engine: tauri::State<Mutex<AudioEngine>>) -> Result<(), String> {
    let mut eng = engine.lock().map_err(|_| "audio engine poisoned")?;
    engine::stop(&mut eng);
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
        .invoke_handler(tauri::generate_handler![run_code, stop_audio, save_file, read_file])
        .setup(|app| {
            app.manage(Mutex::new(engine::start()?));
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

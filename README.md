# brap

A Tauri + React + TypeScript desktop app with a simple code editor.

The main page hosts a CodeMirror editor and a **Play** button. Play is wired to
the Rust command `run_code` (`src-tauri/src/lib.rs`), which currently does
nothing — it's a placeholder for real execution. The keyboard shortcut for Play
is **⌘↵ (Cmd+Enter)**.

Styling is done with [Tailwind CSS v4](https://tailwindcss.com/) via its Vite plugin.

## Development

```bash
npm install
npm run tauri dev
```

## Build

```bash
npm run tauri build
```

## Recommended IDE Setup

- [VS Code](https://code.visualstudio.com/) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)

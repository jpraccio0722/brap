# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

Scree: a live-coding language and its editor, shipped as a Tauri 2 desktop app. The frontend (`src/`) is React 19 + TypeScript + Vite, with CodeMirror 6 as the editor. The backend (`src-tauri/src/`) is Rust and holds the whole language — lexer, parser, lowerer, audio-graph realizer, pattern scheduler — plus the audio engine (`fundsp` for DSP, `cpal` for the output stream).

The repo directory is `brap`; the product, npm package and Rust crate are all `scree`.

## Commands

```bash
npm install && npm run tauri dev
```

That is the only way to run the app — `npm run dev` alone serves the frontend with no backend, so every `invoke` fails.

| | |
|---|---|
| Typecheck + build frontend | `npm run build` (`tsc && vite build`) |
| Bundle the app | `npm run tauri build` |
| Rust tests | `cargo test` from `src-tauri/` |
| One Rust test | `cargo test the_test_name` from `src-tauri/` (names are sentences: `cargo test master_volume_scales_the_output`) |

There is no linter and no frontend test runner. The Rust suite is the test suite — ~725 tests, mostly `#[cfg(test)]` modules beside the code, with larger ones in `lowerer/tests.rs`, `imports/tests.rs`, `files/tests.rs` and `library/tests.rs`.

Three tests fail on a fresh clone and always have: `the_example_project_compiles`, `every_example_compiles_and_realizes` and `a_program_without_imports_needs_no_folder`. The first two want fixture files under `examples/`, which is not checked in; the third is confused by a stray `src-tauri/patterns.scree` if one is sitting there. Confirm against `HEAD` before assuming a change caused them.

`[profile.dev]` in `Cargo.toml` raises `opt-level` for this crate and to 3 for dependencies. That is not incidental: an unoptimized `fundsp` misses the real-time deadline and the audio crackles. Leave it alone.

## The eval pipeline

`run_code` in `src-tauri/src/lib.rs` is the spine — the editor's play key (`Cmd/Ctrl + ,`) lands there, and reading it explains most of the backend. In order:

1. **`parser::parse`** — `logos` lexer (`parser/lex.rs`), `chumsky` parser (`parser/parser.rs`) → `Vec<ScreeItem>`.
2. **`imports::expand`** — resolves `use` into one flat program. After this pass no modules exist, only definitions with longer names, so nothing downstream knows about files. It is also the last moment anything knows *which* file a definition came out of, so it is where a module's `load` paths are made absolute (see Libraries below).
3. **`samples::load`** — decodes what `load` names. This is the only thread allowed to touch a disk; the scheduler thread builds a voice per note and must never block on I/O.
4. **`lowerer::lower_with_samples`** — evaluates the program into two artifacts: a `ScreeGraph` (the persistent signal graph) and pattern `Binding`s.
5. **`scree_graph::realizer::realize`** — turns the graph into a `fundsp` `Net`.
6. Publish: the net is crossfaded into the engine's one slot; instruments then patterns go to the scheduler.

Every stage returns `Result<_, Diagnostic>` tagged with a `Stage`, and nothing is swapped in until all of them agree — a program that does not compile leaves whatever is playing alone. Diagnostics surface in the editor's problems panel.

**Two things make sound, by different routes.** The graph is continuous and lives in `engine.slot`, replaced by a 0.2s crossfade. Patterns are discrete: the scheduler thread (`scheduler/scheduler.rs`) free-runs from app start, wakes every 25ms, and pushes voices into a `Sequencer` 0.2s ahead of the audio clock. An eval never "triggers" the scheduler — it swaps the state the scheduler reads on its next pass. Ordering inside `run_code` matters and is commented where it does: instruments before patterns, clock reset before patterns are published.

## Language surface lives in one table

`src-tauri/src/lang.rs` holds every callable name with its arity, parameter names, `receives`/`returns` kinds and doc string. The lowerer dispatches off these tables, and the `language_metadata` Tauri command serves the same data to `src/scree/metadata.ts`, which drives highlighting, completion, signature help and the docs panel.

So **adding a builtin means adding one entry to `UGENS` (or `LIST_BUILTINS`, etc.)** — the editor picks it up with no TypeScript change. `ValueKind` is mirrored by hand in `metadata.ts`; the Rust test `every_builtin_receives_what_it_declares` keeps the declarations honest against the compiler.

## Projects, patterns, imports

A project is a folder with a `scree-project.json` (name, bpm, volume), written debounced as you change things. `src-tauri/src/project.rs` and `files/` own it. Two other files may sit beside it — a `scree-library.json` naming what the project exports as, and a hidden `.scree/libraries/` holding vendored libraries — both covered below.

Drawn patterns from the right-hand panel are a real file, `patterns.scree` at the project root, folded into every eval as an implicit `use patterns::*`. The panel also sends its patterns *with* the eval rather than relying on the write having landed, so `run_code` takes `Option<Vec<GraphicalPattern>>`: `None` means "the panel has nothing to say, use the disk", `Some([])` means "the panel read this project and it has no patterns" — only the second may hide a file on disk.

`use` paths are routes, not names, and only ever go downward. Renaming or moving a file therefore rewrites the `use` lines that pointed at it (`files/reroute.rs`); a move that no rewrite can honestly follow still happens, and the broken imports are named in the problems panel. Imports read what is *saved*, so a module must be written to disk before a file that uses it is played.

Two rules hold across `files/`: nothing is overwritten (a collision is refused, never merged), and nothing is destroyed (deletes go to the platform trash).

## Libraries

`src-tauri/src/library/` — a shareable folder of modules, installed once and reachable from every project. A **pack** is a `.screepack`: a zip of `manifest.json` plus a `root/` whose contents are copied into a **store**, and a store is nothing but another folder a `use` resolves in. That is the whole design — `use kit::kick` already means `kit/kick.scree` beside the file, and an installed library is that same shape somewhere else, so importing, renaming and lowering learn nothing new.

Three things carry the weight:

- **`Resolver::locate` asks each root in turn and answers each in full before moving on** — beside the file, then `<project>/.scree/libraries/`, then the app config dir's store. A project file therefore beats an installed library of the same name at every step, so installing something can never change what a working project means. The roots reach `Workspace` via `set_libraries`, filled in by `run_code`: only the Rust side knows where the config dir is.
- **A pack owns exactly one top-level name.** Install refuses any entry under `root/` that is not `<name>.scree` or inside `<name>/`. That single check is why two libraries can never write the same file, and why there is no version resolution anywhere in here to get wrong. `install` returns `Outcome::Conflict` rather than replacing, having written nothing, so the editor can ask.
- **A module's `load` paths are rewritten to absolute during expansion** (`imports/rename.rs`, `Scope::relocating_samples`). Without this a library's samples resolve against whatever file imported it, which is the wrong folder — and it is the only reason a pack can carry audio at all. The program's own paths are left as written.

`library/` deliberately breaks `files/`'s trash rule: an installed pack is app-managed and reinstallable, and leaving a hundred megabytes of somebody else's samples in the Trash is worse than no undo. `export` audits before it writes, refusing an absolute `load` path, one that reaches out of the library, or a top-level `use` naming something the library does not carry — all three work on the author's machine and nowhere else.

## Frontend shape

`src/App.tsx` is deliberately the center — tabs, project state, transport, panel wiring and all the `invoke` calls live there; the panel components are mostly presentational. Native menu items arrive as Tauri events (`file-new`, `project-open`, …) listened to in `App.tsx`.

`src/scree/` is the CodeMirror extension bundle. `screeExtensions()` must be called once and memoized — CodeMirror reconfigures when the extension array's identity changes, which would discard completion state on every keystroke. Values that change (drawn pattern names, the docs callback, the `Symbols` cache) are passed as getters or long-lived objects for the same reason.

Completion reads the buffer with regexes, because the text being completed is half-written and the real parser would reject it. Two things it cannot get that way:

- **What a `use kit::*` brought in.** Those names live in a file the frontend has never read, so the `module_symbols` command runs the real expander and reports the spellings *this file* would write (`kick`, `kit::kick`, `k::kick`). `src/scree/symbols.ts` asks only when the document's `use` lines change, and answers nothing when the file does not expand — which is most keystrokes, and is the right answer while it is half-typed.
- **Which argument of a call the cursor is in.** `src/scree/callsite.ts` holds `callAt`, shared with signature help rather than written twice. It is what makes `play(pat, ` offer only playable `fn`s and `play(pat, kick, ` offer that instrument's lanes — both rules live in `lowerer/play.rs` and are otherwise invisible until the program is run.

`src/scree/indent.ts` is the third thing the frontend cannot read off the buffer alone: which line breaks end a statement. It mirrors `cont_next` in `parser/lex.rs`, so a line opening with `.` or `>>` is indented one step from the line that began the statement. It applies through `indentOnInput` rather than on Enter — a break after `some()` ends a statement until the `.` is typed, and the `.` is the only moment the answer changes.

Icons are imported as components via `vite-plugin-svgr` (`import Icon from "./icon.svg?react"`), so they take a `className` and inherit `currentColor`. Styling is Tailwind 4 via the Vite plugin.

## Conventions

Comments in this codebase explain *why*, in prose, and are load-bearing — constants, orderings and refusals carry the reasoning that would otherwise be lost. Match that: a change that invalidates a comment's reasoning should update the reasoning. Test names are full sentences describing the behavior being pinned.

`README.md` is the user-facing manual, including the full function reference. It is the place to look for what a language feature is supposed to do, and the place to update when that changes.

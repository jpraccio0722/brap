//! Patterns drawn in the side panel rather than written in the editor.
//!
//! A graphical pattern is a name and a row of beats, and it earns its keep by
//! being *the same thing* a list literal is. That used to be a resemblance —
//! the panel's rows were seeded into the environment as `Value::List`s. Now it
//! is literal: the panel writes `patterns.scree` in the project folder, one
//! `let` per row, and every program in the project has that file folded into it
//! the way an explicit `use` would fold in any other.
//!
//! So this module is a translation, both ways. [`to_source`] writes what the
//! panel holds; [`from_source`] reads a file back into rows the grid can draw.
//! Everything between the two is the ordinary language: the parser reads those
//! `let`s, the lowerer builds the lists, and nothing downstream — `to_pattern`,
//! the scheduler, the lanes — has any idea a grid was involved.

use crate::diagnostic::{Diagnostic, Stage};
use crate::lang::{self, NoteName};
use crate::parser::parser::{parse, Expr, ScreeItem};

/// What the panel's file is called, in every project.
pub const FILE: &str = "patterns.scree";

/// One beat. Pitches carry a MIDI note number; the other two are the same
/// silence and bare hit the language spells `` ` `` and `\`.
#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum GraphicalStep {
    Rest,
    Trigger,
    Pitch { note: f64 },
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, PartialEq)]
pub struct GraphicalPattern {
    pub name: String,
    pub steps: Vec<GraphicalStep>,
}

/// The header the panel's file carries.
///
/// It is a generated file that people will open — it is scree, it sits in their
/// project, and the project panel lists it — so it has to say what it is and
/// what will happen to anything else they put in it.
const HEADER: &str = "\
// Patterns drawn in the side panel.
//
// The panel rewrites this file whenever a row changes, so anything else kept
// here will be lost. Every file in this project can name these patterns
// without importing them.
";

impl GraphicalPattern {
    /// One row, as the `let` the language would have been given.
    fn to_source(&self) -> String {
        let steps: Vec<String> = self.steps.iter().map(step_source).collect();
        format!("let {} = [{}]", self.name, steps.join(", "))
    }
}

/// Every row, as a file.
pub fn to_source(patterns: &[GraphicalPattern]) -> String {
    let mut out = String::from(HEADER);
    for pattern in patterns {
        out.push('\n');
        out.push_str(&pattern.to_source());
        out.push('\n');
    }
    out
}

/// One beat, spelled the way it would be written by hand: a note name where
/// there is one, so the file reads as music rather than as MIDI.
fn step_source(step: &GraphicalStep) -> String {
    match step {
        GraphicalStep::Rest => "`".to_string(),
        GraphicalStep::Trigger => "\\".to_string(),
        GraphicalStep::Pitch { note } => note_source(*note),
    }
}

fn note_source(note: f64) -> String {
    /// Sharps rather than flats, matching `lang::note`'s own spelling.
    const NAMES: [&str; 12] = [
        "c", "cs", "d", "ds", "e", "f", "fs", "g", "gs", "a", "as", "b",
    ];

    // Only whole notes inside the range note names cover; anything else is a
    // number, which the language reads just as well.
    let whole = note.fract() == 0.0 && note >= 12.0 && note <= 127.0;
    if !whole {
        return format_number(note);
    }

    let midi = note as i64;
    format!("{}{}", NAMES[(midi % 12) as usize], midi / 12 - 1)
}

/// A number without a trailing `.0`, since the file is read by people.
fn format_number(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        return format!("{}", n as i64);
    }
    format!("{n}")
}

/// Read a patterns file back into rows.
///
/// Anything the grid cannot draw is passed over rather than refused: the file
/// is ordinary scree and somebody may well have written something in it that a
/// row of cells has no way to show. It still compiles — the whole file is
/// included in every program — it simply is not drawn.
///
/// A file that does not parse *is* refused, because the alternative is a panel
/// that silently shows nothing and then overwrites what it could not read.
pub fn from_source(code: &str) -> Result<Vec<GraphicalPattern>, Diagnostic> {
    let items = parse(code.to_string())?;

    let mut patterns = Vec::new();
    for item in &items {
        let ScreeItem::Let { name, value: Expr::List(steps) } = item else {
            continue;
        };
        let Some(steps) = steps.iter().map(parse_step).collect::<Option<Vec<_>>>() else {
            continue;
        };
        patterns.push(GraphicalPattern { name: name.0.clone(), steps });
    }

    Ok(patterns)
}

/// One element of a list, as a beat — or `None` when it is something a cell
/// cannot hold, like a nested list or an expression.
fn parse_step(expr: &Expr) -> Option<GraphicalStep> {
    match expr {
        Expr::Rest => Some(GraphicalStep::Rest),
        Expr::Trigger => Some(GraphicalStep::Trigger),
        Expr::Num(n) => Some(GraphicalStep::Pitch { note: *n }),
        // A note name is a plain variable to the parser; the lowerer only reads
        // it as a pitch when nothing else has that name, and so do we.
        Expr::Var(name) => match lang::note(&name.0) {
            NoteName::Note(note) => Some(GraphicalStep::Pitch { note }),
            _ => None,
        },
        _ => None,
    }
}

/// Refuse a name the panel could not write and read back.
///
/// The panel enforces this while you type, so a failure here means something
/// got past it — worth an error rather than a file with an unusable name in it.
pub fn check_names(patterns: &[GraphicalPattern]) -> Result<(), Diagnostic> {
    for (i, p) in patterns.iter().enumerate() {
        let mut chars = p.name.chars();
        let ok = matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
        if !ok {
            return Err(Diagnostic::message(
                Stage::Import,
                format!(
                    "'{}' is not a usable pattern name: start with a letter or \
                     underscore, then letters, digits or underscores",
                    p.name
                ),
            ));
        }
        if patterns[..i].iter().any(|q| q.name == p.name) {
            return Err(Diagnostic::message(
                Stage::Import,
                format!("two graphical patterns are both named '{}'", p.name),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lowerer::lower::lower;
    use crate::pattern::pattern::{Pattern, Step};

    fn pat(name: &str, steps: Vec<GraphicalStep>) -> GraphicalPattern {
        GraphicalPattern { name: name.to_string(), steps }
    }

    fn pitch(note: f64) -> GraphicalStep {
        GraphicalStep::Pitch { note }
    }

    /// The claim this module rests on: a drawn row and the list a person would
    /// have typed are the same program.
    #[test]
    fn a_drawn_pattern_is_the_list_it_stands_for() {
        let source = to_source(&[pat(
            "riff",
            vec![pitch(60.0), GraphicalStep::Rest, GraphicalStep::Trigger],
        )]);

        let items = parse(format!("fn kick(f) = sin(f)\n{source}\nplay(riff, kick)\n"))
            .expect("the written file should parse");
        let lowered = lower(&items).expect("and lower");

        assert_eq!(
            lowered.bindings[0].pattern,
            Pattern::Steps(vec![Step::Value(60.0), Step::Rest, Step::Value(1.0)]),
        );
    }

    /// Written as music: the file is meant to be read, and `c4` says what `60`
    /// only means.
    #[test]
    fn pitches_are_written_as_note_names() {
        let source = to_source(&[pat("riff", vec![pitch(60.0), pitch(61.0), pitch(127.0)])]);
        assert!(source.contains("let riff = [c4, cs4, g9]"), "got:\n{source}");
    }

    /// Except where there is no name to write: the panel takes a MIDI number,
    /// and a number is what comes back out.
    #[test]
    fn unnameable_pitches_stay_numbers() {
        let source = to_source(&[pat("odd", vec![pitch(60.5), pitch(4.0)])]);
        assert!(source.contains("let odd = [60.5, 4]"), "got:\n{source}");
    }

    /// What the panel writes, the panel reads.
    #[test]
    fn a_written_file_reads_back_the_same() {
        let patterns = vec![
            pat("hats", vec![GraphicalStep::Trigger, GraphicalStep::Rest]),
            pat("riff", vec![pitch(60.0), pitch(60.5), GraphicalStep::Rest]),
            pat("empty", vec![]),
        ];
        assert_eq!(from_source(&to_source(&patterns)).unwrap(), patterns);
    }

    /// And what a person writes, the panel reads: both spellings of a pitch,
    /// and both of a step.
    #[test]
    fn a_hand_written_file_is_read_too() {
        let read = from_source("let hats = [\\, `, ef3, 62]\n").expect("should read");
        assert_eq!(
            read,
            vec![pat(
                "hats",
                vec![
                    GraphicalStep::Trigger,
                    GraphicalStep::Rest,
                    pitch(51.0),
                    pitch(62.0),
                ]
            )]
        );
    }

    /// A file may hold more than a grid can draw. It still compiles — every
    /// program in the project includes the whole of it — so the panel passes
    /// over what it cannot show rather than refusing the file.
    #[test]
    fn what_the_grid_cannot_draw_is_passed_over() {
        let read = from_source(
            "let hats = [\\, `]\nfn kick(f) = sin(f)\nlet nested = [[1, 2], 3]\nlet n = 4\n",
        )
        .expect("should read");
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].name, "hats");
    }

    /// A file that does not parse is refused, and says why: the panel showing
    /// nothing and then writing over it is how work disappears.
    #[test]
    fn a_broken_file_is_refused() {
        let err = from_source("let hats = [\\, `\nfn oops(").expect_err("should not read");
        assert!(matches!(err.stage, Stage::Parse | Stage::Lex), "{err}");
    }

    /// Empty in, empty out — and a file that is only a header, so a project
    /// with no patterns still has an honest file rather than a stale one.
    #[test]
    fn no_patterns_is_a_file_with_no_patterns() {
        let source = to_source(&[]);
        assert_eq!(from_source(&source).unwrap(), Vec::new());
        assert!(source.starts_with("// Patterns drawn"));
    }

    #[test]
    fn bad_names_are_refused() {
        assert!(check_names(&[pat("2fast", vec![])]).is_err());
        assert!(check_names(&[pat("has space", vec![])]).is_err());
        assert!(check_names(&[pat("", vec![])]).is_err());
        assert!(check_names(&[pat("hats_2", vec![])]).is_ok());
    }

    #[test]
    fn duplicate_names_are_refused() {
        let err = check_names(&[pat("hats", vec![]), pat("hats", vec![])])
            .expect_err("duplicates must not be written");
        assert!(err.message.contains("hats"), "{err}");
    }

    /// The exact shape `src/patterns.ts` sends. Worth pinning: the two sides
    /// only ever meet over this JSON, and a renamed tag would fail at the one
    /// moment it cannot be caught — someone's eval, mid-performance.
    #[test]
    fn the_wire_format_is_what_the_panel_sends() {
        let json = r#"[{"name":"hats","steps":[
            {"kind":"trigger"},{"kind":"pitch","note":54},{"kind":"rest"}]}]"#;
        let patterns: Vec<GraphicalPattern> = serde_json::from_str(json).expect("wire format");

        assert_eq!(patterns.len(), 1);
        assert_eq!(patterns[0].name, "hats");

        // All the way through: the JSON the panel sent, written as a file,
        // read as a program, and played as the rhythm that was drawn.
        let source = to_source(&patterns);
        let items = parse(format!("fn tone(f) = sin(f)\n{source}\nplay(hats, tone)\n"))
            .expect("should parse");
        let lowered = lower(&items).expect("should lower");
        assert_eq!(
            lowered.bindings[0].pattern,
            Pattern::Steps(vec![Step::Value(1.0), Step::Value(54.0), Step::Rest]),
        );
    }

    /// And the shape it reads back, which the panel deserializes.
    #[test]
    fn rows_serialize_for_the_panel() {
        let json = serde_json::to_string(&[pat("hats", vec![GraphicalStep::Trigger, pitch(54.0)])])
            .expect("should serialize");
        assert_eq!(
            json,
            r#"[{"name":"hats","steps":[{"kind":"trigger"},{"kind":"pitch","note":54.0}]}]"#
        );
    }
}

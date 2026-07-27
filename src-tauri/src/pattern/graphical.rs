//! Patterns drawn in the side panel rather than written in the editor.
//!
//! A graphical pattern is a name and a row of beats, and it earns its keep by
//! being *the same thing* a list literal is: it lowers to a `Value::List`,
//! seeded into the environment before the program runs, so `play(hats, hat)`
//! reads a drawn pattern exactly as it would read `let hats = [...]`. Nothing
//! downstream — `to_pattern`, the scheduler, the lanes — needs to know which
//! one it got.

use std::rc::Rc;

use crate::scree_graph::environment::Value;

/// One beat. Pitches carry a MIDI note number; the other two are the same
/// silence and bare hit the language spells `` ` `` and `\`.
#[derive(serde::Deserialize, Clone, Debug)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum GraphicalStep {
    Rest,
    Trigger,
    Pitch { note: f64 },
}

#[derive(serde::Deserialize, Clone, Debug)]
pub struct GraphicalPattern {
    pub name: String,
    pub steps: Vec<GraphicalStep>,
}

impl GraphicalPattern {
    /// The list this pattern stands for.
    pub fn value(&self) -> Value {
        Value::List(Rc::new(
            self.steps
                .iter()
                .map(|s| match s {
                    GraphicalStep::Rest => Value::Rest,
                    GraphicalStep::Trigger => Value::Trigger,
                    GraphicalStep::Pitch { note } => Value::Number(*note),
                })
                .collect(),
        ))
    }

    /// The panel enforces both of these while you type, so a failure here means
    /// something got past it — worth an error rather than a silent binding the
    /// editor cannot name.
    fn check_name(&self) -> Result<(), String> {
        let mut chars = self.name.chars();
        let ok = matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
        if !ok {
            return Err(format!(
                "'{}' is not a usable pattern name: start with a letter or \
                 underscore, then letters, digits or underscores",
                self.name
            ));
        }
        Ok(())
    }
}

/// Bind every drawn pattern, so the program can name them.
///
/// Done before the program's own items, which means a `let` of the same name
/// shadows the panel rather than colliding with it.
pub fn define_all(
    env: &mut crate::scree_graph::environment::Env,
    patterns: &[GraphicalPattern],
) -> Result<(), String> {
    for (i, p) in patterns.iter().enumerate() {
        p.check_name()?;
        if patterns[..i].iter().any(|q| q.name == p.name) {
            return Err(format!("two graphical patterns are both named '{}'", p.name));
        }
        env.define(&p.name, p.value());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lowerer::play::to_pattern;
    use crate::pattern::pattern::{Pattern, Span, Step};

    fn pat(name: &str, steps: Vec<GraphicalStep>) -> GraphicalPattern {
        GraphicalPattern { name: name.to_string(), steps }
    }

    #[test]
    fn a_drawn_pattern_lowers_like_the_list_it_stands_for() {
        let p = pat(
            "riff",
            vec![
                GraphicalStep::Pitch { note: 60.0 },
                GraphicalStep::Rest,
                GraphicalStep::Trigger,
            ],
        );
        assert_eq!(
            to_pattern(&p.value()).unwrap(),
            Pattern::Steps(vec![Step::Value(60.0), Step::Rest, Step::Value(1.0)]),
        );
    }

    #[test]
    fn an_empty_pattern_is_silent_rather_than_an_error() {
        let p = pat("blank", vec![]);
        let lowered = to_pattern(&p.value()).unwrap();
        assert!(lowered.query(Span::new(0.0, 4.0)).is_empty());
    }

    #[test]
    fn bad_names_are_refused() {
        let mut env = crate::scree_graph::environment::Env::new();
        assert!(define_all(&mut env, &[pat("2fast", vec![])]).is_err());
        assert!(define_all(&mut env, &[pat("has space", vec![])]).is_err());
        assert!(define_all(&mut env, &[pat("", vec![])]).is_err());
        assert!(define_all(&mut env, &[pat("hats_2", vec![])]).is_ok());
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
        assert_eq!(
            to_pattern(&patterns[0].value()).unwrap(),
            Pattern::Steps(vec![Step::Value(1.0), Step::Value(54.0), Step::Rest]),
        );
    }

    #[test]
    fn duplicate_names_are_refused() {
        let mut env = crate::scree_graph::environment::Env::new();
        let err = define_all(&mut env, &[pat("hats", vec![]), pat("hats", vec![])])
            .expect_err("duplicates must not bind");
        assert!(err.contains("hats"), "{err}");
    }
}

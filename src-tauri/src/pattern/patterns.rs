use crate::pattern::pattern::{Event, Pattern, Span};

/// A pattern paired with the instrument that plays it.
#[derive(Clone, Debug, PartialEq)]
pub struct Binding {
    pub instrument: String,   // name of the user `fn`
    pub pattern: Pattern,
}

/// Everything currently playing. An eval replaces this wholesale.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Patterns { pub bindings: Vec<Binding> }

/// An event with its instrument attached — what the scheduler consumes.
#[derive(Clone, Debug, PartialEq)]
pub struct BoundEvent { pub instrument: String, pub event: Event }

impl Patterns {
    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

    pub fn query(&self, span: Span) -> Vec<BoundEvent> {
        self.bindings.iter().flat_map(|b| {
            b.pattern.query(span).into_iter()
                .map(move |event| BoundEvent { instrument: b.instrument.clone(), event })
        }).collect()
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bound_events_carry_their_instrument() {
        let pats = Patterns {
            bindings: vec![
                Binding {
                    instrument: "kick".into(),
                    pattern: Pattern::steps([Some(1.0), None]),
                },
                Binding {
                    instrument: "hat".into(),
                    pattern: Pattern::steps([Some(1.0), Some(1.0)]),
                },
            ],
        };

        let evs = pats.query(Span::new(0.0, 1.0));
        let mut names: Vec<_> = evs.iter().map(|b| b.instrument.as_str()).collect();
        names.sort();
        assert_eq!(names, vec!["hat", "hat", "kick"]);
    }

    #[test]
    fn empty_pattern_set_queries_to_nothing() {
        let pats = Patterns::default();
        assert!(pats.is_empty());
        assert!(pats.query(Span::new(0.0, 100.0)).is_empty());
    }
}

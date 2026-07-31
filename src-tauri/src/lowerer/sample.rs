//! `load`, `sample` and `secs` — the three names a buffer needs.
//!
//! `load("break.wav")` is intercepted before its argument is evaluated, the way
//! `play` is and for the same kind of reason: the path has to be a literal.
//! `samples::Samples` was built by walking the syntax for exactly these calls
//! before any of this ran, so a path assembled at runtime would name a file
//! nobody had loaded — and the only moment it could be loaded is on the audio
//! thread, one note at a time.
//!
//! `sample(buffer, position)` is a UGen like any other except for its first
//! argument, which is a buffer rather than a signal. That is what keeps it out
//! of the `UGENS` table's ordinary path: the table's lowering takes every
//! argument through `as_input`, and a buffer has no input to be.
//!
//! `secs` is neither — it answers with a number during lowering, like the maths
//! builtins, because the length of a buffer is known the moment it is loaded.

use std::sync::Arc;

use fundsp::wave::Wave;

use crate::lowerer::lower::Lowerer;
use crate::parser::parser::{Arg, Expr};
use crate::samples::LOAD;
use crate::scree_graph::environment::Value;
use crate::scree_graph::ugen_nodes::{NodeInput, NodeKind};

/// Reading a buffer at a position.
pub const SAMPLE: &str = "sample";
/// How long a buffer is, in seconds.
pub const SECS: &str = "secs";
/// How many channels a buffer has.
pub const CHANNELS: &str = "channels";

impl Lowerer {
    /// True when this call names a file rather than computing something.
    pub fn is_load(name: &str) -> bool {
        name == LOAD
    }

    /// `load("break.wav")` — the buffer that path was decoded into.
    ///
    /// Nothing is read from disk here. The map was filled at eval time; a miss
    /// means the walk that filled it did not see this call, which is a bug in
    /// the walk rather than anything a program did.
    pub fn load(&mut self, args: &[Arg], piped: Option<Value>) -> Result<Value, String> {
        if piped.is_some() {
            return Err(format!("{LOAD}: a path is not something to chain into"));
        }
        let [arg] = args else {
            return Err(format!(
                "{LOAD} expects one path, written out: {LOAD}(\"break.wav\")"));
        };
        if arg.name.is_some() {
            return Err(format!("{LOAD}: the path is not a named argument"));
        }
        let Expr::Str(path) = &arg.value else {
            return Err(format!(
                "{LOAD}: the path must be written out as a string — \
                 {LOAD}(\"break.wav\") — so the file can be read before anything plays"));
        };

        match self.samples.get(path) {
            Some(wave) => Ok(Value::Buffer(wave)),
            None => Err(format!(
                "{LOAD}: \"{path}\" was not loaded before this program ran")),
        }
    }

    /// True when this call reads or measures a buffer.
    pub fn is_buffer_builtin(name: &str) -> bool {
        matches!(name, SAMPLE | SECS | CHANNELS)
    }

    /// The three that take a buffer as their first argument.
    ///
    /// Dispatched together, after arguments are evaluated, because all three
    /// begin by insisting on a buffer and the message for not having one should
    /// be the same however you got here.
    pub fn buffer_builtin(&mut self, name: &str, args: &[Value]) -> Result<Value, String> {
        let Some(Value::Buffer(wave)) = args.first().cloned() else {
            return Err(format!(
                "{name}: the first argument must be a buffer from `{LOAD}(\"...\")`"));
        };

        match name {
            SECS => arity(name, args, 1).map(|_| Value::Number(seconds(&wave))),
            CHANNELS => arity(name, args, 1).map(|_| Value::Number(wave.channels() as f64)),
            SAMPLE => {
                if args.len() < 2 || args.len() > 3 {
                    return Err(format!(
                        "{SAMPLE} expects a buffer and a position, and optionally a \
                         channel: {SAMPLE}(buffer, position) — got {} arguments",
                        args.len()));
                }
                let position = self.as_input(args[1].clone())?;

                // A stereo file read on one channel is the usual case, so the
                // channel defaults rather than being asked for every time.
                let channel = match args.get(2) {
                    None => 0.0,
                    Some(Value::Number(n)) if *n >= 0.0 && n.fract() == 0.0 => *n,
                    Some(Value::Number(n)) => return Err(format!(
                        "{SAMPLE}: channel must be a whole number from 0, got {n}")),
                    Some(_) => return Err(format!(
                        "{SAMPLE}: channel is chosen when the graph is built, so it \
                         must be a compile-time number rather than a signal")),
                };

                let index = self.graph.intern_sample(wave) as f64;
                let node = self.push_node(
                    NodeKind::Sample,
                    vec![position, NodeInput::Const(index), NodeInput::Const(channel)],
                );
                Ok(Value::Signal(node))
            }
            _ => Err(format!("{name} is not a buffer function")),
        }
    }
}

/// How long a buffer plays for, in seconds. Zero for a buffer with no sample
/// rate to speak of, rather than an infinity that would spread into whatever
/// the program divides by it.
fn seconds(wave: &Arc<Wave>) -> f64 {
    if wave.sample_rate() > 0.0 {
        wave.length() as f64 / wave.sample_rate()
    } else {
        0.0
    }
}

fn arity(name: &str, args: &[Value], expected: usize) -> Result<(), String> {
    if args.len() == expected {
        Ok(())
    } else {
        Err(format!("{name} expects {expected} argument, got {}", args.len()))
    }
}

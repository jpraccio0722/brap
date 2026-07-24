use crate::{brap_graph::{environment::Value, ugen_nodes::NodeKind}, lowerer::lower::Lowerer, parser::parser::{Expr, Ident}};


impl Lowerer {
    pub fn call(&mut self, func: &Ident, args: &[Expr]) -> Result<Value, String> {
        self.call_with(func, args, None)
    }

    pub fn call_with(&mut self, func: &Ident, args: &[Expr], piped: Option<Value>) -> Result<Value, String> {
        let mut arg_vals: Vec<Value> = Vec::with_capacity(args.len() + 1);

        if let Some(p) = piped {
            arg_vals.push(p);
        }
        
        for arg in args {
            arg_vals.push(self.expr(arg)?);
        }

        if let Some((kind, arity)) = self.builtin(&func.0) {
            if arg_vals.len() != arity {
               return Err(format!("{} expects {} inputs, got {}",
                                   func.0, arity, arg_vals.len()));
            }
            let inputs = arg_vals.into_iter()
                .map(|v| self.as_input(v))
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(Value::Signal(self.push_node(kind, inputs)));
        } 

        let Some(Value::Function(def)) = self.env.lookup(&func.0) else {
            return Err(format!("{} is not a function", func.0));
        };

        if arg_vals.len() > def.params.len() {
            return Err(format!("{} expects at most {} args, got {}",
                               func.0, def.params.len(), arg_vals.len()));
        }
        if self.depth >= 64 {
            return Err(format!("call depth exceeded inlining {} (recursive fn?)", func.0));
        }

        self.depth += 1;
        self.env.push_scope();
        let result = (|| {
            for (i, param) in def.params.iter().enumerate() {
                let v = match (arg_vals.get(i), &param.default) {
                    (Some(v), _) => v.clone(),
                    (None, Some(d)) => self.expr(d)?,
                    (None, None) => return Err(format!(
                        "{}: missing argument '{}'", func.0, param.name.0)),
                };
                self.env.define(&param.name.0, v);
            }
            self.expr(&def.body)
        })();
        self.env.pop_scope();
        self.depth -= 1;
        result
        
    }

    fn builtin(&self, func: &str) -> Option<(NodeKind, usize)> {
        match func {
            "adsr" => Some((NodeKind::ADSR, 5)),
            "afollow" => Some((NodeKind::Afollow, 3)),
            "allpass" => Some((NodeKind::Allpass, 3)),
            "allpole" => Some((NodeKind::Allpole, 2)),
            "bandpass" => Some((NodeKind::Bandpass, 3)),
            "bandrez" => Some((NodeKind::Bandrez, 3)),
            "bell" => Some((NodeKind::Bell, 4)),
            "biquad" => Some((NodeKind::Biquad, 6)),
            "brown" => Some((NodeKind::Brown, 0)),
            "butterpass" => Some((NodeKind::Butterpass, 2)),
            "chorus" => Some((NodeKind::Chorus, 5)),
            "clip" => Some((NodeKind::Clip, 1)),
            "clip_to" => Some((NodeKind::ClipTo, 3)),
            "dcblock" => Some((NodeKind::Dcblock, 1)),
            "declick" => Some((NodeKind::Declick, 1)),
            "delay" => Some((NodeKind::Delay, 2)),
            "dsf_saw" => Some((NodeKind::DsfSaw, 2)),
            "dsf_square" => Some((NodeKind::DsfSquare, 2)),
            "fir3" => Some((NodeKind::Fir3, 2)),
            "follow" => Some((NodeKind::Follow, 2)),
            "hammond" => Some((NodeKind::Hammond, 1)),
            "highpass" => Some((NodeKind::Highpass, 3)),
            "highpole" => Some((NodeKind::Highpole, 2)),
            "highshelf" => Some((NodeKind::Highshelf, 4)),
            "hold" => Some((NodeKind::Hold, 3)),
            "impulse" => Some((NodeKind::Impulse, 0)),
            "limiter" => Some((NodeKind::Limiter, 3)),
            "lorenz" => Some((NodeKind::Lorenz, 1)),
            "lowpass" => Some((NodeKind::Lowpass, 3)),
            "lowpole" => Some((NodeKind::Lowpole, 2)),
            "lowrez" => Some((NodeKind::Lowrez, 3)),
            "lowshelf" => Some((NodeKind::Lowshelf, 4)),
            "mls" => Some((NodeKind::Mls, 0)),
            "mls_bits" => Some((NodeKind::MlsBits, 1)),
            "moog" => Some((NodeKind::Moog, 3)),
            "morph" => Some((NodeKind::Morph, 4)),
            "noise" => Some((NodeKind::Noise, 0)),
            "notch" => Some((NodeKind::Notch, 3)),
            "organ" => Some((NodeKind::Organ, 1)),
            "peak" => Some((NodeKind::Peak, 3)),
            "pink" => Some((NodeKind::Pink, 0)),
            "pinkpass" => Some((NodeKind::Pinkpass, 1)),
            "pluck" => Some((NodeKind::Pluck, 4)),
            "poly_pulse" => Some((NodeKind::PolyPulse, 2)),
            "poly_saw" => Some((NodeKind::PolySaw, 1)),
            "poly_square" => Some((NodeKind::PolySquare, 1)),
            "pulse" => Some((NodeKind::Pulse, 2)),
            "ramp" => Some((NodeKind::Ramp, 1)),
            "resonator" => Some((NodeKind::Resonator, 3)),
            "rossler" => Some((NodeKind::Rossler, 1)),
            "saw" => Some((NodeKind::Saw, 1)),
            "sin" => Some((NodeKind::Sin, 1)),
            "soft_saw" => Some((NodeKind::SoftSaw, 1)),
            "square" => Some((NodeKind::Square, 1)),
            "tap" => Some((NodeKind::Tap, 4)),
            "tick" => Some((NodeKind::Tick, 1)),
            "triangle" => Some((NodeKind::Triangle, 1)),
            _ => None
        }
    }
}
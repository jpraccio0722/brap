//! The language's callable surface, as data.
//!
//! Names, arities and parameter names used to live in three places at once: the
//! match arm in `lowerer::call`, the arity literals in `lowerer::lists`, and —
//! for parameters — nowhere except the error strings in `scree_graph::realizer`.
//! The editor needs all of it, so it lives here instead, and the lowerer reads
//! it rather than repeating it. Adding a UGen to `UGENS` is now the only step
//! needed to make it callable *and* completable.
//!
//! Parameter names for the pure audio-rate UGens follow fundsp's own `Input N:`
//! documentation; the ones baked in at construction follow the names already
//! used in the realizer's error messages.

use serde::Serialize;

use crate::scree_graph::ugen_nodes::NodeKind;

/// A UGen builtin: a name that lowers to a graph node.
///
/// Arity is `params.len()` — the two cannot disagree.
pub struct Ugen {
    pub name: &'static str,
    pub kind: NodeKind,
    pub params: &'static [&'static str],
    pub doc: &'static str,
}

/// A list builtin: a name evaluated during lowering that emits no node.
///
/// Unlike UGens these can accept several arg counts, so `arities` is explicit
/// rather than derived. `variadic` means "or anything above the largest listed".
pub struct ListBuiltin {
    pub name: &'static str,
    pub params: &'static [&'static str],
    pub arities: &'static [usize],
    pub variadic: bool,
    pub doc: &'static str,
}

/// Words the lexer reserves. `null` is deliberately absent: it is lexed
/// (`parser::lex::Token::Null`) but no parser rule ever consumes it, so
/// offering it as a completion would suggest something unusable.
pub static KEYWORDS: &[&str] = &["fn", "let", "if", "else", "for", "in"];

pub static UGENS: &[Ugen] = &[
    Ugen {
        name: "adsr",
        kind: NodeKind::ADSR,
        params: &["gate", "attack", "decay", "sustain", "release"],
        doc: "Gated ADSR envelope. Rises while the gate is positive, releases when it returns to zero. Times are in seconds; sustain is a level in 0..=1.",
    },
    Ugen {
        name: "afollow",
        kind: NodeKind::Afollow,
        params: &["signal", "attack", "release"],
        doc: "Asymmetric parameter follower. Smooths rising segments over `attack` and falling ones over `release` (halfway response times, in seconds).",
    },
    Ugen {
        name: "allpass",
        kind: NodeKind::Allpass,
        params: &["audio", "frequency", "q"],
        doc: "Allpass filter. Passes all frequencies but shifts their phase around the center frequency.",
    },
    Ugen {
        name: "allpole",
        kind: NodeKind::Allpole,
        params: &["audio", "delay"],
        doc: "First-order allpass filter with a configurable delay at DC, in samples (must be > 0).",
    },
    Ugen {
        name: "bandpass",
        kind: NodeKind::Bandpass,
        params: &["audio", "frequency", "q"],
        doc: "Bandpass filter. Keeps frequencies near the center, attenuating either side.",
    },
    Ugen {
        name: "bandrez",
        kind: NodeKind::Bandrez,
        params: &["audio", "frequency", "q"],
        doc: "Resonant two-pole bandpass filter.",
    },
    Ugen {
        name: "bell",
        kind: NodeKind::Bell,
        params: &["audio", "frequency", "q", "gain"],
        doc: "Bell equalizer. Boosts or cuts a band around the center frequency by `gain` (an amplitude multiplier, not dB).",
    },
    Ugen {
        name: "biquad",
        kind: NodeKind::Biquad,
        params: &["signal", "a1", "a2", "b0", "b1", "b2"],
        doc: "Arbitrary biquad filter with coefficients in normalized form. All five coefficients must be constants.",
    },
    Ugen {
        name: "brown",
        kind: NodeKind::Brown,
        params: &[],
        doc: "Brown noise: -6 dB per octave. Darker than pink.",
    },
    Ugen {
        name: "butterpass",
        kind: NodeKind::Butterpass,
        params: &["audio", "cutoff"],
        doc: "Second-order Butterworth lowpass. Maximally flat passband, no resonance control.",
    },
    Ugen {
        name: "chorus",
        kind: NodeKind::Chorus,
        params: &["audio", "seed", "separation", "variation", "mod_frequency"],
        doc: "Five-voice mono chorus, mixed with the dry signal. Stack two with different seeds for stereo. All parameters except the audio input are constants.",
    },
    Ugen {
        name: "clip",
        kind: NodeKind::Clip,
        params: &["signal"],
        doc: "Hard-clip the signal to -1..=1.",
    },
    Ugen {
        name: "clip_to",
        kind: NodeKind::ClipTo,
        params: &["signal", "minimum", "maximum"],
        doc: "Hard-clip the signal to `minimum`..=`maximum`. Both bounds are constants.",
    },
    Ugen {
        name: "dcblock",
        kind: NodeKind::Dcblock,
        params: &["signal"],
        doc: "Remove DC offset, keeping the signal zero-centered. Cutoff is 10 Hz.",
    },
    Ugen {
        name: "declick",
        kind: NodeKind::Declick,
        params: &["signal"],
        doc: "Fade the signal in over 10 ms from time zero, suppressing the click at the start of a graph.",
    },
    Ugen {
        name: "delay",
        kind: NodeKind::Delay,
        params: &["signal", "time"],
        doc: "Fixed delay of `time` seconds, rounded to the nearest sample. The time is a constant — use `tap` for a modulatable delay.",
    },
    Ugen {
        name: "dsf_saw",
        kind: NodeKind::DsfSaw,
        params: &["frequency", "roughness"],
        doc: "Saw-like discrete summation formula oscillator. `roughness` in 0..=1 sets how much successive partials are attenuated.",
    },
    // Time-based envelopes. `env` needs the note length, which voices pre-bind
    // as `dur`; `perc` is self-contained.
    Ugen {
        name: "env",
        kind: NodeKind::Env,
        params: &["attack", "decay", "sustain", "release", "duration"],
        doc: "Time-based ADSR for one-shot voices, with the release landing exactly on `duration`. Pass the voice-bound `dur` as the duration. All arguments are constants.",
    },
    Ugen {
        name: "dsf_square",
        kind: NodeKind::DsfSquare,
        params: &["frequency", "roughness"],
        doc: "Square-like discrete summation formula oscillator. `roughness` in 0..=1 sets how much successive partials are attenuated.",
    },
    Ugen {
        name: "fir3",
        kind: NodeKind::Fir3,
        params: &["signal", "gain"],
        doc: "Three-point symmetric FIR filter, specified by its `gain` (>= 0) at the Nyquist frequency. A gain below 1 gives a gentle lowpass.",
    },
    Ugen {
        name: "follow",
        kind: NodeKind::Follow,
        params: &["signal", "response_time"],
        doc: "Parameter follower. Smooths the signal with the given halfway response time, in seconds.",
    },
    Ugen {
        name: "hammond",
        kind: NodeKind::Hammond,
        params: &["frequency"],
        doc: "Hammond organ wavetable oscillator. Emphasizes the first three partials.",
    },
    Ugen {
        name: "highpass",
        kind: NodeKind::Highpass,
        params: &["audio", "cutoff", "q"],
        doc: "Resonant highpass filter.",
    },
    Ugen {
        name: "highpole",
        kind: NodeKind::Highpole,
        params: &["audio", "cutoff"],
        doc: "First-order one-pole one-zero highpass. No resonance.",
    },
    Ugen {
        name: "highshelf",
        kind: NodeKind::Highshelf,
        params: &["audio", "frequency", "q", "gain"],
        doc: "High shelf filter. Scales everything above the center frequency by `gain` (an amplitude multiplier).",
    },
    Ugen {
        name: "hold",
        kind: NodeKind::Hold,
        params: &["signal", "frequency", "variability"],
        doc: "Sample-and-hold. Samples the signal at `frequency` Hz; `variability` in 0..=1 jitters the sampling interval and is a constant.",
    },
    Ugen {
        name: "impulse",
        kind: NodeKind::Impulse,
        params: &[],
        doc: "A single one followed by silence. Useful for exciting `pluck` or measuring an impulse response.",
    },
    Ugen {
        name: "limiter",
        kind: NodeKind::Limiter,
        params: &["signal", "attack", "release"],
        doc: "Look-ahead limiter holding the signal to -1..=1. Look-ahead equals the attack time. Times are constants, in seconds.",
    },
    Ugen {
        name: "lorenz",
        kind: NodeKind::Lorenz,
        params: &["frequency"],
        doc: "Lorenz chaotic oscillator. The frequency input has only a slight effect on the output.",
    },
    Ugen {
        name: "lowpass",
        kind: NodeKind::Lowpass,
        params: &["audio", "cutoff", "q"],
        doc: "Resonant lowpass filter.",
    },
    Ugen {
        name: "lowpole",
        kind: NodeKind::Lowpole,
        params: &["audio", "cutoff"],
        doc: "First-order one-pole lowpass. No resonance.",
    },
    Ugen {
        name: "lowrez",
        kind: NodeKind::Lowrez,
        params: &["audio", "cutoff", "q"],
        doc: "Resonant two-pole lowpass filter.",
    },
    Ugen {
        name: "lowshelf",
        kind: NodeKind::Lowshelf,
        params: &["audio", "frequency", "q", "gain"],
        doc: "Low shelf filter. Scales everything below the center frequency by `gain` (an amplitude multiplier).",
    },
    Ugen {
        name: "mls",
        kind: NodeKind::Mls,
        params: &[],
        doc: "Maximum length sequence noise: a repeating pseudorandom run of -1 and 1.",
    },
    Ugen {
        name: "mls_bits",
        kind: NodeKind::MlsBits,
        params: &["bits"],
        doc: "Maximum length sequence noise from an n-bit sequence (1..=31). More bits means a longer period before it repeats. Constant.",
    },
    Ugen {
        name: "moog",
        kind: NodeKind::Moog,
        params: &["signal", "cutoff", "q"],
        doc: "Moog-style resonant lowpass ladder filter.",
    },
    Ugen {
        name: "morph",
        kind: NodeKind::Morph,
        params: &["signal", "frequency", "q", "morph"],
        doc: "Filter that morphs continuously between modes: `morph` runs -1 (lowpass) to 0 (peak) to 1 (highpass).",
    },
    Ugen {
        name: "noise",
        kind: NodeKind::Noise,
        params: &[],
        doc: "White noise.",
    },
    Ugen {
        name: "notch",
        kind: NodeKind::Notch,
        params: &["audio", "frequency", "q"],
        doc: "Notch filter. Removes a narrow band around the center frequency.",
    },
    Ugen {
        name: "organ",
        kind: NodeKind::Organ,
        params: &["frequency"],
        doc: "Organ wavetable oscillator. Emphasizes octave partials.",
    },
    Ugen {
        name: "peak",
        kind: NodeKind::Peak,
        params: &["audio", "frequency", "q"],
        doc: "Peaking filter.",
    },
    Ugen {
        name: "perc",
        kind: NodeKind::Perc,
        params: &["attack", "release"],
        doc: "Self-contained percussive envelope: rise, fall, silence. Needs no note length, so it works in a voice or the persistent graph. Both times are constants.",
    },
    Ugen {
        name: "pink",
        kind: NodeKind::Pink,
        params: &[],
        doc: "Pink noise: -3 dB per octave.",
    },
    Ugen {
        name: "pinkpass",
        kind: NodeKind::Pinkpass,
        params: &["signal"],
        doc: "Pinking filter: -3 dB per octave. Turns white noise into pink.",
    },
    Ugen {
        name: "pluck",
        kind: NodeKind::Pluck,
        params: &["excitation", "frequency", "gain_per_second", "damping"],
        doc: "Karplus-Strong plucked string. Feed it a burst — `impulse()` or a short noise envelope — as the excitation. Frequency, gain and damping (0..=1) are constants.",
    },
    Ugen {
        name: "poly_pulse",
        kind: NodeKind::PolyPulse,
        params: &["frequency", "width"],
        doc: "PolyBLEP pulse wave. Fast and fairly bandlimited; `width` in 0..=1 is the duty cycle.",
    },
    Ugen {
        name: "poly_saw",
        kind: NodeKind::PolySaw,
        params: &["frequency"],
        doc: "PolyBLEP saw wave. Fast and fairly bandlimited.",
    },
    Ugen {
        name: "poly_square",
        kind: NodeKind::PolySquare,
        params: &["frequency"],
        doc: "PolyBLEP square wave. Fast and fairly bandlimited.",
    },
    Ugen {
        name: "pulse",
        kind: NodeKind::Pulse,
        params: &["frequency", "width"],
        doc: "Bandlimited pulse wave oscillator. `width` in 0..=1 is the duty cycle.",
    },
    Ugen {
        name: "ramp",
        kind: NodeKind::Ramp,
        params: &["frequency"],
        doc: "Rising ramp from 0 to 1 at the given repetition frequency. Not bandlimited — useful as a phasor, not as audio.",
    },
    Ugen {
        name: "resonator",
        kind: NodeKind::Resonator,
        params: &["audio", "frequency", "q"],
        doc: "Constant-gain bandpass resonator.",
    },
    // fundsp's reverbs are all stereo. scree's graph is mono end to end, so
    // each one is wrapped: the signal feeds both inputs and the two outputs are
    // averaged back down. They are wet-only — mix the dry signal yourself.
    Ugen {
        name: "reverb",
        kind: NodeKind::Reverb,
        params: &["audio", "room_size", "time", "damping"],
        doc: "Reverb (32-channel FDN). `room_size` is in meters (10 is an average room), `time` is the decay to -60 dB in seconds, `damping` in 0..=1 rolls off the highs. Wet only: `x + reverb(x, 10, 3, 0.5) * 0.2`. All parameters except the audio input are constants.",
    },
    Ugen {
        name: "reverb2",
        kind: NodeKind::Reverb2,
        params: &["audio", "room_size", "time", "diffusion", "modulation", "damping_cutoff"],
        doc: "Hybrid FDN reverb — richer and more expensive than `reverb`. `room_size` is in meters and clamps to 10..=30, `diffusion` in 0..=1 thickens the tail, `modulation` around 1 adds movement (higher goes audibly Doppler), and `damping_cutoff` is the lowpass applied to each loop pass, in hertz. Wet only. All parameters except the audio input are constants.",
    },
    Ugen {
        name: "reverb3",
        kind: NodeKind::Reverb3,
        params: &["audio", "time", "diffusion", "damping_cutoff"],
        doc: "Allpass-loop reverb, with no room size — just `time` to -60 dB, `diffusion` in 0..=1, and a `damping_cutoff` in hertz applied to each loop pass. Wet only. All parameters except the audio input are constants.",
    },
    Ugen {
        name: "reverb4",
        kind: NodeKind::Reverb4,
        params: &["audio", "room_size", "time"],
        doc: "Reverb with a slow fade-in, for swells rather than rooms. `room_size` is in meters and is treated as at least 15; below that the delay times stop sounding like a space. Wet only. Both `room_size` and `time` are constants.",
    },
    Ugen {
        name: "rossler",
        kind: NodeKind::Rossler,
        params: &["frequency"],
        doc: "Rossler chaotic oscillator, with peaks at multiples of the frequency input.",
    },
    Ugen {
        name: "saw",
        kind: NodeKind::Saw,
        params: &["frequency"],
        doc: "Bandlimited saw wavetable oscillator.",
    },
    Ugen {
        name: "sin",
        kind: NodeKind::Sin,
        params: &["frequency"],
        doc: "Sine oscillator.",
    },
    Ugen {
        name: "soft_saw",
        kind: NodeKind::SoftSaw,
        params: &["frequency"],
        doc: "Soft saw wavetable oscillator. Contains all partials but falls off like a triangle wave.",
    },
    Ugen {
        name: "square",
        kind: NodeKind::Square,
        params: &["frequency"],
        doc: "Bandlimited square wavetable oscillator.",
    },
    Ugen {
        name: "tap",
        kind: NodeKind::Tap,
        params: &["signal", "delay", "min_delay", "max_delay"],
        doc: "Tapped delay line with cubic interpolation. Unlike `delay`, the delay time is a signal, so it can be modulated — it must stay within the constant `min_delay`..=`max_delay` bounds, in seconds.",
    },
    Ugen {
        name: "tick",
        kind: NodeKind::Tick,
        params: &["signal"],
        doc: "Single-sample delay. The building block for feedback and comb filters.",
    },
    Ugen {
        name: "triangle",
        kind: NodeKind::Triangle,
        params: &["frequency"],
        doc: "Bandlimited triangle wavetable oscillator.",
    },
];

pub static LIST_BUILTINS: &[ListBuiltin] = &[
    ListBuiltin {
        name: "len",
        params: &["list"],
        arities: &[1],
        variadic: false,
        doc: "The number of elements in the list.",
    },
    ListBuiltin {
        name: "zip",
        params: &["list"],
        arities: &[1],
        variadic: true,
        doc: "Pair elements positionally: `zip([1, 2], [3, 4])` is `[[1, 3], [2, 4]]`. Every argument must be a list, and all must be the same length.",
    },
    ListBuiltin {
        name: "rev",
        params: &["list"],
        arities: &[1],
        variadic: false,
        doc: "The list, back to front.",
    },
    ListBuiltin {
        name: "palindrome",
        params: &["list"],
        arities: &[1],
        variadic: false,
        doc: "The list followed by its mirror: `[a, b, c]` becomes `[a, b, c, c, b, a]`.",
    },
    ListBuiltin {
        name: "rotl",
        params: &["list", "amount"],
        arities: &[1, 2],
        variadic: false,
        doc: "Rotate left, wrapping. `amount` defaults to 1; a negative amount rotates right.",
    },
    ListBuiltin {
        name: "rotr",
        params: &["list", "amount"],
        arities: &[1, 2],
        variadic: false,
        doc: "Rotate right, wrapping. `amount` defaults to 1; a negative amount rotates left.",
    },
    ListBuiltin {
        name: "push",
        params: &["list", "value"],
        arities: &[2],
        variadic: false,
        doc: "A new list with `value` appended. Lists are immutable — the original is unchanged.",
    },
    ListBuiltin {
        name: "pop",
        params: &["list"],
        arities: &[1],
        variadic: false,
        doc: "A new list without its last element. Index the list to read that element.",
    },
    ListBuiltin {
        name: "sort",
        params: &["list"],
        arities: &[1],
        variadic: false,
        doc: "The list sorted ascending. Every element must be a compile-time number.",
    },
    ListBuiltin {
        name: "sum",
        params: &["list"],
        arities: &[1],
        variadic: false,
        doc: "Add every element together. Folds numbers at compile time and emits mixing nodes for signals, so a list of oscillators sums into the graph.",
    },
    ListBuiltin {
        name: "split",
        params: &["list", "size"],
        arities: &[2],
        variadic: false,
        doc: "Break the list into chunks of `size`. A short final chunk is kept.",
    },
    ListBuiltin {
        name: "choice",
        params: &["list"],
        arities: &[1],
        variadic: false,
        doc: "One element picked at random. Re-rolled on every eval.",
    },
    ListBuiltin {
        name: "wchoice",
        params: &["values", "weights"],
        arities: &[2],
        variadic: false,
        doc: "One element picked at random, weighted. The two lists run in parallel; weights must be finite and >= 0.",
    },
    ListBuiltin {
        name: "scramble",
        params: &["list"],
        arities: &[1],
        variadic: false,
        doc: "The list shuffled. Re-rolled on every eval.",
    },
    ListBuiltin {
        name: "filter",
        params: &["list", "predicate"],
        arities: &[2],
        variadic: false,
        doc: "Keep the elements the predicate answers non-zero for. The predicate is an ordinary user `fn` of one argument.",
    },
];

/// Compile-time arithmetic on numbers. Shares `ListBuiltin`'s shape because
/// the two are the same kind of thing — a name evaluated during lowering that
/// emits no node — and they differ only in what they accept.
///
/// These read naturally through the dot operator: `60.m2h`.
pub static MATH_BUILTINS: &[ListBuiltin] = &[
    // --- conversions ---
    ListBuiltin {
        name: "m2h",
        params: &["note"],
        arities: &[1],
        variadic: false,
        doc: "MIDI note number to frequency in hertz: `69.m2h` is 440. Equal temperament, A4 = 440 Hz.",
    },
    ListBuiltin {
        name: "h2m",
        params: &["hz"],
        arities: &[1],
        variadic: false,
        doc: "Frequency in hertz to MIDI note number, the inverse of `m2h`: `440.h2m` is 69. The result may be fractional.",
    },
    ListBuiltin {
        name: "db",
        params: &["decibels"],
        arities: &[1],
        variadic: false,
        doc: "Decibels to a linear amplitude: `0.db` is 1, `-6.db` is about 0.5. Use it to write gains the way you hear them.",
    },
    ListBuiltin {
        name: "amp",
        params: &["amplitude"],
        arities: &[1],
        variadic: false,
        doc: "Linear amplitude to decibels, the inverse of `db`: `1.amp` is 0. The amplitude must be above zero.",
    },
    ListBuiltin {
        name: "cents",
        params: &["hz", "cents"],
        arities: &[2],
        variadic: false,
        doc: "Detune a frequency by cents, 1/100 of a semitone: `440.cents(-14)` flattens A4 slightly.",
    },
    ListBuiltin {
        name: "bpm",
        params: &["beats"],
        arities: &[1],
        variadic: false,
        doc: "Beats per minute to cycles per second, taking one cycle as four beats: `120.bpm` is 0.5, which is the default tempo.",
    },
    // --- pitch arithmetic ---
    ListBuiltin {
        name: "oct",
        params: &["note", "octaves"],
        arities: &[2],
        variadic: false,
        doc: "Transpose a MIDI note by whole octaves: `60.oct(-1)` is 48.",
    },
    ListBuiltin {
        name: "semi",
        params: &["note", "semitones"],
        arities: &[2],
        variadic: false,
        doc: "Transpose a MIDI note by semitones: `60.semi(7)` is 67.",
    },
    ListBuiltin {
        name: "scale",
        params: &["note", "scale"],
        arities: &[2],
        variadic: false,
        doc: "Snap a MIDI note to the nearest tone of a scale, given as semitone offsets within an octave: `61.scale([0, 2, 4, 5, 7, 9, 11])` is 60. Ties snap down.",
    },
    // --- ranges and shaping ---
    ListBuiltin {
        name: "clamp",
        params: &["x", "lo", "hi"],
        arities: &[3],
        variadic: false,
        doc: "Constrain a number to `lo..=hi`.",
    },
    ListBuiltin {
        name: "norm",
        params: &["x", "lo", "hi"],
        arities: &[3],
        variadic: false,
        doc: "Map 0..1 onto `lo..hi`. Values outside 0..1 extrapolate; `clamp` first if you do not want that.",
    },
    ListBuiltin {
        name: "wrap",
        params: &["x", "lo", "hi"],
        arities: &[3],
        variadic: false,
        doc: "Fold a number back into `lo..hi`, wrapping around rather than clamping. Useful for modular pitch.",
    },
    ListBuiltin {
        name: "round",
        params: &["x"],
        arities: &[1],
        variadic: false,
        doc: "Nearest whole number, halves away from zero.",
    },
    ListBuiltin {
        name: "floor",
        params: &["x"],
        arities: &[1],
        variadic: false,
        doc: "Largest whole number at or below `x`.",
    },
    ListBuiltin {
        name: "ceil",
        params: &["x"],
        arities: &[1],
        variadic: false,
        doc: "Smallest whole number at or above `x`.",
    },
    ListBuiltin {
        name: "abs",
        params: &["x"],
        arities: &[1],
        variadic: false,
        doc: "Magnitude, dropping the sign.",
    },
    ListBuiltin {
        name: "pow",
        params: &["x", "exponent"],
        arities: &[2],
        variadic: false,
        doc: "`x` raised to a power: `2.pow(10)` is 1024. Good for exponential curve shaping.",
    },
    ListBuiltin {
        name: "sqrt",
        params: &["x"],
        arities: &[1],
        variadic: false,
        doc: "Square root. `x` must not be negative.",
    },
    ListBuiltin {
        name: "log2",
        params: &["x"],
        arities: &[1],
        variadic: false,
        doc: "Base-2 logarithm. `x` must be above zero.",
    },
];

/// Names that exist but belong to neither table: `play` is intercepted before
/// evaluation because it needs its instrument argument syntactically, and `dur`
/// is not a function at all — it is the note length, bound only inside a voice.
pub static SPECIALS: &[ListBuiltin] = &[
    ListBuiltin {
        name: "then",
        params: &["play", "section"],
        arities: &[2],
        variadic: false,
        doc: "Sequence one section after another: `playn(verse, lead, 4).then(chorus)`. The left side must be `play_once` or `playn` — plain `play` never finishes. `section` is a no-parameter `fn` whose own `play` calls start where this one stops; it is inlined at eval time, not called by the audio thread.",
    },
    ListBuiltin {
        name: "play",
        params: &["pattern", "instrument", "rate"],
        arities: &[2, 3],
        variadic: false,
        doc: "Schedule a pattern on an instrument: `pat >> play(kick)`. The instrument must name a user `fn`. `rate` defaults to 1. Any further parameter is patterned by name — `play(bass, cut: [400, 2000])` — sampled at each note's onset, and lanes may be any length. `legato:` scales the note's length instead of being passed.",
    },
    ListBuiltin {
        name: "play_once",
        params: &["pattern", "instrument", "rate"],
        arities: &[2, 3],
        variadic: false,
        doc: "`play`, stopping after one pass of the pattern: `[60, 64, 67] >> play_once(stab)`. Started while something is already playing it begins on the next cycle, so the one-shot lands on a downbeat. Re-evaluating fires it again.",
    },
    ListBuiltin {
        name: "playn",
        params: &["pattern", "instrument", "times", "rate"],
        arities: &[3, 4],
        variadic: false,
        doc: "`play`, stopping after `times` passes of the pattern: `playn([220, 330], bass, 4)`. `rate` follows the count and still defaults to 1 — at rate 2 the four passes take two cycles. Lanes work as they do on `play`.",
    },
    ListBuiltin {
        name: "play_all",
        params: &["play"],
        arities: &[1],
        variadic: true,
        doc: "Treat several plays that run at once as one section: `play_all(playn(verse, lead, 4), playn(bassline, bass, 4)).then(chorus)`. Every argument must be a `play_once`, `playn`, `play`, or another `play_all` — they all start together, and the group finishes when the last of them does. A plain `play` among them never finishes, so nothing may follow.",
    },
    ListBuiltin {
        name: "dur",
        params: &[],
        arities: &[],
        variadic: false,
        doc: "The current note's length in seconds. Bound only inside a voice — pass it to `env`.",
    },
];

/// Look up a UGen by name. `lowerer::call` reads arity from `params.len()`, so
/// this table is the single definition of both.
pub fn ugen(name: &str) -> Option<&'static Ugen> {
    UGENS.iter().find(|u| u.name == name)
}

/// Look up a list builtin by name. `lowerer::lists` reads its allowed arities
/// from here.
pub fn list_builtin(name: &str) -> Option<&'static ListBuiltin> {
    LIST_BUILTINS.iter().find(|b| b.name == name)
}

/// Look up a math builtin by name. `lowerer::math` reads its arities from here.
pub fn math_builtin(name: &str) -> Option<&'static ListBuiltin> {
    MATH_BUILTINS.iter().find(|b| b.name == name)
}

// ---------------------------------------------------------------------------
// Note names.
// ---------------------------------------------------------------------------

/// Semitone offset of each natural note from C.
const NOTE_OFFSETS: [(u8, i32); 7] = [
    (b'c', 0),
    (b'd', 2),
    (b'e', 4),
    (b'f', 5),
    (b'g', 7),
    (b'a', 9),
    (b'b', 11),
];

/// `c0` is MIDI 12 and `g9` is 127, the top of the MIDI range.
pub const MIN_NOTE_OCTAVE: i32 = 0;
pub const MAX_NOTE_OCTAVE: i32 = 9;

pub enum NoteName {
    /// A MIDI note number.
    Note(f64),
    /// Shaped like a note, but the octave is outside the supported range.
    OctaveOutOfRange(i32),
    NotANote,
}

/// Read a bare identifier as a note name: letter, optional `s`/`f`, octave.
///
/// Deliberately **not** seeded into the environment. Resolution happens only
/// when a variable lookup misses, so user bindings shadow note names for free
/// and `lower_voice` — which runs per note — pays nothing to set them up.
///
/// The octave is required, which is what keeps `f`, `a` and `e` usable as
/// ordinary parameter names. Flats are `f` rather than `b`, both because `b`
/// is itself a note and because `db3` would read against the `db` builtin.
/// Enharmonics are allowed: `bs3` is `c4`, `cf4` is `b3`.
pub fn note(name: &str) -> NoteName {
    let Some(&letter) = name.as_bytes().first() else {
        return NoteName::NotANote;
    };
    let Some((_, offset)) = NOTE_OFFSETS.iter().find(|(c, _)| *c == letter) else {
        return NoteName::NotANote;
    };

    let rest = &name[1..];
    let (accidental, digits) = if let Some(d) = rest.strip_prefix('s') {
        (1, d)
    } else if let Some(d) = rest.strip_prefix('f') {
        (-1, d)
    } else {
        (0, rest)
    };

    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return NoteName::NotANote;
    }
    let Ok(octave) = digits.parse::<i32>() else {
        return NoteName::NotANote;
    };
    if !(MIN_NOTE_OCTAVE..=MAX_NOTE_OCTAVE).contains(&octave) {
        return NoteName::OctaveOutOfRange(octave);
    }

    NoteName::Note(((octave + 1) * 12 + offset + accidental) as f64)
}

// ---------------------------------------------------------------------------
// The editor-facing view.
// ---------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuiltinInfo {
    pub name: &'static str,
    pub params: &'static [&'static str],
    /// Every accepted argument count. A UGen has exactly one.
    pub arities: Vec<usize>,
    /// True when any count above the largest listed arity is also accepted.
    pub variadic: bool,
    /// `"ugen"`, `"list"` or `"special"` — the editor colours and ranks by this.
    pub category: &'static str,
    pub doc: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LanguageMetadata {
    pub builtins: Vec<BuiltinInfo>,
    pub keywords: &'static [&'static str],
}

/// Everything the editor needs to highlight, complete and describe scree.
pub fn metadata() -> LanguageMetadata {
    let ugens = UGENS.iter().map(|u| BuiltinInfo {
        name: u.name,
        params: u.params,
        arities: vec![u.params.len()],
        variadic: false,
        category: "ugen",
        doc: u.doc,
    });

    let lists = LIST_BUILTINS.iter().map(|b| BuiltinInfo {
        name: b.name,
        params: b.params,
        arities: b.arities.to_vec(),
        variadic: b.variadic,
        category: "list",
        doc: b.doc,
    });

    let maths = MATH_BUILTINS.iter().map(|b| BuiltinInfo {
        name: b.name,
        params: b.params,
        arities: b.arities.to_vec(),
        variadic: b.variadic,
        category: "math",
        doc: b.doc,
    });

    let specials = SPECIALS.iter().map(|b| BuiltinInfo {
        name: b.name,
        params: b.params,
        arities: b.arities.to_vec(),
        variadic: b.variadic,
        category: "special",
        doc: b.doc,
    });

    LanguageMetadata {
        builtins: ugens.chain(lists).chain(maths).chain(specials).collect(),
        keywords: KEYWORDS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// Every name is unique across all three tables — a duplicate would make
    /// one entry unreachable, since `call_with` tries the tables in order.
    #[test]
    fn names_are_unique_and_non_empty() {
        let mut seen = HashSet::new();
        for name in UGENS
            .iter()
            .map(|u| u.name)
            .chain(LIST_BUILTINS.iter().map(|b| b.name))
            .chain(SPECIALS.iter().map(|b| b.name))
        {
            assert!(!name.is_empty(), "a builtin has an empty name");
            assert!(seen.insert(name), "{name} is defined twice");
        }
    }

    /// Guards the arity refactor: `lowerer::call` derives each UGen's arity
    /// from `params.len()`, so a short parameter list would silently loosen an
    /// arity check. These are the counts the old match arm enforced.
    #[test]
    fn ugen_arities_match_the_original_table() {
        let expected: &[(&str, usize)] = &[
            ("adsr", 5), ("afollow", 3), ("allpass", 3), ("allpole", 2),
            ("bandpass", 3), ("bandrez", 3), ("bell", 4), ("biquad", 6),
            ("brown", 0), ("butterpass", 2), ("chorus", 5), ("clip", 1),
            ("clip_to", 3), ("dcblock", 1), ("declick", 1), ("delay", 2),
            ("dsf_saw", 2), ("env", 5), ("dsf_square", 2), ("fir3", 2),
            ("follow", 2), ("hammond", 1), ("highpass", 3), ("highpole", 2),
            ("highshelf", 4), ("hold", 3), ("impulse", 0), ("limiter", 3),
            ("lorenz", 1), ("lowpass", 3), ("lowpole", 2), ("lowrez", 3),
            ("lowshelf", 4), ("mls", 0), ("mls_bits", 1), ("moog", 3),
            ("morph", 4), ("noise", 0), ("notch", 3), ("organ", 1),
            ("peak", 3), ("perc", 2), ("pink", 0), ("pinkpass", 1),
            ("pluck", 4), ("poly_pulse", 2), ("poly_saw", 1), ("poly_square", 1),
            ("pulse", 2), ("ramp", 1), ("resonator", 3),
            ("reverb", 4), ("reverb2", 6), ("reverb3", 4), ("reverb4", 3),
            ("rossler", 1),
            ("saw", 1), ("sin", 1), ("soft_saw", 1), ("square", 1),
            ("tap", 4), ("tick", 1), ("triangle", 1),
        ];

        assert_eq!(UGENS.len(), expected.len(), "a UGen was added or removed");
        for (name, arity) in expected {
            let u = ugen(name).unwrap_or_else(|| panic!("{name} is missing from UGENS"));
            assert_eq!(
                u.params.len(),
                *arity,
                "{name} takes {arity} arguments but has {} parameter names",
                u.params.len()
            );
        }
    }

    /// `dur` aside, every special is one of the `play` family — and the lowerer
    /// has to intercept exactly those. A name in the table the lowerer does not
    /// know is completable but not callable; one it knows that is missing here
    /// is callable but invisible to the editor.
    #[test]
    fn the_table_and_the_lowerer_agree_on_the_play_family() {
        use crate::lowerer::lower::Lowerer;
        for b in SPECIALS {
            // `dur` is a bound value rather than a call; `then` and `play_all`
            // are intercepted on their own paths — everything else is a `play`.
            let intercepted = Lowerer::is_play(b.name)
                || Lowerer::is_then(b.name)
                || Lowerer::is_play_all(b.name);
            assert_eq!(
                intercepted,
                b.name != "dur",
                "{} is intercepted by one of the two and not the other",
                b.name,
            );
        }
    }

    /// The realizer reads inputs positionally, so a UGen's parameter names must
    /// line up one-to-one with its inputs — no duplicates, none blank.
    #[test]
    fn ugen_parameter_names_are_distinct() {
        for u in UGENS {
            let unique: HashSet<_> = u.params.iter().collect();
            assert_eq!(unique.len(), u.params.len(), "{} repeats a parameter name", u.name);
            assert!(u.params.iter().all(|p| !p.is_empty()), "{} has a blank parameter", u.name);
        }
    }

    /// A list builtin's parameter names must cover its largest arity, or
    /// signature help would run out of names mid-call.
    #[test]
    fn list_builtin_params_cover_their_arities() {
        for b in LIST_BUILTINS.iter().chain(SPECIALS) {
            assert!(!b.arities.is_empty() || b.params.is_empty(), "{}", b.name);
            if let Some(max) = b.arities.iter().max() {
                assert!(
                    b.params.len() >= *max,
                    "{} accepts {max} arguments but names only {} parameters",
                    b.name,
                    b.params.len()
                );
            }
        }
    }

    /// The wire shape the `Builtin` interface in `src/scree/metadata.ts` is
    /// written against. A rename here is a silent breakage over there.
    #[test]
    fn metadata_serializes_to_the_shape_the_editor_expects() {
        let json = serde_json::to_value(metadata()).unwrap();

        let keywords = json["keywords"].as_array().unwrap();
        assert!(keywords.iter().any(|k| k == "fn"));

        let builtins = json["builtins"].as_array().unwrap();
        assert_eq!(
            builtins.len(),
            UGENS.len() + LIST_BUILTINS.len() + MATH_BUILTINS.len() + SPECIALS.len()
        );

        let m2h = builtins
            .iter()
            .find(|b| b["name"] == "m2h")
            .expect("m2h is missing");
        assert_eq!(m2h["category"], "math");

        let lowpass = builtins
            .iter()
            .find(|b| b["name"] == "lowpass")
            .expect("lowpass is missing");
        assert_eq!(lowpass["params"], serde_json::json!(["audio", "cutoff", "q"]));
        assert_eq!(lowpass["arities"], serde_json::json!([3]));
        assert_eq!(lowpass["variadic"], serde_json::json!(false));
        assert_eq!(lowpass["category"], "ugen");
        assert!(lowpass["doc"].as_str().unwrap().len() > 10);

        // The two shapes that are not a plain fixed arity.
        let rotl = builtins.iter().find(|b| b["name"] == "rotl").unwrap();
        assert_eq!(rotl["arities"], serde_json::json!([1, 2]));
        let zip = builtins.iter().find(|b| b["name"] == "zip").unwrap();
        assert_eq!(zip["variadic"], serde_json::json!(true));
    }

    /// Everything is documented. Empty docs render as a blank completion panel.
    #[test]
    fn everything_has_a_doc() {
        for (name, doc) in UGENS
            .iter()
            .map(|u| (u.name, u.doc))
            .chain(LIST_BUILTINS.iter().map(|b| (b.name, b.doc)))
            .chain(SPECIALS.iter().map(|b| (b.name, b.doc)))
        {
            assert!(!doc.trim().is_empty(), "{name} has no documentation");
        }
    }
}

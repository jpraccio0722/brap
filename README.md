# Scree

A live coding language and environment, written in Rust, wrapping much of
[fundsp's](https://github.com/SamiPerttu/fundsp) functionality. 

```bash
npm install && npm run tauri dev
```

`Cmd/Ctrl + ,` evaluates the current tab

`Cmd/Ctrl + .` stops audio. 

## Step by step tutorial


## Imports

A file can use another file's definitions. The spelling is Rust's:

```rust
use kit                      // the module, as kit::kick, kit::snare
use kit as k                 // the same, as k::kick
use kit::kick                // one name, on its own
use kit::{kick, hat as tick} // several, renamed where you like
use kit::*                   // everything the file defines
```

**A `use` does not run the file it names.** Its `fn`s and `let`s come across;
Top level expressions and `play`s in the imported file do not run.

Imports resolve relative to the file on disk, and read what is saved: save a
module before playing a file that uses it.

Everything else follows from names. An imported `fn` is an instrument like any
other, qualified or not:

```rust
play([\, `, \, `], kit::kick)
```


## Drawn patterns

The right-hand panel draws patterns on a grid: click a cell to walk it through
rest → trigger → pitch. Each row has a name, and that name is what the editor
plays:

```rust
play(hats, hat)
```

Rows are saved with the project, in a `patterns.scree` beside your files:

```rust
let hats = [\, `, \, `]
let riff = [c4, e4, `, g4]
```

That file is folded into every program the project runs, so a drawn pattern
needs no `use` — it is simply in scope.

Being a real file, it goes both ways: open it in a tab, edit it by hand, save,
and the panel redraws from what you wrote. Draw in the panel and the file is
rewritten. Anything else you keep in that file is lost the next time a row
changes.


## Samples

`load` reads an audio file into a buffer. The path is relative to the file it is
written in, exactly like a `use` path, and any format symphonia reads will do —
wav, mp3, flac, ogg:

```rust
let amen = load("breaks/amen.wav")
```

A buffer is not audio. Nothing comes out of it until `sample` reads it **at a
position**, where 0 is the start of the buffer and 1 is the end:

```rust
sample(amen, ramp(1 / amen.secs))       // forwards, at its own speed
```

That is the whole interface. There is no play, no stop, no rate and no reverse,
because a position is a signal and everything those would do is arithmetic on
it:

```rust
sample(amen, 1 - ramp(1 / amen.secs))       // backwards
sample(amen, ramp(2 / amen.secs))           // twice as fast, an octave up
sample(amen, ramp(0.5 / amen.secs))         // half speed
sample(amen, ramp(4 / amen.secs) * 0.25)    // the first quarter, looping four times a pass
sample(amen, 0.5 + ramp(4 / amen.secs) * 0.25)  // the third quarter
sample(amen, ramp(1 / amen.secs) >> hold(16, 0))  // stuttered into sixteen steps
```

`ramp(f)` is the phasor: a rising 0 to 1, `f` times a second. `secs` is how long
the buffer is, so `1 / amen.secs` is the frequency that reads it exactly once —
and any multiple of that is a speed. Reading outside 0..1 is silence rather than
a held edge, so a position that overshoots goes quiet instead of clicking.

Chopping from a pattern is the same arithmetic with the slice as a lane:

```rust
// Sixteenths of the break, played as a pattern. `at` is where in the buffer
// this note starts; the phasor covers one sixteenth from there.
fn chop(n, at = 0) =
  sample(load("breaks/amen.wav"), at + ramp(n) * 0.0625) * perc(0.001, 0.2)

play([\, \, \, \, \, \, \, \], chop, 1,
     at: [0, 0.25, 0.5, 0.0625, 0.75, 0.5, 0.125, 0.875])
```

**An instrument names its own file.** A `fn` sees only other `fn`s — a top-level
`let` is the persistent graph's, not a voice's — so `let amen = load(...)` above
a `fn` that reads `amen` will not compile. Write the `load` inside the
instrument, as above. It costs nothing: one path is one buffer however many
`load`s name it, so the inline spelling shares the same audio as everything
else that reads that file.

Three more things worth knowing:

- **Every file is decoded once, before the program runs.** A path is found by
  reading the program, not by running it, which is why it has to be written out
  rather than assembled — `load(name)` will not compile. It also means no note
  ever waits on a disk, and re-evaluating costs nothing however long the file
  is.
- **A buffer is stored once** however many times it is read, so chopping a break
  sixteen ways is sixteen readers over one copy of the audio.
- **Reading interpolates** (cubic), so a break holds up away from its own speed
  rather than aliasing.

`channels` says how many channels a file has, and `sample`'s optional third
argument picks one — it defaults to 0 and wraps, so asking a mono file for
channel 1 gives you the mono back rather than an error:

```rust
let stereo = load("pad.wav")
let pos = ramp(1 / stereo.secs)

// Both channels of a stereo file, summed. An instrument is mono, so this is
// where a stereo file stops being stereo — use `pan:` on the `play` to place
// the voice.
(sample(stereo, pos, 0) + sample(stereo, pos, 1)) * 0.5
```


## Function Reference

- [Patterns and Playback](#patterns-and-playback)
- [Samples](#sample-functions)
- [List Functions](#list-functions)
- [Math Functions](#math-functions)
- [Random Numbers](#random-numbers)
- [Oscillators and Sources](#oscillators-and-sources)
- [Noise and Chaos](#noise-and-chaos)
- [Filters](#filters)
- [Envelopes and Dynamics](#envelopes-and-dynamics)
- [Delays and Effects](#delays-and-effects)

Every name here takes its first argument on the left of a dot, so `f(a, b)`,
`a >> f(b)` and `a.f(b)` are three spellings of one call. The editor offers only
the names that suit whatever is in front of the dot.

The notes below are the same ones the editor shows, and are generated from the
same tables. Hold ⌘ and point at a name already written in a file to read its
signature and note where it stands; ⌘-click it to open the whole reference,
searchable, in the right-hand panel.

In the UGen tables an **`→` separates ports from constants**. Everything to its
left is a wired input and may be modulated by another signal; everything to its
right is baked in when the graph is built and must be a compile-time number. So
`tap(signal, delay → min_delay, max_delay)` follows a moving `delay`, while
`delay(signal → time)` will not.

### Patterns and Playback

These are what turn a list into sound. `play` and its variants take the pattern
first, so they chain off one like anything else: `riff.rev.play(bass)`.

An instrument is written in mono. Where it sits is `play`'s business, not the
instrument's, so `pan:` is a lane like any other — read a value per note,
wrapping when it runs out, and free to be a different length from the pattern:

```rust
play([\, `, \, `, \, `, \, `], hat, pan: [-0.8, 0.8])   // alternating, note by note
play([c2, e2, g2], bass, pan: 0)                        // pinned to the centre
```

-1 is hard left, 0 the centre, 1 hard right; anything further clamps. The law is
equal-power, scaled so that the centre is where an unpanned voice already sat —
adding `pan: 0` to a line that was playing changes nothing about how loud it is.
The 3 dB that buys is paid at the extremes instead, where one channel is silent
and the other is a little louder than the instrument wrote, so a hard-panned
voice is worth checking against a limiter.

A pan is sampled once, at the note's onset, and holds for that note. Sweeping
one *within* a note is a different thing and is not in the language yet: the
signal graph an instrument builds is mono throughout, and stereo begins after
it.

| Name | Signature | Notes |
| --- | --- | --- |
| `play` | `play(pattern, instrument, rate?)` | Schedule a pattern on an instrument, forever. The instrument must name a user `fn`. `rate` defaults to 1. A list divides the cycle evenly unless a step is given a length with `;` — ``[220;2, 330, 440, `;4]`` is a quarter, two eighths and a half of silence — and lengths are relative, so only their ratio matters. A long step is one sustained note, not several. Any further parameter is patterned by name — `play(bass, cut: [400, 2000])` — sampled at each note's onset, and lanes may be any length. In a lane a `;` is how many notes the value covers, so it has to be a whole number there. Two names are reserved and reach the note rather than the instrument: `legato:` scales its length, and `pan:` places it across the stereo field. |
| `play_once` | `play_once(pattern, instrument, rate?)` | `play`, stopping after one pass. Started while something is already playing, it begins on the next cycle, so the one-shot lands on a downbeat. Re-evaluating fires it again. |
| `playn` | `playn(pattern, instrument, times, rate?)` | `play`, stopping after `times` passes. `rate` follows the count and still defaults to 1 — at rate 2, four passes take two cycles. |
| `play_all` | `play_all(play, ...)` | Treat several plays that run at once as one section. Every argument must be a `play`, `play_once`, `playn` or another `play_all`; they start together and the group finishes when the last does. A plain `play` among them never finishes, so nothing may follow. |
| `then` | `then(play, section)` | Sequence one section after another: `playn(verse, lead, 4).then(chorus)`. The left side must finish, so plain `play` will not do. `section` is a no-parameter `fn` whose own `play` calls start where this one stops; it is inlined at eval time, not called from the audio thread. |
| `dur` | `dur` | The current note's length in seconds. A binding rather than a function, and bound only inside a voice — pass it to `env`. |

### Sample Functions

Reading an audio file. See [Samples](#samples) above for what these are for; the
table is the signatures.

| Name | Signature | Notes |
| --- | --- | --- |
| `load` | `load(path) -> buffer` | Read an audio file into a buffer. The path is relative to the file it is written in, the same way a `use` path is, and must be written out rather than computed — every file is decoded once, before the program runs, so no note ever waits on a disk. Any format symphonia reads: wav, mp3, flac, ogg. Nothing comes out of a buffer until `sample` reads it. |
| `sample` | `sample(buffer, position, channel?) -> signal` | Read a buffer at a position: 0 is the start, 1 is the end, and anything outside that is silence. `position` is a signal, which is where speed, direction and chopping all come from. Cubic interpolation, so it holds up away from its own speed. `channel` defaults to 0 and wraps if the buffer has fewer; it picks which reader is built, so it must be a compile-time number. |
| `secs` | `secs(buffer) -> number` | How long a buffer is, in seconds. A compile-time number, so it divides into a `ramp` frequency: `ramp(1 / amen.secs)` reads the whole buffer once at its own speed. |
| `channels` | `channels(buffer) -> number` | How many channels a buffer has — 1 for mono, 2 for a stereo file. |

### List Functions

```rust
let riff = [60, 63, 67]
play(riff.rev.rotl(2), bass)
play(riff.push(72).scramble, bass)
riff.rev.play(bass)             // `play` takes a pattern, so it chains too
```

Lists are immutable. List functions generate a new list and leave the existing list intact.

| Name | Signature | Notes |
| --- | --- | --- |
| `len` | `len(list) -> number` | Works on list literals and ranges. Errors on non-lists and on wrong arity. |
| `zip` | `zip(list, ...) -> list` | Variadic. Pairs positionally into rows `[[a0,b0], [a1,b1], …]`. **All arguments must be lists of equal length** — a mismatch is an error, not a silent truncation. Rows carry whatever `Value`s went in, including signals. |
| `rev` | `rev(list) -> list` | Reversed. |
| `palindrome` | `palindrome(list) -> list` | The sequence then its mirror: `[1,2,3]` → `[1,2,3,3,2,1]`. |
| `rotl` | `rotl(list, amount?) -> list` | Rotate left, wrapping. `amount` defaults to 1, and a negative amount rotates the other way — `rotl(l, -1)` is `rotr(l, 1)`. Rotating by the length is the identity. An empty list is returned unchanged. |
| `rotr` | `rotr(list, amount?) -> list` | Rotate right, wrapping. The mirror of `rotl` in every respect. |
| `push` | `push(list, value) -> list` | Appends. Returns a new list; the original is untouched. |
| `pop` | `pop(list) -> list` | Drops the **last** element. Nothing is returned "off the top" — index the list for that. Errors on an empty list. |
| `sort` | `sort(list) -> list` | Ascending. Every element must be a compile-time number. |
| `sum` | `sum(list) -> value` | Folds through the same `combine` the arithmetic operators use, so numbers fold to a constant and **signals emit `Add` nodes** — `sum([sin(110), sin(220)])` is additive synthesis without a `for`. Empty list → `0`. |
| `split` | `split(list, size) -> list` | Chunks of `size` (not split-at-index). A short final chunk is kept. `size` must be a whole number ≥ 1. |
| `map` | `map(list, transform) -> list` | The function applied to every element. It is an ordinary user `fn` of one argument and may answer with anything, so `map` is also the only way to build a **list of signals** — a `for` over audio sums instead of collecting. |
| `filter` | `filter(list, predicate) -> list` | Keeps elements the predicate answers non-zero for. The predicate is an ordinary user `fn`; it must return a compile-time number. |
| `choice` | `choice(list) -> value` | One random element. Errors on an empty list. |
| `wchoice` | `wchoice(values, weights) -> value` | Weighted random pick. Parallel lists of equal length, like `zip`. Weights must be finite and ≥ 0, and not all zero. |
| `scramble` | `scramble(list) -> list` | Shuffled. |

The last three re-roll on every eval, and draw from the same generator as
[the random numbers](#random-numbers) — so `seed` pins them too.

### Math Functions
| Name | Signature | Notes |
| --- | --- | --- |
| `m2h` | `m2h(note) -> number` | MIDI note to hertz. `69.m2h` is 440, `60.m2h` is 261.63. Equal temperament, A4 = 440. Fractional notes work — that is how you get glides. |
| `h2m` | `h2m(hz) -> number` | The inverse. `440.h2m` is 69; the result may be fractional. Frequency must be above zero. |
| `db` | `db(decibels) -> number` | Decibels to linear amplitude. `0.db` is 1, `(-6).db` is about 0.5. |
| `amp` | `amp(amplitude) -> number` | The inverse. `1.amp` is 0. Amplitude must be above zero. |
| `cents` | `cents(hz, cents) -> number` | Detune a frequency by hundredths of a semitone. `440.cents(1200)` is 880. |
| `bpm` | `bpm(beats) -> number` | Beats per minute to cycles per second, **taking one cycle as four beats**. `120.bpm` is 0.5 — exactly `DEFAULT_CPS`. |
| `oct` | `oct(note, octaves) -> number` | Transpose by whole octaves. `60.oct(-1)` is 48. |
| `semi` | `semi(note, semitones) -> number` | Transpose by semitones. `60.semi(7)` is 67. |
| `scale` | `scale(note, scale) -> number` | Snap to the **nearest** tone of a scale given as semitone offsets within an octave. `61.scale([0,2,4,5,7,9,11])` is 60. Neighbouring octaves are candidates, so 59 against `[0,4,7]` rises to 60 rather than falling a seventh. Ties snap down. |
| `clamp` | `clamp(x, lo, hi) -> number` | Constrain to `lo..=hi`. An empty range is an error. |
| `norm` | `norm(x, lo, hi) -> number` | Map 0..1 onto `lo..hi`. Values outside 0..1 **extrapolate** — `clamp` first if that is not what you want. |
| `wrap` | `wrap(x, lo, hi) -> number` | Fold back into the range rather than clamping. `13.wrap(0, 12)` is 1. Useful for modular pitch. |
| `round` | `round(x) -> number` | Nearest whole number, halves away from zero. |
| `floor` | `floor(x) -> number` | Round down. |
| `ceil` | `ceil(x) -> number` | Round up. |
| `abs` | `abs(x) -> number` | Magnitude, sign discarded. |
| `pow` | `pow(x, exponent) -> number` | `2.pow(10)` is 1024. For exponential curve shaping. |
| `sqrt` | `sqrt(x) -> number` | `x` must not be negative. |
| `log2` | `log2(x) -> number` | `x` must be above zero. |

### Random Numbers

```rust
play(randis(8, 60, 72), lead)          // eight notes, settled until you eval again
play(riff, bass, cut: rands(4, 400, 2000))
fn snare(n) = noise() * perc(0.001, 0.1) * rand(0.7, 1)   // a new draw per note
```

| Name | Signature | Notes |
| --- | --- | --- |
| `rand` | `rand(lo?, hi?) -> number` | A uniform number. `rand()` draws from 0..1; `rand(lo, hi)` — or `60.rand(72)` — draws from `lo..hi`, `hi` excluded. An empty range is an error. |
| `randi` | `randi(lo, hi) -> number` | A uniform whole number in `lo..hi`, `hi` excluded: `randi(60, 72)` is an octave of notes that never repeats the root. Both bounds must be whole. |
| `coin` | `coin(probability?) -> number` | 1 or 0 at odds you choose. `coin()` is even; `coin(0.25)` answers 1 one time in four. Multiply by it to drop a note. |
| `seed` | `seed(seed) -> number` | Fix every draw made after it, and answer with the seed. Any number will do — `seed(0.5)` is a seed like any other. |
| `gauss` | `gauss(mean?, deviation?) -> number` | Normally distributed: clustered around `mean`, two thirds within one `deviation`. Both default to the standard 0 and 1. **Unbounded** — `clamp` it if a stray value would hurt. |
| `humanize` | `humanize(x, amount) -> number` | `x` plus a normal draw of deviation `amount`, so most nudges are small and a few are not. `0.5.humanize(0.05)` is a velocity that no longer sounds typed in. |
| `expo` | `expo(mean?) -> number` | Exponential, above zero, averaging `mean` (default 1). Short values common, long ones rare — the shape of a wait between events. |
| `tri` | `tri(lo, hi, mode?) -> number` | Triangular over `lo..hi`, peaking at `mode` (the midpoint if omitted). Bounded like `rand` but with a centre. |
| `cauchy` | `cauchy(median?, spread?) -> number` | Heavy-tailed around `median` (default 0). Mostly close in, but far likelier than `gauss` to lurch — which is the point. |
| `pareto` | `pareto(scale?, shape?) -> number` | Power-law at or above `scale` (default 1). A smaller `shape` (default 1) makes big values likelier. |
| `poisson` | `poisson(mean?) -> number` | A whole count averaging `mean` (default 1) — how many things happened, when each was independent. |
| `rands` | `rands(count, lo?, hi?) -> list` | `count` uniform numbers, from 0..1 or from `lo..hi`. A lane in one line. |
| `randis` | `randis(count, lo, hi) -> list` | `count` whole numbers in `lo..hi`, `hi` excluded. |
| `walk` | `walk(count, start, step) -> list` | A random walk: `count` numbers beginning at `start`, each drifting from the one before by up to `step` either way. **Neighbours stay close**, so it moves rather than jumps — which `rands` does not. Good for a cutoff or a pan. |
| `walki` | `walki(count, start, step) -> list` | `walk` in whole steps, so it stays on the semitone grid. `walki(16, 60, 2)` wanders a melody around middle C. `step` must be whole. |
| `choices` | `choices(list, count) -> list` | `count` elements without replacement, in a random order — `choice` several times over may repeat itself, and this cannot. Taking the whole list is `scramble`; asking for more than it holds is an error. |
| `randscale` | `randscale(count, scale, lo?, hi?) -> list` | `count` notes drawn **evenly** from the tones of a scale, given as semitone offsets within an octave, between MIDI `lo` and `hi` (60..72 by default). Unlike snapping a uniform draw with `scale`, every degree is equally likely and nothing lands outside the range. |

### Oscillators and Sources

| Name | Arguments | Notes |
| --- | --- | --- |
| `sin` | `(frequency)` | Sine oscillator. |
| `saw` | `(frequency)` | Bandlimited saw wavetable oscillator. |
| `square` | `(frequency)` | Bandlimited square wavetable oscillator. |
| `triangle` | `(frequency)` | Bandlimited triangle wavetable oscillator. |
| `soft_saw` | `(frequency)` | Soft saw wavetable oscillator. Contains all partials but falls off like a triangle wave. |
| `hammond` | `(frequency)` | Hammond organ wavetable oscillator. Emphasizes the first three partials. |
| `organ` | `(frequency)` | Organ wavetable oscillator. Emphasizes octave partials. |
| `ramp` | `(frequency)` | Rising ramp from 0 to 1 at the given repetition frequency, starting at 0. Not bandlimited — useful as a phasor, not as audio. Its zero is the start of the cycle, which is what lets it drive `sample`: `sample(b, ramp(1 / b.secs))` reads a buffer once, end to end. |
| `poly_saw` | `(frequency)` | PolyBLEP saw wave. Fast and fairly bandlimited. |
| `poly_square` | `(frequency)` | PolyBLEP square wave. Fast and fairly bandlimited. |
| `poly_pulse` | `(frequency, width)` | PolyBLEP pulse wave. Fast and fairly bandlimited; `width` in 0..=1 is the duty cycle. |
| `pulse` | `(frequency, width)` | Bandlimited pulse wave oscillator. `width` in 0..=1 is the duty cycle. |
| `dsf_saw` | `(frequency, roughness)` | Saw-like discrete summation formula oscillator. `roughness` in 0..=1 sets how much successive partials are attenuated. |
| `dsf_square` | `(frequency, roughness)` | Square-like discrete summation formula oscillator. `roughness` in 0..=1 sets how much successive partials are attenuated. |
| `impulse` | `()` | A single one followed by silence. Useful for exciting `pluck` or measuring an impulse response. |

### Noise and Chaos
| Name | Arguments | Notes |
| --- | --- | --- |
| `noise` | `()` | White noise. |
| `pink` | `()` | Pink noise: -3 dB per octave. |
| `brown` | `()` | Brown noise: -6 dB per octave. Darker than pink. |
| `mls` | `()` | Maximum length sequence noise: a repeating pseudorandom run of -1 and 1. |
| `mls_bits` | `(→ bits)` | Maximum length sequence noise from an n-bit sequence (1..=31). More bits means a longer period before it repeats. Constant. |
| `lorenz` | `(frequency)` | Lorenz chaotic oscillator. The frequency input has only a slight effect on the output. |
| `rossler` | `(frequency)` | Rossler chaotic oscillator, with peaks at multiples of the frequency input. |

### Filters
| Name | Arguments | Notes |
| --- | --- | --- |
| `lowpass` | `(audio, cutoff, q)` | Resonant lowpass filter. |
| `highpass` | `(audio, cutoff, q)` | Resonant highpass filter. |
| `bandpass` | `(audio, frequency, q)` | Bandpass filter. Keeps frequencies near the center, attenuating either side. |
| `notch` | `(audio, frequency, q)` | Notch filter. Removes a narrow band around the center frequency. |
| `peak` | `(audio, frequency, q)` | Peaking filter. |
| `allpass` | `(audio, frequency, q)` | Allpass filter. Passes all frequencies but shifts their phase around the center frequency. |
| `lowrez` | `(audio, cutoff, q)` | Resonant two-pole lowpass filter. |
| `bandrez` | `(audio, frequency, q)` | Resonant two-pole bandpass filter. |
| `moog` | `(signal, cutoff, q)` | Moog-style resonant lowpass ladder filter. |
| `resonator` | `(audio, frequency, q)` | Constant-gain bandpass resonator. |
| `bell` | `(audio, frequency, q, gain)` | Bell equalizer. Boosts or cuts a band around the center frequency by `gain` (an amplitude multiplier, not dB). |
| `lowshelf` | `(audio, frequency, q, gain)` | Low shelf filter. Scales everything below the center frequency by `gain` (an amplitude multiplier). |
| `highshelf` | `(audio, frequency, q, gain)` | High shelf filter. Scales everything above the center frequency by `gain` (an amplitude multiplier). |
| `morph` | `(signal, frequency, q, morph)` | Filter that morphs continuously between modes: `morph` runs -1 (lowpass) to 0 (peak) to 1 (highpass). |
| `lowpole` | `(audio, cutoff)` | First-order one-pole lowpass. No resonance. |
| `highpole` | `(audio, cutoff)` | First-order one-pole one-zero highpass. No resonance. |
| `allpole` | `(audio, delay)` | First-order allpass filter with a configurable delay at DC, in samples (must be > 0). |
| `butterpass` | `(audio, cutoff)` | Second-order Butterworth lowpass. Maximally flat passband, no resonance control. |
| `pinkpass` | `(signal)` | Pinking filter: -3 dB per octave. Turns white noise into pink. |
| `dcblock` | `(signal)` | Remove DC offset, keeping the signal zero-centered. Cutoff is 10 Hz. |
| `biquad` | `(signal → a1, a2, b0, b1, b2)` | Arbitrary biquad filter with coefficients in normalized form. All five coefficients must be constants. |
| `fir3` | `(signal → gain)` | Three-point symmetric FIR filter, specified by its `gain` (>= 0) at the Nyquist frequency. A gain below 1 gives a gentle lowpass. |

### Envelopes and Dynamics
| Name | Arguments | Notes |
| --- | --- | --- |
| `perc` | `(→ attack, release)` | Self-contained percussive envelope: rise, fall, silence. Needs no note length, so it works in a voice or the persistent graph. Both times are constants. |
| `env` | `(→ attack, decay, sustain, release, duration)` | Time-based ADSR for one-shot voices, with the release landing exactly on `duration`. Pass the voice-bound `dur` as the duration. All arguments are constants. |
| `adsr` | `(gate → attack, decay, sustain, release)` | Gated ADSR envelope. Rises while the gate is positive, releases when it returns to zero. Times are in seconds; sustain is a level in 0..=1. |
| `follow` | `(signal → response_time)` | Parameter follower. Smooths the signal with the given halfway response time, in seconds. |
| `afollow` | `(signal → attack, release)` | Asymmetric parameter follower. Smooths rising segments over `attack` and falling ones over `release` (halfway response times, in seconds). |
| `limiter` | `(signal → attack, release)` | Look-ahead limiter holding the signal to -1..=1. Look-ahead equals the attack time. Times are constants, in seconds. |
| `clip` | `(signal)` | Hard-clip the signal to -1..=1. |
| `clip_to` | `(signal → minimum, maximum)` | Hard-clip the signal to `minimum`..=`maximum`. Both bounds are constants. |
| `declick` | `(signal)` | Fade the signal in over 10 ms from time zero, suppressing the click at the start of a graph. |

### Delays and Effects
| Name | Arguments | Notes |
| --- | --- | --- |
| `delay` | `(signal → time)` | Fixed delay of `time` seconds, rounded to the nearest sample. The time is a constant — use `tap` for a modulatable delay. |
| `tap` | `(signal, delay → min_delay, max_delay)` | Tapped delay line with cubic interpolation. Unlike `delay`, the delay time is a signal, so it can be modulated — it must stay within the constant `min_delay`..=`max_delay` bounds, in seconds. |
| `tick` | `(signal)` | Single-sample delay. The building block for feedback and comb filters. |
| `hold` | `(signal, frequency → variability)` | Sample-and-hold. Samples the signal at `frequency` Hz; `variability` in 0..=1 jitters the sampling interval and is a constant. |
| `chorus` | `(audio → seed, separation, variation, mod_frequency)` | Five-voice mono chorus, mixed with the dry signal. Stack two with different seeds for stereo. All parameters except the audio input are constants. |
| `pluck` | `(excitation → frequency, gain_per_second, damping)` | Karplus-Strong plucked string. Feed it a burst — `impulse()` or a short noise envelope — as the excitation. Frequency, gain and damping (0..=1) are constants. |
| `reverb` | `(audio → room_size, time, damping)` | Reverb (32-channel FDN). `room_size` is in meters (10 is an average room), `time` is the decay to -60 dB in seconds, `damping` in 0..=1 rolls off the highs. Wet only: `x + reverb(x, 10, 3, 0.5) * 0.2`. All parameters except the audio input are constants. |
| `reverb2` | `(audio → room_size, time, diffusion, modulation, damping_cutoff)` | Hybrid FDN reverb — richer and more expensive than `reverb`. `room_size` is in meters and clamps to 10..=30, `diffusion` in 0..=1 thickens the tail, `modulation` around 1 adds movement (higher goes audibly Doppler), and `damping_cutoff` is the lowpass applied to each loop pass, in hertz. Wet only. All parameters except the audio input are constants. |
| `reverb3` | `(audio → time, diffusion, damping_cutoff)` | Allpass-loop reverb, with no room size — just `time` to -60 dB, `diffusion` in 0..=1, and a `damping_cutoff` in hertz applied to each loop pass. Wet only. All parameters except the audio input are constants. |
| `reverb4` | `(audio → room_size, time)` | Reverb with a slow fade-in, for swells rather than rooms. `room_size` is in meters and is treated as at least 15; below that the delay times stop sounding like a space. Wet only. Both `room_size` and `time` are constants. |

The reverbs and delays are wet only:

```
fn pad(n) = saw(n.m2h) * env(0.3, 0.2, 0.7, 0.4, dur)
fn wet(x) = x + reverb(x, 10, 3, 0.5) * 0.3
```

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


## Function Reference

- [List Functions](#fist-functions)
- [Math Functions](#math-functions)
- [Oscilators and Sources](#oscilators-and-sources)
- [Noise and Chaos](#noise-and-chaos)
- [Filters](#filters)
- [Envelopes and dynamics](#envelopes-and-dynamics)
- [Delays and Effects](#delays-and-effects)

### List Functions

| Name | Signature | Notes |
| --- | --- | --- |
| `len` | `len(list) -> number` | Works on list literals and ranges. Errors on non-lists and on wrong arity. |
| `zip` | `zip(a, b, …) -> list of lists` | Variadic. Pairs positionally into rows `[[a0,b0], [a1,b1], …]`. **All arguments must be lists of equal length** — a mismatch is an error, not a silent truncation. Rows carry whatever `Value`s went in, including signals. |
| `rev` | `rev(list) -> list` | Reversed. |
| `palindrome` | `palindrome(list) -> list` | The sequence then its mirror: `[1,2,3]` → `[1,2,3,3,2,1]`. |
| `rotl` / `rotr` | `rotl(list, n = 1) -> list` | Rotate left / right. The amount wraps, so `rotl(l, len(l))` is the identity, and a negative amount rotates the other way. An empty list is returned unchanged. |
| `push` | `push(list, value) -> list` | Appends. Returns a new list; the original is untouched. |
| `pop` | `pop(list) -> list` | Drops the **last** element. Nothing is returned "off the top" — index the list for that. Errors on an empty list. |
| `sort` | `sort(list) -> list` | Ascending. Every element must be a compile-time number. |
| `sum` | `sum(list) -> number | signal` | Folds through the same `combine` the arithmetic operators use, so numbers fold to a constant and **signals emit `Add` nodes** — `sum([sin(110), sin(220)])` is additive synthesis without a `for`. Empty list → `0`. |
| `split` | `split(list, n) -> list of lists` | Chunks of `n` (not split-at-index). A short final chunk is kept. `n` must be a whole number ≥ 1. |
| `filter` | `filter(list, fn) -> list` | Keeps elements the predicate answers non-zero for. The predicate is an ordinary user `fn`; it must return a compile-time number. |
| `choice` | `choice(list) -> value` | One random element. Errors on an empty list. |
| `wchoice` | `wchoice(values, weights) -> value` | Weighted random pick. Parallel lists of equal length, like `zip`. Weights must be finite and ≥ 0, and not all zero. |
| `scramble` | `scramble(list) -> list` | Fisher-Yates shuffle. |

### Math Functions
| Name | Signature | Notes |
| --- | --- | --- |
| `m2h` | `m2h(note)` | MIDI note to hertz. `69.m2h` is 440, `60.m2h` is 261.63. Equal temperament, A4 = 440. Fractional notes work — that is how you get glides. |
| `h2m` | `h2m(hz)` | The inverse. `440.h2m` is 69; the result may be fractional. Frequency must be above zero. |
| `db` | `db(decibels)` | Decibels to linear amplitude. `0.db` is 1, `(-6).db` is about 0.5. |
| `amp` | `amp(amplitude)` | The inverse. `1.amp` is 0. Amplitude must be above zero. |
| `cents` | `cents(hz, cents)` | Detune a frequency by hundredths of a semitone. `440.cents(1200)` is 880. |
| `bpm` | `bpm(beats)` | Beats per minute to cycles per second, **taking one cycle as four beats**. `120.bpm` is 0.5 — exactly `DEFAULT_CPS`. |
| `oct` | `oct(note, octaves)` | Transpose by whole octaves. `60.oct(-1)` is 48. |
| `semi` | `semi(note, semitones)` | Transpose by semitones. `60.semi(7)` is 67. |
| `scale` | `scale(note, scale)` | Snap to the **nearest** tone of a scale given as semitone offsets within an octave. `61.scale([0,2,4,5,7,9,11])` is 60. Neighbouring octaves are candidates, so 59 against `[0,4,7]` rises to 60 rather than falling a seventh. Ties snap down. |
| `clamp` | `clamp(x, lo, hi)` | Constrain to `lo..=hi`. An empty range is an error. |
| `norm` | `norm(x, lo, hi)` | Map 0..1 onto `lo..hi`. Values outside 0..1 **extrapolate** — `clamp` first if that is not what you want. |
| `wrap` | `wrap(x, lo, hi)` | Fold back into the range rather than clamping. `13.wrap(0, 12)` is 1. Useful for modular pitch. |
| `round` / `floor` / `ceil` / `abs` | `round(x)` | The obvious ones. `round` takes halves away from zero. |
| `pow` | `pow(x, exponent)` | `2.pow(10)` is 1024. For exponential curve shaping. |
| `sqrt` | `sqrt(x)` | `x` must not be negative. |
| `log2` | `log2(x)` | `x` must be above zero. |

### Oscilators and Sources

| Function  | Arguments |
| --- | --- |
| `sin` | `(freq)` |
| `saw` | `(freq)` |
| `square` | `(freq)` |
| `triangle` |  `(freq)` |
| `soft_saw` | `(freq)` |
| `hammond` |  `(freq)` |
| `organ` | `(freq)` |
| `ramp` | `(freq)` |
| `poly_saw` | `(freq)` — band-limited |
| `poly_square` | `(freq)` — band-limited |
| `poly_pulse` | `(freq, duty)` — band-limited |
| `pulse` | `(freq, duty)` |
| `dsf_saw` | `(freq, roughness)` |
| `dsf_square` | `(freq, roughness)` |
| `impulse` | `()` — single-sample impulse |

### Noise and Chaos
| Builtin | Arguments |
| --- | --- |
| `noise` | `()` — white |
| `pink` | `()` |
| `brown` | `()` |
| `mls` | `()` — maximum-length sequence |
| `mls_bits` | `(→ bits)` — **param only**, no ports |
| `lorenz` | `(freq)` — chaotic attractor |
| `rossler` | `(freq)` — chaotic attractor |

### Filters
| Builtin | Arguments |
| --- | --- |
| `lowpass` | `(audio, cutoff, Q)` |
| `highpass` | `(audio, cutoff, Q)` |
| `bandpass` | `(audio, center, Q)` |
| `notch` | `(audio, center, Q)` |
| `peak` | `(audio, center, Q)` |
| `allpass` |`(audio, center, Q)` |
| `lowrez` | `(audio, cutoff, Q)` |
| `bandrez` | `(audio, center, Q)` |
| `moog` | `(audio, cutoff, Q)` — Moog ladder |
| `resonator` |  `(audio, center, bandwidth)` |
| `bell` | `(audio, center, Q, gain)` |
| `lowshelf` | `(audio, cutoff, Q, gain)` |
| `highshelf` | `(audio, cutoff, Q, gain)` |
| `morph` | `(audio, cutoff, Q, morph)` |
| `lowpole` | `(audio, cutoff)` — 1-pole |
| `highpole` | `(audio, cutoff)` — 1-pole |
| `allpole` | `(audio, cutoff)` — 1-pole |
| `butterpass` | `(audio, cutoff)` — Butterworth lowpass |
| `pinkpass` | `(audio)` — pinking filter |
| `dcblock` | `(audio)` |
| `biquad` | `(audio → a1, a2, b0, b1, b2)` — raw coefficients, all params |
| `fir3` | `(audio → gain)` |

### Envelopes and dynamics
| Builtin | Arguments |
| --- | --- |
| `perc` |`(→ attack, release)` — **params only, no ports.** Self-contained percussive shape: rise over `attack`, fall over `release`, silence after. Needs no note length, so it works in a voice **or** the persistent graph. |
| `env` | `(→ attack, decay, sustain, release, dur)` — **params only, no ports.** Time-based ADSR whose release lands exactly on `dur`, so it fits inside the sequencer event. `sustain` is a **level** (clamped to 0..=1), not a time. Voice-only in practice, since `dur` is only bound there. |
| `adsr` | `(gate → attack, decay, sustain, release)` — **gate-driven**, for the persistent graph. ⚠️ It swallows its first trigger: the underlying `adsr_live` starts in the "note already in progress" state, so a gate that is high from t=0 never fires an attack. It needs a full off→on edge first. Prefer `perc` / `env` in instruments. |
| `follow` | `(audio → response_time)` — smoothing follower |
| `afollow` | `(audio → attack, release)` — asymmetric follower |
| `limiter` | `(audio → attack, release)` |
| `clip` | `(audio)` — clip to ±1 |
| `clip_to` | `(audio → min, max)` |
| `declick` | `(audio)` — fade in at start |

### Delays and Effects
| Builtin | Arguments |
| --- | --- |
| `delay` | `(audio → time)` — fixed delay in seconds |
| `tap` | `(audio, delay_time → min_delay, max_delay)` — modulatable tap |
| `tick` | `(audio)` — one-sample delay |
| `hold` | `(audio, freq → variability)` — sample & hold |
| `chorus` | `(audio → seed, separation, variation, mod_freq)` |
| `pluck` | `(excitation → freq, gain_per_second, damping)` — Karplus-Strong |
| `reverb` | `(audio → room_size, time, damping)` — 32-channel FDN. `room_size` in meters (10 is average), `time` is decay to -60 dB, `damping` in 0..=1 rolls off the highs. |
| `reverb2` | `(audio → room_size, time, diffusion, modulation, damping_cutoff)` — hybrid FDN, richer and more expensive. `room_size` clamps to 10..=30 m, `diffusion` in 0..=1 thickens the tail, `modulation` near 1 adds movement (higher goes audibly Doppler), `damping_cutoff` is a lowpass in hertz applied on each loop pass. |
| `reverb3` | `(audio → time, diffusion, damping_cutoff)` — allpass loop, no room size. |
| `reverb4` | `(audio → room_size, time)` — slow fade-in, for swells rather than rooms. `room_size` is treated as at least 15 m; below that the delay times stop sounding like a space. |

The reverbs and delays are **wet only** — mix the dry signal yourself:

```
fn pad(n) = saw(n.m2h) * env(0.3, 0.2, 0.7, 0.4, dur)
fn wet(x) = x + reverb(x, 10, 3, 0.5) * 0.3
```

/**
 * Patterns drawn in the side panel.
 *
 * A drawn pattern is exactly what a list literal is — a row of steps, each one
 * a pitch, a rest or a bare trigger — so the backend can bind its name to the
 * same `Value::List` the language would have built from `[c4, `, \]`. That is
 * the whole trick: nothing downstream needs to know a pattern was drawn.
 */

/** One beat. A pitch carries a MIDI note number, always a valid one: the
 *  editor commits a cell only when what was typed parses. */
export type PatternStep =
  | { kind: "rest" }
  | { kind: "trigger" }
  | { kind: "pitch"; note: number };

export interface GraphicalPattern {
  /** Stable across renames — the name is the user's, this is React's. */
  id: string;
  name: string;
  steps: PatternStep[];
}

export const MIN_BEATS = 1;
export const MAX_BEATS = 64;

/** What the backend will accept as a name it can bind. Mirrors `check_name`
 *  in `src-tauri/src/pattern/graphical.rs`. */
const IDENT = /^[a-zA-Z_]\w*$/;

/**
 * Why this pattern's name cannot be used, or null when it can.
 *
 * `all` is every pattern including this one, so the duplicate test is against
 * the others by id rather than by position.
 */
export function nameError(p: GraphicalPattern, all: GraphicalPattern[]): string | null {
  if (p.name.trim() === "") return "needs a name";
  if (!IDENT.test(p.name)) return "letters, digits and _ only, not starting with a digit";
  if (all.some((q) => q.id !== p.id && q.name === p.name)) return "already taken";
  return null;
}

let counter = 0;

/** A new pattern, named so it does not collide with the ones already there. */
export function makePattern(existing: GraphicalPattern[]): GraphicalPattern {
  const taken = new Set(existing.map((p) => p.name));
  let name: string;
  do {
    counter += 1;
    name = `pat${counter}`;
  } while (taken.has(name));

  return {
    id: `gp-${counter}-${Date.now()}`,
    name,
    steps: Array.from({ length: 8 }, () => ({ kind: "rest" }) as PatternStep),
  };
}

/** Grow with rests or truncate, keeping whatever the shorter of the two held. */
export function resize(steps: PatternStep[], beats: number): PatternStep[] {
  const n = Math.max(MIN_BEATS, Math.min(MAX_BEATS, Math.round(beats)));
  if (n <= steps.length) return steps.slice(0, n);
  return [...steps, ...Array.from({ length: n - steps.length }, () => ({ kind: "rest" }) as PatternStep)];
}

/** Rest -> trigger -> pitch -> rest. The pitch a cell lands on is middle C,
 *  which is then edited in place rather than cycled to. */
export function cycle(step: PatternStep): PatternStep {
  switch (step.kind) {
    case "rest":
      return { kind: "trigger" };
    case "trigger":
      return { kind: "pitch", note: 60 };
    case "pitch":
      return { kind: "rest" };
  }
}

/** Sharps rather than flats, matching how `lang::note` spells them: `cs4`. */
const NOTE_NAMES = ["c", "cs", "d", "ds", "e", "f", "fs", "g", "gs", "a", "as", "b"];

/** The lowest and highest MIDI numbers `lang::note` can spell: `c0` to `g9`. */
const MIN_NOTE = 12;
const MAX_NOTE = 127;

/** A MIDI number as the language would write it: `60` is `c4`. */
export function noteName(note: number): string {
  if (note < MIN_NOTE || note > MAX_NOTE || !Number.isInteger(note)) return String(note);
  return `${NOTE_NAMES[note % 12]}${Math.floor(note / 12) - 1}`;
}

const SPELLED = /^([a-g])([sf]?)([0-9])$/;
const OFFSETS: Record<string, number> = { c: 0, d: 2, e: 4, f: 5, g: 7, a: 9, b: 11 };

/**
 * Read a cell's text as a MIDI note, or null if it is not one.
 *
 * Both spellings the language accepts work — a note name (`a2`, `fs3`, `ef4`)
 * or a bare MIDI number — because both are things a user of this language
 * already writes.
 */
export function parseNote(text: string): number | null {
  const s = text.trim().toLowerCase();
  if (s === "") return null;

  if (/^\d+(\.\d+)?$/.test(s)) {
    const n = Number(s);
    return n >= 0 && n <= MAX_NOTE ? n : null;
  }

  const m = SPELLED.exec(s);
  if (!m) return null;
  const [, letter, accidental, octave] = m;
  const semitone = OFFSETS[letter] + (accidental === "s" ? 1 : accidental === "f" ? -1 : 0);
  const note = (Number(octave) + 1) * 12 + semitone;
  return note >= MIN_NOTE && note <= MAX_NOTE ? note : null;
}

/** The shape `run_code` deserializes. Ids and edit state stay on this side. */
export interface WirePattern {
  name: string;
  steps: PatternStep[];
}

/** Everything the backend can bind. A pattern whose name it would refuse is
 *  left out, so a half-typed name cannot fail an otherwise good eval — and, for
 *  the same reason, cannot reach the project's patterns file. */
export function toWire(patterns: GraphicalPattern[]): WirePattern[] {
  return patterns
    .filter((p) => nameError(p, patterns) === null)
    .map((p) => ({ name: p.name, steps: p.steps }));
}

/**
 * Rows read back from the project's `patterns.scree`.
 *
 * The ids are minted here because they never existed on disk: they are this
 * side's handle on a row across a rename, and the file has no use for them.
 */
export function fromWire(wire: WirePattern[]): GraphicalPattern[] {
  return wire.map((p) => {
    counter += 1;
    return { id: `gp-${counter}-${Date.now()}`, name: p.name, steps: p.steps };
  });
}

import { useState } from "react";
import {
  cycle,
  makePattern,
  nameError,
  noteName,
  parseNote,
  resize,
  MAX_BEATS,
  MIN_BEATS,
  type GraphicalPattern,
  type PatternStep,
} from "./patterns";

/**
 * The drawn-pattern editor: a list of named grids, each one a row of beats.
 *
 * A pattern is worth drawing rather than typing when the rhythm is the point,
 * so the cell is a single click that walks rest -> trigger -> pitch, and the
 * only thing that ever needs typing is the pitch itself.
 *
 * The name is the whole reason this connects to anything: it is bound in the
 * program's environment before the program runs, so `play(hats, hat)` in the
 * editor reads the grid below. That makes a valid, unique name a hard
 * requirement rather than a nicety, which is why it is checked as you type.
 */

/** Colours match the editor's own: rests warm, triggers green, pitches rose. */
const CELL_STYLE: Record<PatternStep["kind"], string> = {
  rest: "border-neutral-700 bg-neutral-800/60 text-neutral-500 hover:border-neutral-600",
  trigger: "border-green-700/60 bg-green-500/15 text-green-400 hover:border-green-600",
  pitch: "border-rose-700/60 bg-rose-500/15 text-rose-300 hover:border-rose-600",
};

const CELL_GLYPH: Record<PatternStep["kind"], string> = {
  rest: "`",
  trigger: "\\",
  pitch: "♪",
};

/**
 * The pitch of one cell, edited as text.
 *
 * Held locally while it is being typed and committed only when it parses, so
 * the pattern itself never holds a note the backend could not read. A bad
 * entry reverts on blur rather than blocking the field — half of `cs4` is not
 * an error, it is someone still typing.
 */
function NoteInput({ note, onChange }: { note: number; onChange: (note: number) => void }) {
  const [text, setText] = useState<string | null>(null);
  const shown = text ?? noteName(note);
  const bad = text !== null && parseNote(text) === null;

  const commit = () => {
    if (text !== null) {
      const parsed = parseNote(text);
      if (parsed !== null) onChange(parsed);
    }
    setText(null); // back to showing the committed value, however it was typed
  };

  return (
    <input
      value={shown}
      onChange={(e) => setText(e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => {
        if (e.key === "Enter") e.currentTarget.blur();
        if (e.key === "Escape") {
          setText(null);
          e.currentTarget.blur();
        }
      }}
      title="A note name (c4, fs3, ef2) or a MIDI number"
      className={
        "w-full rounded border bg-neutral-900 px-1 py-0.5 text-center font-mono text-[11px] " +
        "text-rose-200 outline-none " +
        // The warning has to survive focus: it is at its most useful while the
        // thing that caused it is still being typed.
        (bad ? "border-red-500" : "border-neutral-700 focus:border-rose-500")
      }
    />
  );
}

interface CellProps {
  index: number;
  step: PatternStep;
  onChange: (step: PatternStep) => void;
}

function Cell({ index, step, onChange }: CellProps) {
  return (
    <div className="flex w-11 flex-col items-center gap-0.5">
      <span className="text-[9px] tabular-nums text-neutral-600">{index + 1}</span>
      <button
        onClick={() => onChange(cycle(step))}
        title={`Beat ${index + 1}: ${step.kind} — click to change`}
        className={
          "w-full rounded border py-1 text-center font-mono text-sm leading-4 transition-colors " +
          CELL_STYLE[step.kind]
        }
      >
        {CELL_GLYPH[step.kind]}
      </button>
      {/* The slot is kept whether or not it holds an input, so a row of mixed
          cells lines up instead of stepping up and down. */}
      <div className="h-5 w-full">
        {step.kind === "pitch" && (
          <NoteInput note={step.note} onChange={(note) => onChange({ kind: "pitch", note })} />
        )}
      </div>
    </div>
  );
}

interface PatternCardProps {
  pattern: GraphicalPattern;
  error: string | null;
  onChange: (pattern: GraphicalPattern) => void;
  onRemove: () => void;
}

function PatternCard({ pattern, error, onChange, onRemove }: PatternCardProps) {
  const setBeats = (beats: number) =>
    onChange({ ...pattern, steps: resize(pattern.steps, beats) });

  const setStep = (i: number, step: PatternStep) =>
    onChange({ ...pattern, steps: pattern.steps.map((s, j) => (j === i ? step : s)) });

  return (
    <div className="rounded-md border border-neutral-800 bg-neutral-900/60 p-2">
      <div className="flex items-center gap-1">
        <input
          value={pattern.name}
          onChange={(e) => onChange({ ...pattern, name: e.target.value })}
          spellCheck={false}
          placeholder="name"
          title="The name to use in play()"
          className={
            "min-w-0 flex-1 rounded border bg-neutral-950 px-2 py-1 font-mono text-sm " +
            "text-neutral-100 outline-none " +
            (error ? "border-red-500" : "border-neutral-700 focus:border-blue-600")
          }
        />
        <button
          onClick={onRemove}
          title="Delete pattern"
          className="rounded p-1 text-neutral-500 transition-colors hover:bg-neutral-800 hover:text-red-400"
        >
          <svg viewBox="0 0 24 24" fill="currentColor" className="h-3.5 w-3.5">
            <path d="M18.3 5.7 12 12l6.3 6.3-1.4 1.4L10.6 13.4 4.3 19.7 2.9 18.3 9.2 12 2.9 5.7 4.3 4.3l6.3 6.3 6.3-6.3z" />
          </svg>
        </button>
      </div>

      {error && <p className="mt-1 text-[11px] text-red-400">{error}</p>}

      <div className="mt-2 flex items-center gap-2">
        <span className="text-[11px] uppercase tracking-wide text-neutral-500">Beats</span>
        <div className="flex items-center gap-1">
          <button
            onClick={() => setBeats(pattern.steps.length - 1)}
            disabled={pattern.steps.length <= MIN_BEATS}
            title="One fewer beat"
            className="h-5 w-5 rounded border border-neutral-700 text-neutral-300 transition-colors hover:bg-neutral-800 disabled:opacity-30"
          >
            −
          </button>
          <input
            type="number"
            min={MIN_BEATS}
            max={MAX_BEATS}
            value={pattern.steps.length}
            onChange={(e) => {
              // An emptied field is mid-edit, not a request for zero beats.
              if (Number.isFinite(e.target.valueAsNumber)) setBeats(e.target.valueAsNumber);
            }}
            className="w-12 rounded border border-neutral-700 bg-neutral-950 px-1 py-0.5 text-center font-mono text-xs text-neutral-100 outline-none focus:border-blue-600"
          />
          <button
            onClick={() => setBeats(pattern.steps.length + 1)}
            disabled={pattern.steps.length >= MAX_BEATS}
            title="One more beat"
            className="h-5 w-5 rounded border border-neutral-700 text-neutral-300 transition-colors hover:bg-neutral-800 disabled:opacity-30"
          >
            +
          </button>
        </div>
      </div>

      <div className="mt-2 flex flex-wrap gap-1">
        {pattern.steps.map((step, i) => (
          <Cell key={i} index={i} step={step} onChange={(s) => setStep(i, s)} />
        ))}
      </div>
    </div>
  );
}

interface PatternPanelProps {
  patterns: GraphicalPattern[];
  onChange: (patterns: GraphicalPattern[]) => void;
  /** The project's patterns file, once it is known. Named on screen because a
   *  panel that quietly writes a file into someone's project should say so —
   *  and because it is a file they can open, edit and keep. */
  path: string | null;
}

export function PatternPanel({ patterns, onChange, path }: PatternPanelProps) {
  return (
    <section className="border-t border-neutral-800 px-4 py-4">
      <div className="flex items-center justify-between">
        <h2 className="text-xs font-semibold uppercase tracking-wide text-neutral-400">
          Patterns
        </h2>
        <button
          onClick={() => onChange([...patterns, makePattern(patterns)])}
          title="New graphical pattern"
          className="rounded-md p-1 text-neutral-400 transition-colors hover:bg-neutral-800 hover:text-neutral-100"
        >
          <svg viewBox="0 0 24 24" fill="currentColor" className="h-4 w-4">
            <path d="M11 11V5h2v6h6v2h-6v6h-2v-6H5v-2z" />
          </svg>
        </button>
      </div>

      {patterns.length === 0 ? (
        <p className="mt-2 text-xs leading-relaxed text-neutral-500">
          None yet. Add one, name it, and play it from the editor:{" "}
          <span className="font-mono text-neutral-400">play(name, kick)</span>
        </p>
      ) : (
        <div className="mt-3 flex flex-col gap-3">
          {patterns.map((p) => (
            <PatternCard
              key={p.id}
              pattern={p}
              error={nameError(p, patterns)}
              onChange={(next) => onChange(patterns.map((q) => (q.id === p.id ? next : q)))}
              onRemove={() => onChange(patterns.filter((q) => q.id !== p.id))}
            />
          ))}
        </div>
      )}

      {path !== null && (
        <p className="mt-3 text-[11px] leading-relaxed text-neutral-600" title={path}>
          Saved in{" "}
          <span className="font-mono text-neutral-500">{path.split(/[\\/]/).pop()}</span>, and
          in scope for every file in this project.
        </p>
      )}
    </section>
  );
}

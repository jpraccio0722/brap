import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { PatternPanel } from "./PatternPanel";
import type { GraphicalPattern } from "./patterns";

const MIN_BPM = 40;
const MAX_BPM = 240;

/** The engine's global controls, as the backend reports them. */
interface Transport {
  /** Beats per minute. One cycle is four beats. */
  bpm: number;
  /** Linear amplitude, 0 to 1. */
  volume: number;
}

interface SliderProps {
  label: string;
  /** The value spelled out for reading, e.g. "120 bpm". */
  readout: string;
  value: number;
  min: number;
  max: number;
  step: number;
  onChange: (value: number) => void;
}

function Slider({ label, readout, value, min, max, step, onChange }: SliderProps) {
  return (
    <label className="block">
      <div className="flex items-baseline justify-between">
        <span className="text-xs font-medium uppercase tracking-wide text-neutral-400">
          {label}
        </span>
        <span className="font-mono text-sm tabular-nums text-neutral-200">{readout}</span>
      </div>
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        onChange={(e) => onChange(e.target.valueAsNumber)}
        className="mt-2 w-full accent-emerald-500"
      />
    </label>
  );
}

interface TransportPanelProps {
  open: boolean;
  /** Pixels wide, as the drag handle has left it. */
  width: number;
  /** Grab handle for that drag, drawn along the panel's leading edge. */
  onResizeStart: (e: React.PointerEvent) => void;
  /** Evaluate the active tab and hand the result to the engine. */
  play: () => void | Promise<void>;
  /** Silence everything the engine and scheduler are holding. */
  stop: () => void | Promise<void>;
  patterns: GraphicalPattern[];
  onPatternsChange: (patterns: GraphicalPattern[]) => void;
  /** The project's patterns file, passed through to the panel that writes it. */
  patternsPath: string | null;
}

/**
 * Global controls, separate from the program in the editor: they apply to
 * whatever is playing and survive a re-eval.
 *
 * The engine owns the values, so the panel reads them on mount rather than
 * assuming defaults. Hiding only takes it off screen — it stays mounted, so
 * showing it again is instant and lands on the settings it was left at.
 *
 * Each edit goes straight out; sends are cheap enough to fire on every frame
 * of a drag, and the audio side is built to take them mid-performance.
 *
 * The drawn patterns below the transport are a different kind of thing — they
 * are part of the program, not the engine — but they share the panel because
 * they share the gesture: something you reach for while the music is running.
 */
export function TransportPanel({
  open,
  width,
  onResizeStart,
  play,
  stop,
  patterns,
  onPatternsChange,
  patternsPath,
}: TransportPanelProps) {
  const [transport, setTransport] = useState<Transport | null>(null);

  useEffect(() => {
    let live = true;
    invoke<Transport>("transport")
      .then((t) => {
        if (live) setTransport(t);
      })
      .catch((e) => console.error("could not read the transport:", e));
    return () => {
      live = false;
    };
  }, []);

  const setBpm = useCallback((bpm: number) => {
    setTransport((t) => (t ? { ...t, bpm } : t));
    invoke("set_tempo", { bpm }).catch((e) => console.error("could not set tempo:", e));
  }, []);

  const setVolume = useCallback((volume: number) => {
    setTransport((t) => (t ? { ...t, volume } : t));
    invoke("set_master_volume", { volume }).catch((e) =>
      console.error("could not set volume:", e),
    );
  }, []);

  return (
    <aside
      style={{ width }}
      className={
        "relative shrink-0 flex-col border-l border-neutral-800 bg-neutral-950/40 " +
        (open ? "flex" : "hidden")
      }
    >
      {/* The drag handle sits on the border itself and is a little wider than
          it looks, so the panel can be resized without hunting for one pixel. */}
      <div
        onPointerDown={onResizeStart}
        title="Drag to resize"
        role="separator"
        aria-orientation="vertical"
        className="absolute inset-y-0 -left-1 z-10 w-2 cursor-col-resize hover:bg-emerald-600/40 active:bg-emerald-600/60"
      />

      {/* No close control of its own: the header's hamburger works both ways,
          which is the one place a reader already looks for it. */}
      <div className="border-b border-neutral-800 px-4 py-2">
        <h2 className="text-xs font-semibold uppercase tracking-wide text-neutral-400">
          Transport
        </h2>
      </div>

      {/* Everything below the title scrolls together: the patterns list has no
          bound, and clipping the sliders off the bottom of a short window would
          be worse than scrolling past them. */}
      <div className="min-h-0 flex-1 overflow-y-auto">
        {/* Play and stop sit above the sliders, and outside the gate below: they
            are the panel's reason to be open, and they work before the engine has
            answered. Keyboard players still have ⌘, and ⌘. with the panel shut. */}
        <div className="flex gap-2 border-b border-neutral-800 px-4 py-4">
          <button
            onClick={() => void play()}
            title="Run (⌘,)"
            className="inline-flex flex-1 items-center justify-center gap-2 rounded-md bg-emerald-600 px-3 py-1.5 text-sm font-medium text-white transition-colors hover:bg-emerald-500 active:bg-emerald-700"
          >
            <svg viewBox="0 0 24 24" fill="currentColor" className="h-4 w-4">
              <path d="M8 5v14l11-7z" />
            </svg>
            Play
          </button>
          <button
            onClick={() => void stop()}
            title="Stop (⌘.)"
            className="inline-flex flex-1 items-center justify-center gap-2 rounded-md bg-red-700 px-3 py-1.5 text-sm font-medium text-white transition-colors hover:bg-red-600 active:bg-red-800"
          >
            <svg viewBox="0 0 24 24" fill="currentColor" className="h-4 w-4">
              <path d="M6 6h12v12H6z" />
            </svg>
            Stop
          </button>
        </div>

        {/* Nothing to drag until the engine has told us where the controls sit. */}
        {transport && (
          <div className="flex flex-col gap-6 px-4 py-4">
            <Slider
              label="Tempo"
              readout={`${Math.round(transport.bpm)} bpm`}
              value={transport.bpm}
              min={MIN_BPM}
              max={MAX_BPM}
              step={1}
              onChange={setBpm}
            />
            <Slider
              label="Volume"
              readout={`${Math.round(transport.volume * 100)}%`}
              value={transport.volume}
              min={0}
              max={1}
              step={0.01}
              onChange={setVolume}
            />
          </div>
        )}

        <PatternPanel
          patterns={patterns}
          onChange={onPatternsChange}
          path={patternsPath}
        />
      </div>
    </aside>
  );
}

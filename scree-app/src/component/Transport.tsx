import { useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import PlayIcon from "../assets/play.svg?react";
import StopIcon from "../assets/stop.svg?react";
import { ValueControl } from "./ValueControl";

const MIN_BPM = 40;
const MAX_BPM = 240;

/** The engine's global controls, as the backend reports them. */
export interface TransportState {
  /** Beats per minute. One cycle is four beats. */
  bpm: number;
  /** Linear amplitude, 0 to 1. */
  volume: number;
}

interface TransportProps {
  /** Evaluate the active tab and hand the result to the engine. */
  play: () => void | Promise<void>;
  /** Silence everything the engine and scheduler are holding. */
  stop: () => void | Promise<void>;
  /**
   * Where the controls sit, or null until something has said — on launch,
   * before the engine has answered.
   */
  state: TransportState | null;
  /** Report a move, so the open project can remember it. */
  onChange: (state: TransportState) => void;
}

/**
 * Play, stop, tempo and volume, in the title bar.
 *
 * They live here rather than in the transport panel because they are the
 * controls that must stay reachable with every panel shut — the engine keeps
 * running whatever is on screen, and the things that steer it should not be
 * behind a tab. Keyboard players still have ⌘, and ⌘.
 *
 * Tempo and volume belong to the engine rather than to the program, so their
 * values are never assumed here: they are read from the engine on launch and
 * from the project's own file when one is opened, both of which happen a level
 * up — a setting that is remembered belongs to the thing that remembers it.
 * What is left here is the surface. Each edit goes straight out: sends are
 * cheap enough to fire on every frame of a drag, and the audio side is built to
 * take them mid-performance.
 */
export function Transport({ play, stop, state, onChange }: TransportProps) {
  const setBpm = useCallback(
    (bpm: number) => {
      if (!state) return;
      onChange({ ...state, bpm });
      invoke("set_tempo", { bpm }).catch((e) => console.error("could not set tempo:", e));
    },
    [state, onChange],
  );

  // The control works in percent, which is how volume is read and typed; the
  // engine's own scale is linear amplitude.
  const setVolumePercent = useCallback(
    (percent: number) => {
      if (!state) return;
      const volume = percent / 100;
      onChange({ ...state, volume });
      invoke("set_master_volume", { volume }).catch((e) =>
        console.error("could not set volume:", e),
      );
    },
    [state, onChange],
  );

  return (
    <div className="flex items-center gap-2">
      <button
        onClick={() => void play()}
        title="Run (⌘,)"
        className="rounded-md p-1.5 text-xs text-neutral-400 transition-colors hover:bg-neutral-800 hover:text-blue-400"
      >
        <PlayIcon className="h-6 w-6" />
        play
      </button>
      <button
        onClick={() => void stop()}
        title="Stop (⌘.)"
        className="rounded-md p-1.5 text-xs text-neutral-400 transition-colors hover:bg-neutral-800 hover:text-red-400"
      >
        <StopIcon className="h-6 w-6" />
        stop
      </button>

      {/* Nothing to show until the engine has told us where the controls sit —
          a slider with a made-up value on it would be a lie about the sound. */}
      {state && (
        <>
          <ValueControl
            label="tempo"
            value={state.bpm}
            min={MIN_BPM}
            max={MAX_BPM}
            step={1}
            unit=" bpm"
            onChange={setBpm}
          />
          <ValueControl
            label="volume"
            value={state.volume * 100}
            min={0}
            max={100}
            step={1}
            unit="%"
            onChange={setVolumePercent}
          />
        </>
      )}
    </div>
  );
}

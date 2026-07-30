import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import PlayIcon from "../assets/play.svg?react";
import StopIcon from "../assets/stop.svg?react";
import { ValueControl } from "./ValueControl";

const MIN_BPM = 40;
const MAX_BPM = 240;

/** The engine's global controls, as the backend reports them. */
interface TransportState {
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
 * values are read from it on mount rather than assumed. Each edit goes
 * straight out: sends are cheap enough to fire on every frame of a drag, and
 * the audio side is built to take them mid-performance.
 */
export function Transport({ play, stop }: TransportProps) {
  const [transport, setTransport] = useState<TransportState | null>(null);

  useEffect(() => {
    let live = true;
    invoke<TransportState>("transport")
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

  // The control works in percent, which is how volume is read and typed; the
  // engine's own scale is linear amplitude.
  const setVolumePercent = useCallback((percent: number) => {
    const volume = percent / 100;
    setTransport((t) => (t ? { ...t, volume } : t));
    invoke("set_master_volume", { volume }).catch((e) =>
      console.error("could not set volume:", e),
    );
  }, []);

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
      {transport && (
        <>
          <ValueControl
            label="tempo"
            value={transport.bpm}
            min={MIN_BPM}
            max={MAX_BPM}
            step={1}
            unit=" bpm"
            onChange={setBpm}
          />
          <ValueControl
            label="volume"
            value={transport.volume * 100}
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

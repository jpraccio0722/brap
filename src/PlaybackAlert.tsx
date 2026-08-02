import { useEffect, useRef } from "react";
import { STAGE_LABEL, type Diagnostic } from "./diagnostics";

interface PlaybackAlertProps {
  /** The failure to announce, or null when there is nothing to announce. */
  diagnostic: Diagnostic | null;
  onDismiss: () => void;
}

/**
 * What a failure *during playback* looks like.
 *
 * Everything else that can go wrong is answered by the problems panel, which is
 * enough because everything else is something you just asked for: a run refused
 * is a refusal to the button you pressed. This is not — the scheduler builds a
 * voice per note, cycles after the eval that bound it, so an instrument with a
 * typo in a branch nothing reached yet fails while you are looking somewhere
 * else entirely. It used to fail into the console: one line per step, silently
 * dropping the notes it named, with a pattern that looked like it was playing.
 *
 * So it is said here, over the editor, where it cannot be missed — and the
 * backend has already stopped, because a pattern that cannot make a sound is
 * not a performance to be left running. The panel keeps the same diagnostic
 * afterwards; this is the part that goes away once it has been read.
 */
export function PlaybackAlert({ diagnostic, onDismiss }: PlaybackAlertProps) {
  // Escape closes it, like any other dismissible thing on screen. The handler
  // reaches the current callback through a ref so the listener is bound once
  // and cannot miss a key while it is being swapped.
  const dismiss = useRef(onDismiss);
  dismiss.current = onDismiss;
  const showing = diagnostic !== null;

  useEffect(() => {
    if (!showing) return;
    function onKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape") dismiss.current();
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [showing]);

  if (!diagnostic) return null;

  return (
    // Over the editor but not across it: a performance is stopped, not ruined,
    // and the code that has to be fixed should stay readable behind this.
    <div
      role="alert"
      aria-live="assertive"
      className="pointer-events-none absolute inset-x-0 top-3 z-50 flex justify-center px-4"
    >
      <div className="pointer-events-auto w-full max-w-xl rounded-lg border border-red-800 bg-neutral-950/95 shadow-xl shadow-black/40">
        <div className="flex items-start gap-3 px-4 py-3">
          <span className="mt-0.5 shrink-0 rounded bg-red-950 px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wide text-red-300">
            {STAGE_LABEL[diagnostic.stage]}
          </span>
          <div className="min-w-0 flex-1">
            <p className="text-sm font-medium text-red-300">
              Playback stopped
            </p>
            <p className="mt-1 break-words font-mono text-xs leading-snug text-neutral-200">
              {diagnostic.message}
            </p>
            <p className="mt-2 text-xs text-neutral-500">
              Fix it and run again with{" "}
              <span className="font-mono text-neutral-400">⌘,</span>
            </p>
          </div>
          <button
            onClick={onDismiss}
            title="Dismiss (Esc)"
            className="shrink-0 rounded p-1 text-neutral-500 transition-colors hover:bg-neutral-800 hover:text-neutral-100"
          >
            <svg viewBox="0 0 24 24" fill="currentColor" className="h-3.5 w-3.5">
              <path d="M18.3 5.7 12 12l6.3 6.3-1.4 1.4L10.6 13.4 4.3 19.7 2.9 18.3 9.2 12 2.9 5.7 4.3 4.3l6.3 6.3 6.3-6.3z" />
            </svg>
          </button>
        </div>
      </div>
    </div>
  );
}

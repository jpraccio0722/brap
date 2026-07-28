import { STAGE_LABEL, type Diagnostic } from "./diagnostics";

/** What the last run came to. `idle` is a session that has not run anything —
 *  distinct from a clean run, which is worth saying out loud. */
export type RunStatus = "idle" | "ok" | "error";

interface ProblemsPanelProps {
  open: boolean;
  /** Pixels wide, as the drag handle has left it. */
  width: number;
  /** Grab handle for that drag, drawn along the panel's trailing edge. */
  onResizeStart: (e: React.PointerEvent) => void;
  status: RunStatus;
  diagnostics: Diagnostic[];
  /** The tab the diagnostics came from, for the note below — the file being
   *  looked at is not always the file that failed. */
  sourceTitle: string | null;
  /** True when that tab is the one on screen. */
  sourceIsActive: boolean;
  /** Go to a diagnostic: switch to the tab it came from and put the cursor on
   *  it. Only called for diagnostics that carry a line. */
  onReveal: (diagnostic: Diagnostic) => void;
}

/**
 * A caret under the column an error points at.
 *
 * Built from the snippet rather than from spaces so the caret lands under the
 * right character when the line is indented with tabs — copying the original's
 * whitespace keeps the two lines in step whatever a tab renders as.
 */
function caretLine(snippet: string, column: number): string {
  const upto = snippet.slice(0, Math.max(column - 1, 0));
  return [...upto].map((ch) => (ch === "\t" ? "\t" : " ")).join("") + "^";
}

function Problem({
  diagnostic,
  onReveal,
}: {
  diagnostic: Diagnostic;
  onReveal: (diagnostic: Diagnostic) => void;
}) {
  const { stage, message, line, column, snippet } = diagnostic;
  const locatable = line !== null;

  const body = (
    <>
      <div className="flex items-baseline gap-2">
        <span className="rounded bg-red-950 px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wide text-red-300">
          {STAGE_LABEL[stage]}
        </span>
        {locatable && (
          <span className="font-mono text-xs text-neutral-400">
            line {line}
            {column !== null && `, col ${column}`}
          </span>
        )}
      </div>

      <p className="mt-1.5 text-sm leading-snug text-neutral-200">{message}</p>

      {/* The source as it was when it ran, which is not necessarily what the
          editor holds now — so it is shown rather than pointed at. */}
      {snippet !== null && snippet.trim() !== "" && (
        <pre className="mt-2 overflow-x-auto rounded bg-neutral-950 p-2 font-mono text-xs leading-tight text-neutral-400">
          <code>
            {snippet}
            {column !== null && `\n${caretLine(snippet, column)}`}
          </code>
        </pre>
      )}
    </>
  );

  // A diagnostic with nowhere to go is not a button: the lowerer walks an AST
  // that carries no spans, and offering a click that cannot move the cursor
  // would be a promise the compiler has not made.
  return locatable ? (
    <button
      onClick={() => onReveal(diagnostic)}
      title="Go to this line"
      className="w-full border-b border-neutral-800 px-4 py-3 text-left transition-colors hover:bg-neutral-900"
    >
      {body}
    </button>
  ) : (
    <div className="border-b border-neutral-800 px-4 py-3">{body}</div>
  );
}

/**
 * Everything the last run refused, on the side of the window the code is on.
 *
 * A failed run used to be silent: `run_code` returned its error to a caller
 * that awaited it and did nothing, so a typo simply left the old sound playing
 * with no sign that the new program had never started. This is where that goes
 * now.
 *
 * The panel outlives the run — it keeps showing the last failure while it is
 * being fixed, and is only replaced by the next run's verdict. Like the
 * transport it stays mounted when hidden, so the hamburger is instant.
 */
export function ProblemsPanel({
  open,
  width,
  onResizeStart,
  status,
  diagnostics,
  sourceTitle,
  sourceIsActive,
  onReveal,
}: ProblemsPanelProps) {
  return (
    <aside
      style={{ width }}
      className={
        "relative shrink-0 flex-col border-r border-neutral-800 bg-neutral-950/40 " +
        (open ? "flex" : "hidden")
      }
    >
      <div className="flex items-baseline justify-between border-b border-neutral-800 px-4 py-2">
        <h2 className="text-xs font-semibold uppercase tracking-wide text-neutral-400">
          Problems
        </h2>
        {diagnostics.length > 0 && (
          <span className="font-mono text-xs text-red-400">{diagnostics.length}</span>
        )}
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto">
        {diagnostics.length === 0 ? (
          <div className="px-4 py-4 text-xs leading-relaxed text-neutral-500">
            {status === "ok" ? (
              <p className="text-emerald-500">Compiled. Nothing to report.</p>
            ) : (
              <p>
                Nothing has failed yet. Run with{" "}
                <span className="font-mono text-neutral-400">⌘,</span> and
                anything the compiler refuses shows up here.
              </p>
            )}
          </div>
        ) : (
          <>
            {/* Which file failed only needs saying when it is not the one being
                looked at — otherwise it is a label on the obvious. */}
            {!sourceIsActive && sourceTitle !== null && (
              <p className="border-b border-neutral-800 px-4 py-2 text-xs text-neutral-500">
                from <span className="text-neutral-300">{sourceTitle}</span>
              </p>
            )}
            {diagnostics.map((diagnostic, i) => (
              <Problem
                key={`${diagnostic.stage}-${diagnostic.line}-${i}`}
                diagnostic={diagnostic}
                onReveal={onReveal}
              />
            ))}
          </>
        )}
      </div>

      {/* Mirrors the transport's handle, on the edge that faces the editor. */}
      <div
        onPointerDown={onResizeStart}
        title="Drag to resize"
        role="separator"
        aria-orientation="vertical"
        className="absolute inset-y-0 -right-1 z-10 w-2 cursor-col-resize hover:bg-emerald-600/40 active:bg-emerald-600/60"
      />
    </aside>
  );
}

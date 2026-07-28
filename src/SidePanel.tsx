import type { ReactNode } from "react";

/** Which of the panel's two views is on top. */
export type SideTab = "problems" | "project";

interface SidePanelProps {
  open: boolean;
  /** Pixels wide, as the drag handle has left it. */
  width: number;
  /** Grab handle for that drag, drawn along the panel's trailing edge. */
  onResizeStart: (e: React.PointerEvent) => void;
  tab: SideTab;
  onTabChange: (tab: SideTab) => void;
  /** Drawn on the Problems tab, so a failure is visible from the other one. */
  problemCount: number;
  problems: ReactNode;
  project: ReactNode;
}

function Tab({
  label,
  selected,
  count,
  onClick,
}: {
  label: string;
  selected: boolean;
  /** A badge, when there is a number worth carrying on the tab itself. */
  count?: number;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      role="tab"
      aria-selected={selected}
      className={
        "flex items-center gap-1.5 border-b-2 px-3 py-2 text-xs font-semibold uppercase tracking-wide transition-colors " +
        (selected
          ? "border-emerald-600 text-neutral-100"
          : "border-transparent text-neutral-500 hover:text-neutral-300")
      }
    >
      {label}
      {count !== undefined && count > 0 && (
        <span className="font-mono text-[10px] normal-case text-red-400">{count}</span>
      )}
    </button>
  );
}

/**
 * The left-hand panel: the last run's problems, and the project's files.
 *
 * Both views stay mounted and the hidden one is only taken off screen, so
 * switching tabs keeps what each was showing — the folders opened in the tree
 * are worth as much as the errors above them, and rebuilding either on every
 * click would be a tax on looking at the other.
 */
export function SidePanel({
  open,
  width,
  onResizeStart,
  tab,
  onTabChange,
  problemCount,
  problems,
  project,
}: SidePanelProps) {
  return (
    <aside
      style={{ width }}
      className={
        "relative shrink-0 flex-col border-r border-neutral-800 bg-neutral-950/40 " +
        (open ? "flex" : "hidden")
      }
    >
      <div role="tablist" className="flex items-stretch border-b border-neutral-800">
        <Tab
          label="Problems"
          selected={tab === "problems"}
          count={problemCount}
          onClick={() => onTabChange("problems")}
        />
        <Tab
          label="Project"
          selected={tab === "project"}
          onClick={() => onTabChange("project")}
        />
      </div>

      <div
        className={
          "min-h-0 flex-1 flex-col overflow-y-auto " +
          (tab === "problems" ? "flex" : "hidden")
        }
      >
        {problems}
      </div>
      <div
        className={
          "min-h-0 flex-1 flex-col overflow-y-auto " +
          (tab === "project" ? "flex" : "hidden")
        }
      >
        {project}
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

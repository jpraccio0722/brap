import type { ReactNode } from "react";
import { PanelTab, PanelTabs } from "./PanelTabs";

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
      <PanelTabs>
        <PanelTab
          label="Problems"
          selected={tab === "problems"}
          count={problemCount}
          onClick={() => onTabChange("problems")}
        />
        <PanelTab
          label="Project"
          selected={tab === "project"}
          onClick={() => onTabChange("project")}
        />
      </PanelTabs>

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

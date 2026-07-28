import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

/** One entry in a directory, as the backend's `list_dir` reports it. */
export interface Entry {
  name: string;
  /** Absolute, so opening it needs nothing but the click. */
  path: string;
  isDir: boolean;
}

/** Extract the file name from an absolute path (cross-platform). */
function basename(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() || path;
}

/** How far in each level of the tree sits, in pixels. */
const INDENT = 12;

function Chevron({ open }: { open: boolean }) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="currentColor"
      className={
        "h-3 w-3 shrink-0 text-neutral-500 transition-transform " +
        (open ? "rotate-90" : "")
      }
    >
      <path d="M9 6l6 6-6 6z" />
    </svg>
  );
}

function FileIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="currentColor" className="h-3.5 w-3.5 shrink-0 text-neutral-600">
      <path d="M6 2h7l5 5v15H6V2zm7 1.5V8h4.5L13 3.5z" />
    </svg>
  );
}

/**
 * The contents of one directory.
 *
 * Mounted only while its folder is open, which is what makes the tree lazy: a
 * project with a `target/` or `node_modules/` in it costs nothing until
 * somebody looks inside. Collapsing throws the listing away, so reopening a
 * folder shows what is on disk now rather than what was there an hour ago.
 */
function Children({
  path,
  depth,
  activePath,
  onOpenFile,
}: {
  path: string;
  depth: number;
  activePath: string | null;
  onOpenFile: (path: string) => void;
}) {
  const [entries, setEntries] = useState<Entry[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    setEntries(null);
    setError(null);
    invoke<Entry[]>("list_dir", { path })
      .then((e) => {
        if (live) setEntries(e);
      })
      .catch((e) => {
        // A directory that won't list is this row's problem, not the
        // program's, so it is said here rather than in the problems panel.
        if (live) setError(String(e));
      });
    return () => {
      live = false;
    };
  }, [path]);

  const pad = { paddingLeft: 8 + depth * INDENT };

  if (error !== null) {
    return (
      <p style={pad} className="py-1 pr-2 text-xs leading-snug text-red-400">
        {error}
      </p>
    );
  }
  if (entries === null) {
    return (
      <p style={pad} className="py-1 pr-2 text-xs text-neutral-600">
        loading…
      </p>
    );
  }
  if (entries.length === 0) {
    return (
      <p style={pad} className="py-1 pr-2 text-xs italic text-neutral-600">
        empty
      </p>
    );
  }

  return (
    <>
      {entries.map((entry) => (
        <Row
          key={entry.path}
          entry={entry}
          depth={depth}
          activePath={activePath}
          onOpenFile={onOpenFile}
        />
      ))}
    </>
  );
}

/** One line of the tree: a file to open, or a folder to expand. */
function Row({
  entry,
  depth,
  activePath,
  onOpenFile,
}: {
  entry: Entry;
  depth: number;
  activePath: string | null;
  onOpenFile: (path: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const isActive = !entry.isDir && entry.path === activePath;

  return (
    <>
      <button
        onClick={() => (entry.isDir ? setOpen((o) => !o) : onOpenFile(entry.path))}
        title={entry.path}
        aria-expanded={entry.isDir ? open : undefined}
        style={{ paddingLeft: 8 + depth * INDENT }}
        className={
          "flex w-full items-center gap-1.5 py-0.5 pr-2 text-left text-xs transition-colors " +
          (isActive
            ? "bg-neutral-800 text-neutral-100"
            : "text-neutral-300 hover:bg-neutral-900")
        }
      >
        {entry.isDir ? <Chevron open={open} /> : <FileIcon />}
        <span className="truncate">{entry.name}</span>
      </button>
      {entry.isDir && open && (
        <Children
          path={entry.path}
          depth={depth + 1}
          activePath={activePath}
          onOpenFile={onOpenFile}
        />
      )}
    </>
  );
}

interface ProjectPanelProps {
  /** The project's folder, or null before the backend has named one. */
  root: string | null;
  /** The file the editor is showing, so the tree can mark it. */
  activePath: string | null;
  /** Open a file in a tab. */
  onOpenFile: (path: string) => void;
  /** Throw the listings away and read the folder again. */
  onRefresh: () => void;
}

/**
 * Every file in the project, which is simply a folder on disk.
 *
 * It starts on the directory the app was launched from and follows whatever
 * File ▸ New Project… picks after that. There is no project file and nothing
 * to configure: a project is a root path, and this is a view of it.
 */
export function ProjectPanel({ root, activePath, onOpenFile, onRefresh }: ProjectPanelProps) {
  if (root === null) {
    return (
      <div className="px-4 py-4 text-xs leading-relaxed text-neutral-500">
        <p>
          No project open. Pick a folder with{" "}
          <span className="text-neutral-400">File ▸ New Project…</span> and its
          files show up here.
        </p>
      </div>
    );
  }

  return (
    <>
      <div className="flex items-center justify-between gap-2 border-b border-neutral-800 px-3 py-1.5">
        <span
          title={root}
          className="truncate text-xs font-medium uppercase tracking-wide text-neutral-300"
        >
          {basename(root)}
        </span>
        {/* Nothing watches the filesystem, so a folder changed from outside the
            app needs asking for. Collapsing and reopening does it for one
            folder; this does it for the lot. */}
        <button
          onClick={onRefresh}
          title="Refresh"
          className="shrink-0 rounded p-1 text-neutral-500 transition-colors hover:bg-neutral-800 hover:text-neutral-200"
        >
          <svg viewBox="0 0 24 24" fill="currentColor" className="h-3.5 w-3.5">
            <path d="M17.65 6.35A8 8 0 1 0 19.73 14h-2.08A6 6 0 1 1 16.24 7.76L13 11h7V4l-2.35 2.35z" />
          </svg>
        </button>
      </div>

      <div className="py-1">
        <Children path={root} depth={0} activePath={activePath} onOpenFile={onOpenFile} />
      </div>
    </>
  );
}

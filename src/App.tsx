import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import CodeMirror from "@uiw/react-codemirror";
import type { EditorView } from "@codemirror/view";
import {
  screeExtensions,
  revealPosition,
  showErrorLines,
  EMPTY_METADATA,
  loadMetadata,
  type LanguageMetadata,
} from "./scree";
import { DocsPanel, type DocsFocus } from "./DocsPanel";
import { ProblemsPanel, type RunStatus } from "./ProblemsPanel";
import { ProjectPanel } from "./ProjectPanel";
import { RightPanel, type RightTab } from "./RightPanel";
import { SidePanel, type SideTab } from "./SidePanel";
import { toDiagnostic, type Diagnostic } from "./diagnostics";
import { TransportPanel } from "./TransportPanel";
import { fromWire, toWire, type GraphicalPattern, type WirePattern } from "./patterns";
import { Transport } from "./component/Transport";


/** A project's drawn patterns, as `read_patterns` returns them. */
interface PatternsFile {
  /** Absolute path of the project's `patterns.scree`. */
  path: string;
  patterns: WirePattern[];
}

/** How long the panel waits before writing a change out. Long enough that
 *  dragging across a row is one write, short enough to be over before anyone
 *  reaches for the play key. */
const PATTERN_SAVE_DELAY = 400;

interface Tab {
  id: string;
  /** Display name in the tab bar. */
  title: string;
  /** Absolute path on disk, or null if the tab has never been saved. */
  path: string | null;
  content: string;
  /** True when content differs from what's on disk. */
  dirty: boolean;
}

const SCREE_FILTER = [{ name: "scree", extensions: ["scree"] }];

/** How wide either side panel may be dragged. The floor is what a pattern's
 *  grid and the sliders need; the ceiling keeps the editor from vanishing. */
const MIN_PANEL = 240;
const MAX_PANEL = 720;
const DEFAULT_PANEL = 288;
/** The left panel holds wrapped prose, a snippet and a file tree, all of which
 *  want a little more room than the transport's sliders do. */
const DEFAULT_SIDE_PANEL = 340;

/**
 * Drag a panel's inner edge.
 *
 * Pointer capture is what makes this reliable: the pointer leaves the 8px
 * handle on the first frame of any real drag, and without capture the moves
 * would be delivered to whatever it crossed — including the editor, which
 * would start selecting text.
 *
 * @param edge Which side of the window the panel is anchored to, which decides
 * whether its width grows or shrinks as the pointer moves right.
 */
function usePanelResize(edge: "left" | "right", setWidth: (width: number) => void) {
  return useCallback(
    (e: React.PointerEvent) => {
      e.preventDefault();
      const handle = e.currentTarget as HTMLElement;
      handle.setPointerCapture(e.pointerId);

      const onMove = (ev: PointerEvent) => {
        const width =
          edge === "left" ? ev.clientX : window.innerWidth - ev.clientX;
        setWidth(Math.min(MAX_PANEL, Math.max(MIN_PANEL, width)));
      };
      const onUp = () => {
        handle.removeEventListener("pointermove", onMove);
        handle.removeEventListener("pointerup", onUp);
        handle.removeEventListener("pointercancel", onUp);
      };

      handle.addEventListener("pointermove", onMove);
      handle.addEventListener("pointerup", onUp);
      handle.addEventListener("pointercancel", onUp);
    },
    [edge, setWidth],
  );
}

const STARTER_CONTENT = `// Write some code, then hit Play (or ⌘,)`;

let tabCounter = 0;
function makeTab(overrides: Partial<Tab> = {}): Tab {
  tabCounter += 1;
  return {
    id: `tab-${tabCounter}`,
    title: `untitled-${tabCounter}.scree`,
    path: null,
    content: "",
    dirty: false,
    ...overrides,
  };
}

/** Extract the file name from an absolute path (cross-platform). */
function basename(path: string): string {
  return path.split(/[\\/]/).pop() || path;
}

function App() {
  const [tabs, setTabs] = useState<Tab[]>(() => [
    makeTab({ content: STARTER_CONTENT }),
  ]);
  const [activeId, setActiveId] = useState<string | null>(() => tabs[0].id);
  const [panelOpen, setPanelOpen] = useState(true);
  const [panelWidth, setPanelWidth] = useState(DEFAULT_PANEL);
  // The right panel opens on the transport: it is the tab with the buttons the
  // app is played from. The reference is always beside it, and a ⌘-click in
  // the editor is what usually brings it up.
  const [panelTab, setPanelTab] = useState<RightTab>("transport");
  const [docsFocus, setDocsFocus] = useState<DocsFocus | null>(null);

  // The left panel starts shut, on its problems tab: there is nothing to say
  // until something has been run. A failed run opens it, which is the whole
  // point of it.
  const [sideOpen, setSideOpen] = useState(false);
  const [sideWidth, setSideWidth] = useState(DEFAULT_SIDE_PANEL);
  const [sideTab, setSideTab] = useState<SideTab>("project");
  const [diagnostics, setDiagnostics] = useState<Diagnostic[]>([]);
  const [runStatus, setRunStatus] = useState<RunStatus>("idle");
  // Which tab the diagnostics describe. They outlive the run, and the editor
  // may well have moved to another file by the time they are read.
  const [sourceTabId, setSourceTabId] = useState<string | null>(null);

  // Drawn patterns belong to the project rather than to any one tab: they live
  // in its `patterns.scree`, which every file in the project can name without
  // importing. The panel is that file's editor — it reads it when a project
  // opens and writes it as you draw.
  const [patterns, setPatterns] = useState<GraphicalPattern[]>([]);
  // Where that file is, as the backend composes it. Kept so the panel can
  // recognise the file when it is opened as an ordinary tab.
  const [patternsPath, setPatternsPath] = useState<string | null>(null);
  // The project whose patterns the panel is currently holding, and what that
  // project's file says. Together they are the guard on writing: the panel
  // never writes to a project it has not successfully read, so a file it could
  // not parse is left alone rather than overwritten with an empty grid.
  const patternsFrom = useRef<string | null>(null);
  const patternsOnDisk = useRef<string | null>(null);

  // A project is a folder and nothing else. It opens on the directory the app
  // was launched from and follows whatever File ▸ New Project… picks after
  // that. Null only until the backend has answered.
  const [projectRoot, setProjectRoot] = useState<string | null>(null);
  // Bumped to throw the tree's listings away and read the folder again —
  // nothing watches the filesystem, so a change made outside the app has to be
  // asked for.
  const [projectVersion, setProjectVersion] = useState(0);

  const startResize = usePanelResize("right", setPanelWidth);
  const startSideResize = usePanelResize("left", setSideWidth);

  // Null when every tab has been closed, which the editor is built to show.
  const activeTab = tabs.find((t) => t.id === activeId) ?? null;

  /** Show a failure. Everything that can fail comes through here, so nothing
   *  can fail quietly — which is what this panel exists to prevent. */
  const report = useCallback((diagnostic: Diagnostic, tabId: string | null) => {
    setDiagnostics([diagnostic]);
    setRunStatus("error");
    setSourceTabId(tabId);
    // Both, since the panel may well be open on the project tree instead: an
    // open panel showing something else is as silent as a shut one.
    setSideOpen(true);
    setSideTab("problems");
  }, []);

  // The language's builtins, fetched once. Until it arrives the editor runs on
  // EMPTY_METADATA: syntax highlighting is already correct, only the builtin
  // colouring and completions are missing.
  const [metadata, setMetadata] = useState<LanguageMetadata>(EMPTY_METADATA);
  useEffect(() => {
    let live = true;
    loadMetadata()
      .then((meta) => {
        if (live) setMetadata(meta);
      })
      .catch((e) => console.error("could not load language metadata:", e));
    return () => {
      live = false;
    };
  }, []);

  // The default project, fetched once. Only the initial value: a folder picked
  // from the File menu afterwards is the frontend's own, and must not be
  // overwritten by a late answer.
  useEffect(() => {
    let live = true;
    invoke<string>("project_root")
      .then((root) => {
        if (live) setProjectRoot((current) => current ?? root);
      })
      .catch((e) => console.error("could not read the working directory:", e));
    return () => {
      live = false;
    };
  }, []);

  // Which project the panel's rows belong to, for the load below to check
  // against when it finally answers: opening two projects quickly must not
  // leave the first one's answer on screen.
  const projectRootRef = useRef(projectRoot);
  projectRootRef.current = projectRoot;

  /**
   * Read a project's drawn patterns into the panel.
   *
   * A project with no patterns file simply has none. A file that cannot be
   * read is a failure worth showing, and it also stops the panel writing:
   * showing an empty grid over a file we could not parse, and then saving that
   * grid over it, is how somebody's work disappears.
   */
  const loadPatterns = useCallback(
    async (root: string) => {
      try {
        const file = await invoke<PatternsFile>("read_patterns", { root });
        if (projectRootRef.current !== root) return; // the project moved on
        setPatternsPath(file.path);
        setPatterns(fromWire(file.patterns));
        patternsOnDisk.current = JSON.stringify(file.patterns);
        patternsFrom.current = root;
      } catch (e) {
        if (projectRootRef.current !== root) return;
        // Cleared rather than left showing: rows from another project would
        // be played by this one, since an eval sends whatever the panel holds.
        setPatterns([]);
        patternsFrom.current = null;
        report(toDiagnostic(e, "could not read this project's patterns"), null);
      }
    },
    [report],
  );

  // On open, and again whenever the project tree is refreshed — which is the
  // way to pick up a patterns file changed outside the app.
  useEffect(() => {
    if (projectRoot === null) return;
    void loadPatterns(projectRoot);
  }, [projectRoot, projectVersion, loadPatterns]);

  // And back out again as the panel is drawn in. Debounced, because dragging
  // across a row is a dozen state changes and one edit.
  useEffect(() => {
    if (projectRoot === null || patternsFrom.current !== projectRoot) return;

    const wire = toWire(patterns);
    const json = JSON.stringify(wire);
    if (json === patternsOnDisk.current) return; // the file already says this

    const timer = setTimeout(() => {
      invoke<string>("write_patterns", { root: projectRoot, patterns: wire })
        .then((path) => {
          patternsOnDisk.current = json;
          setPatternsPath(path);
        })
        // Left un-synced on purpose, so the next change tries again. The music
        // is unaffected: an eval sends the panel's rows with the code.
        .catch((e) => report(toDiagnostic(e, "could not save the patterns"), null));
    }, PATTERN_SAVE_DELAY);

    return () => clearTimeout(timer);
  }, [patterns, projectRoot, report]);

  // The completion source reads the drawn patterns through a ref so a rename
  // in the panel shows up in the next completion without touching the
  // extension array — whose identity must stay stable, since CodeMirror
  // reconfigures whenever it changes.
  const patternsRef = useRef(patterns);
  patternsRef.current = patterns;
  const patternNames = useCallback(
    () => toWire(patternsRef.current).map((p) => p.name),
    [],
  );

  /**
   * Show a builtin in the reference, from a ⌘-click on its name in the editor.
   *
   * Opens the panel as well as switching it: a panel that is shut is as silent
   * as one showing the transport, and the click asked to read something.
   *
   * Identity-stable, like `patternNames` and for the same reason — it is built
   * into the extension array, which CodeMirror reconfigures from.
   */
  const openDocs = useCallback((name: string) => {
    setPanelOpen(true);
    setPanelTab("docs");
    // The nonce is what makes a second click on the same name scroll to it
    // again, after it has been scrolled away from.
    setDocsFocus((prev) => ({ name, nonce: (prev?.nonce ?? 0) + 1 }));
  }, []);

  const extensions = useMemo(
    () => screeExtensions(metadata, patternNames, openDocs),
    [metadata, patternNames, openDocs],
  );

  const newTab = useCallback(() => {
    const tab = makeTab();
    setTabs((prev) => [...prev, tab]);
    setActiveId(tab.id);
  }, []);

  const closeTab = useCallback(
    (id: string) => {
      const idx = tabs.findIndex((t) => t.id === id);
      if (idx === -1) return;

      const next = tabs.filter((t) => t.id !== id);
      setTabs(next);

      // Closing the active tab hands the editor to a neighbour; closing the
      // last one leaves nothing to hand it to, and the empty state shows.
      if (id === activeId) {
        setActiveId(next.length === 0 ? null : next[Math.min(idx, next.length - 1)].id);
      }
    },
    [tabs, activeId],
  );

  const updateContent = useCallback((id: string, content: string) => {
    setTabs((prev) =>
      prev.map((t) =>
        t.id === id ? { ...t, content, dirty: true } : t,
      ),
    );
  }, []);

  /** Open a file by path, wherever the path came from — the open dialog, or a
   *  click in the project tree. */
  const openPath = useCallback(
    async (path: string) => {
      // If the file is already open, just focus its tab.
      const existing = tabs.find((t) => t.path === path);
      if (existing) {
        setActiveId(existing.id);
        return;
      }

      try {
        const content = await invoke<string>("read_file", { path });
        const tab = makeTab({ title: basename(path), path, content, dirty: false });
        setTabs((prev) => [...prev, tab]);
        setActiveId(tab.id);
      } catch (e) {
        // Anything that isn't text lands here, which is the honest answer: the
        // editor has nothing to show for a binary file.
        report(toDiagnostic(e, `could not open ${basename(path)}`), null);
      }
    },
    [tabs, report],
  );

  const openTab = useCallback(async () => {
    try {
      const selected = await open({ multiple: false, filters: SCREE_FILTER });
      if (!selected || typeof selected !== "string") return; // user cancelled
      await openPath(selected);
    } catch (e) {
      report(toDiagnostic(e, "could not open that file"), null);
    }
  }, [openPath, report]);

  /**
   * Point the project panel at another folder.
   *
   * There is no project file to create, so "new" is a folder chooser — and the
   * platform's own dialog is where a new folder gets made, which is why this
   * needs nothing else. Open tabs are left alone: they are files, and a file
   * does not stop being open because the tree beside it moved.
   */
  const newProject = useCallback(async () => {
    try {
      const selected = await open({ directory: true, multiple: false });
      if (!selected || typeof selected !== "string") return; // user cancelled
      setProjectRoot(selected);
      setProjectVersion((v) => v + 1);
      setSideOpen(true);
      setSideTab("project");
    } catch (e) {
      report(toDiagnostic(e, "could not open that folder"), null);
    }
  }, [report]);

  /**
   * Evaluate the active tab.
   *
   * A refusal from any compiler pass arrives here as a rejection, and every
   * one of them ends up in the problems panel: this used to be an unhandled
   * `await`, so a program with a typo in it simply never started while the
   * previous one kept playing, with nothing on screen to say why.
   */
  const play = useCallback(async () => {
    // Nothing open is nothing to run — the transport's buttons stay live for
    // stop, which is about what the engine is holding, not about a tab.
    if (!activeTab) return;
    const tabId = activeTab.id;
    try {
      // The drawn patterns go with the code: they are bindings the program can
      // name, and an eval is the only moment they mean anything.
      //
      // So does the workspace, for the same reason: a `use` resolves against
      // the folder this tab is saved in. What it finds there is what is on
      // disk, so a module edited in another tab is heard once it is saved.
      await invoke("run_code", {
        code: activeTab.content,
        // Null while the panel has nothing to say about this project — before
        // its file has been read, or after a read that failed. An empty panel
        // that *has* read the project is an empty list, and means it: only
        // that may hide a patterns file sitting on disk.
        patterns: patternsFrom.current === projectRoot ? toWire(patterns) : null,
        workspace: { path: activeTab.path, root: projectRoot },
      });
      setDiagnostics([]);
      setRunStatus("ok");
      setSourceTabId(tabId);
    } catch (e) {
      report(toDiagnostic(e), tabId);
    }
  }, [activeTab, patterns, projectRoot, report]);

  const stop = useCallback(async () => {
    try {
      await invoke("stop_audio");
    } catch (e) {
      report(toDiagnostic(e, "could not stop the engine"), null);
    }
  }, [report]);

  const saveTab = useCallback(async () => {
    const tab = activeTab;
    if (!tab) return;
    try {
      let path = tab.path;
      if (!path) {
        // First save: ask where to put it, defaulting to a .scree file.
        path = await save({ defaultPath: tab.title, filters: SCREE_FILTER });
        if (!path) return; // user cancelled
      }
      await invoke("save_file", { path, content: tab.content });
      const savedPath = path;
      setTabs((prev) =>
        prev.map((t) =>
          t.id === tab.id
            ? { ...t, path: savedPath, title: basename(savedPath), dirty: false }
            : t,
        ),
      );

      // The patterns file has two editors — the panel and, since it is an
      // ordinary scree file, a tab. Saving it in one updates the other, so
      // they cannot drift apart without somebody watching it happen.
      if (savedPath === patternsPath && projectRoot !== null) {
        await loadPatterns(projectRoot);
      }
    } catch (e) {
      // Left dirty on purpose: the tab still differs from what is on disk.
      report(toDiagnostic(e, `could not save ${tab.title}`), tab.id);
    }
  }, [activeTab, patternsPath, projectRoot, loadPatterns, report]);

  // The File menu's items arrive as events. Their handlers take new identities
  // on every keystroke, so the listeners reach them through a ref and subscribe
  // once for the life of the app: re-subscribing is asynchronous, and a menu
  // click landing mid-swap could be heard by both the old listener and the new.
  const fileActions = useRef({ newTab, openTab, saveTab, newProject });
  fileActions.current = { newTab, openTab, saveTab, newProject };

  useEffect(() => {
    const subscriptions = [
      listen("file-new", () => fileActions.current.newTab()),
      listen("file-open", () => void fileActions.current.openTab()),
      listen("file-save", () => void fileActions.current.saveTab()),
      listen("project-new", () => void fileActions.current.newProject()),
    ];
    return () => {
      for (const sub of subscriptions) void sub.then((unlisten) => unlisten());
    };
  }, []);

  // The mounted editor, and the tab it belongs to.
  //
  // State rather than a ref, and tagged with its tab, for two reasons. The
  // view is built in an effect inside CodeMirror, so a ref still reads empty
  // when this component's effects run on a remount — the marks below would
  // never be applied. And the editor remounts per tab, so a view without its
  // tab's name on it is indistinguishable from the one that just closed.
  const [editor, setEditor] = useState<{ tabId: string; view: EditorView } | null>(null);
  const activeView = editor && editor.tabId === activeId ? editor.view : null;

  // Only the ones about the file that was run. A line number from an imported
  // module means nothing in this buffer, and marking it would be pointing at
  // innocent code.
  const errorLines = useMemo(
    () =>
      diagnostics
        .filter((d) => d.file === null)
        .map((d) => d.line)
        .filter((line): line is number => line !== null),
    [diagnostics],
  );

  // Marks belong to the tab that was run, so switching away clears them — the
  // same line number in another file means nothing. Switching back re-applies
  // them, since the remount gives the editor a fresh state.
  useEffect(() => {
    if (!activeView) return;
    showErrorLines(activeView, sourceTabId === activeId ? errorLines : []);
  }, [activeView, errorLines, sourceTabId, activeId]);

  // A click in the panel may have to switch tabs first, and the view for that
  // tab does not exist until it has mounted — so the jump is left pending and
  // made once there is somewhere to jump in.
  const [pendingReveal, setPendingReveal] = useState<Diagnostic | null>(null);

  const revealDiagnostic = useCallback(
    (diagnostic: Diagnostic) => {
      // A diagnostic from an imported module is about that file, so the click
      // opens it. The jump waits for the open: until the tab exists there is
      // no editor to move the cursor in, and moving it in the old one would
      // put it on an unrelated line.
      if (diagnostic.file !== null) {
        void openPath(diagnostic.file).then(() => setPendingReveal(diagnostic));
        return;
      }
      if (sourceTabId && sourceTabId !== activeId) setActiveId(sourceTabId);
      setPendingReveal(diagnostic);
    },
    [openPath, sourceTabId, activeId],
  );

  useEffect(() => {
    if (!pendingReveal || !activeView) return;
    if (pendingReveal.line !== null) {
      revealPosition(activeView, pendingReveal.line, pendingReveal.column);
    }
    setPendingReveal(null);
  }, [pendingReveal, activeView]);

  // ⌘, plays, ⌘. stops. The file shortcuts aren't here: they hang off their
  // menu items, whose accelerators fire before the key reaches the webview.
  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      const mod = e.metaKey || e.ctrlKey;
      if (mod && e.key === ",") {
        e.preventDefault();
        void play();
      } else if (mod && e.key === ".") {
        e.preventDefault();
        void stop();
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [play, stop]);

  return (
    <div className="flex h-screen flex-col bg-neutral-900 text-neutral-100">
      <header className="flex items-center justify-between border-b border-neutral-800 px-4 py-2">
        <div className="flex items-center gap-2">
          {/* On the left, for the panel on the left. Wears the number of
              problems so a run that failed is visible with the panel shut. */}
          <button
            onClick={() => setSideOpen((open) => !open)}
            title={sideOpen ? "Hide panel" : "Show panel"}
            aria-expanded={sideOpen}
            className={
              "relative rounded-md p-1.5 transition-colors " +
              (sideOpen
                ? "bg-neutral-700 text-neutral-100 hover:bg-neutral-600"
                : diagnostics.length > 0
                  ? "text-red-400 hover:bg-neutral-800 hover:text-red-300"
                  : "text-neutral-400 hover:bg-neutral-800 hover:text-neutral-100")
            }
          >
            <svg viewBox="0 0 24 24" fill="currentColor" className="h-4 w-4">
              <path d="M3 6h18v2H3V6zm0 5h18v2H3v-2zm0 5h18v2H3v-2z" />
            </svg>
            {diagnostics.length > 0 && (
              <span className="absolute -right-1 -top-1 min-w-4 rounded-full bg-red-600 px-1 text-center text-[10px] font-semibold leading-4 text-white">
                {diagnostics.length}
              </span>
            )}
          </button>
          <div className="ml-4">
            <Transport 
              play={play}
              stop={stop}
            />
          </div>
        </div>
        <div className="flex items-center gap-2">
          {/* One hamburger, both ways: it is where a reader looks for the
              panel whether it is showing or not. */}
          <button
            onClick={() => setPanelOpen((open) => !open)}
            title={panelOpen ? "Hide right panel" : "Show right panel"}
            aria-expanded={panelOpen}
            className={
              "rounded-md p-1.5 transition-colors " +
              (panelOpen
                ? "bg-neutral-700 text-neutral-100 hover:bg-neutral-600"
                : "text-neutral-400 hover:bg-neutral-800 hover:text-neutral-100")
            }
          >
            <svg viewBox="0 0 24 24" fill="currentColor" className="h-4 w-4">
              <path d="M3 6h18v2H3V6zm0 5h18v2H3v-2zm0 5h18v2H3v-2z" />
            </svg>
          </button>
        </div>
      </header>

      {/* Tab bar */}
      <div className="flex items-stretch border-b border-neutral-800 bg-neutral-950/40">
        <div className="flex flex-1 items-stretch overflow-x-auto">
          {tabs.map((tab) => {
            const isActive = tab.id === activeId;
            return (
              <div
                key={tab.id}
                onClick={() => setActiveId(tab.id)}
                className={
                  "group flex cursor-pointer items-center gap-2 border-r border-neutral-800 px-3 py-1.5 text-sm " +
                  (isActive
                    ? "bg-neutral-900 text-neutral-100"
                    : "bg-neutral-950/40 text-neutral-400 hover:bg-neutral-900/60")
                }
              >
                <span className="whitespace-nowrap">{tab.title}</span>
                {tab.dirty && (
                  <span
                    className="h-1.5 w-1.5 rounded-full bg-neutral-400"
                    title="Unsaved changes"
                  />
                )}
                <button
                  onClick={(e) => {
                    e.stopPropagation();
                    closeTab(tab.id);
                  }}
                  title="Close tab"
                  className="rounded p-0.5 text-neutral-500 opacity-0 transition-opacity hover:bg-neutral-700 hover:text-neutral-100 group-hover:opacity-100"
                >
                  <svg viewBox="0 0 24 24" fill="currentColor" className="h-3.5 w-3.5">
                    <path d="M18.3 5.7 12 12l6.3 6.3-1.4 1.4L10.6 13.4 4.3 19.7 2.9 18.3 9.2 12 2.9 5.7 4.3 4.3l6.3 6.3 6.3-6.3z" />
                  </svg>
                </button>
              </div>
            );
          })}
        </div>
        <button
          onClick={newTab}
          title="New tab (⌘N)"
          className="flex items-center px-3 text-neutral-400 transition-colors hover:bg-neutral-900 hover:text-neutral-100"
        >
          <svg viewBox="0 0 24 24" fill="currentColor" className="h-4 w-4">
            <path d="M11 11V5h2v6h6v2h-6v6h-2v-6H5v-2z" />
          </svg>
        </button>
      </div>

      <div className="flex min-h-0 flex-1">
        <SidePanel
          open={sideOpen}
          width={sideWidth}
          onResizeStart={startSideResize}
          tab={sideTab}
          onTabChange={setSideTab}
          problemCount={diagnostics.length}
          problems={
            <ProblemsPanel
              status={runStatus}
              diagnostics={diagnostics}
              sourceTitle={tabs.find((t) => t.id === sourceTabId)?.title ?? null}
              sourceIsActive={sourceTabId === activeId}
              onReveal={revealDiagnostic}
            />
          }
          project={
            <ProjectPanel
              // Remounts the tree, which is how a refresh throws away every
              // listing it is holding.
              key={projectVersion}
              root={projectRoot}
              activePath={activeTab?.path ?? null}
              onOpenFile={(path) => void openPath(path)}
              onRefresh={() => setProjectVersion((v) => v + 1)}
            />
          }
        />
        <main className="min-h-0 min-w-0 flex-1">
          {activeTab ? (
            <CodeMirror
              key={activeTab.id}
              onCreateEditor={(view) => setEditor({ tabId: activeTab.id, view })}
              value={activeTab.content}
              onChange={(value) => updateContent(activeTab.id, value)}
              height="100%"
              theme="dark"
              extensions={extensions}
              className="h-full text-sm"
            />
          ) : (
            // Closing the last tab is allowed to leave nothing open. The two
            // ways back are both in the File menu, so name them rather than
            // growing a second set of buttons that only exist here.
            <div className="flex h-full flex-col items-center justify-center gap-1 text-sm text-neutral-500">
              <p>No file open</p>
              <p className="text-xs">
                <span className="font-mono text-neutral-400">⌘N</span> for a new
                one, <span className="font-mono text-neutral-400">⌘O</span> to
                open
              </p>
            </div>
          )}
        </main>
        <RightPanel
          open={panelOpen}
          width={panelWidth}
          onResizeStart={startResize}
          tab={panelTab}
          onTabChange={setPanelTab}
          transport={
            <TransportPanel
              patterns={patterns}
              onPatternsChange={setPatterns}
              patternsPath={patternsPath}
            />
          }
          docs={<DocsPanel builtins={metadata.builtins} focus={docsFocus} />}
        />
      </div>
    </div>
  );
}

export default App;

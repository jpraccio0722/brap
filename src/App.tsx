import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import CodeMirror from "@uiw/react-codemirror";
import { javascript } from "@codemirror/lang-javascript";

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

const BRAP_FILTER = [{ name: "brap", extensions: ["brap"] }];

const STARTER_CONTENT = `// Write some code, then hit Play (or ⌘↵)`;

let tabCounter = 0;
function makeTab(overrides: Partial<Tab> = {}): Tab {
  tabCounter += 1;
  return {
    id: `tab-${tabCounter}`,
    title: `untitled-${tabCounter}.brap`,
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
  const [activeId, setActiveId] = useState<string>(() => tabs[0].id);

  const activeTab = tabs.find((t) => t.id === activeId) ?? tabs[0];

  const newTab = useCallback(() => {
    const tab = makeTab();
    setTabs((prev) => [...prev, tab]);
    setActiveId(tab.id);
  }, []);

  const closeTab = useCallback(
    (id: string) => {
      setTabs((prev) => {
        const next = prev.filter((t) => t.id !== id);
        // Never leave the editor with zero tabs.
        if (next.length === 0) {
          const fresh = makeTab({ content: STARTER_CONTENT });
          setActiveId(fresh.id);
          return [fresh];
        }
        // If we closed the active tab, activate a neighbour.
        if (id === activeId) {
          const idx = prev.findIndex((t) => t.id === id);
          setActiveId(next[Math.min(idx, next.length - 1)].id);
        }
        return next;
      });
    },
    [activeId],
  );

  const updateContent = useCallback((id: string, content: string) => {
    setTabs((prev) =>
      prev.map((t) =>
        t.id === id ? { ...t, content, dirty: true } : t,
      ),
    );
  }, []);

  const openTab = useCallback(async () => {
    const selected = await open({ multiple: false, filters: BRAP_FILTER });
    if (!selected || typeof selected !== "string") return; // user cancelled
    const path = selected;

    // If the file is already open, just focus its tab.
    const existing = tabs.find((t) => t.path === path);
    if (existing) {
      setActiveId(existing.id);
      return;
    }

    const content = await invoke<string>("read_file", { path });
    const tab = makeTab({ title: basename(path), path, content, dirty: false });
    setTabs((prev) => [...prev, tab]);
    setActiveId(tab.id);
  }, [tabs]);

  const play = useCallback(async () => {
    // Wired to the (currently empty) Rust backend command.
    await invoke("run_code", { code: activeTab.content });
  }, [activeTab.content]);

  const stop = useCallback(async () => {
    await invoke("stop_audio");
  }, []);

  const saveTab = useCallback(async () => {
    const tab = activeTab;
    let path = tab.path;
    if (!path) {
      // First save: ask where to put it, defaulting to a .brap file.
      path = await save({ defaultPath: tab.title, filters: BRAP_FILTER });
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
  }, [activeTab]);

  // Global shortcuts: ⌘↵ plays, ⌘S saves.
  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      const mod = e.metaKey || e.ctrlKey;
      if (mod && e.key === "Enter") {
        e.preventDefault();
        void play();
      } else if (mod && e.key === ".") {
        e.preventDefault();
        void stop();
      } else if (mod && e.key.toLowerCase() === "s") {
        e.preventDefault();
        void saveTab();
      } else if (mod && e.key.toLowerCase() === "o") {
        e.preventDefault();
        void openTab();
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [play, stop, saveTab, openTab]);

  return (
    <div className="flex h-screen flex-col bg-neutral-900 text-neutral-100">
      <header className="flex items-center justify-between border-b border-neutral-800 px-4 py-2">
        <h1 className="text-sm font-semibold tracking-wide text-neutral-300">brap</h1>
        <div className="flex items-center gap-2">
          <button
            onClick={() => void openTab()}
            title="Open (⌘O)"
            className="inline-flex items-center gap-2 rounded-md bg-neutral-700 px-3 py-1.5 text-sm font-medium text-neutral-100 transition-colors hover:bg-neutral-600 active:bg-neutral-800"
          >
            <svg viewBox="0 0 24 24" fill="currentColor" className="h-4 w-4">
              <path d="M10 4H4a2 2 0 0 0-2 2v12a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-8l-2-2z" />
            </svg>
            Open
            <span className="text-xs text-neutral-400">⌘O</span>
          </button>
          <button
            onClick={() => void saveTab()}
            title="Save (⌘S)"
            className="inline-flex items-center gap-2 rounded-md bg-neutral-700 px-3 py-1.5 text-sm font-medium text-neutral-100 transition-colors hover:bg-neutral-600 active:bg-neutral-800"
          >
            <svg viewBox="0 0 24 24" fill="currentColor" className="h-4 w-4">
              <path d="M17 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2V7l-4-4zm-5 16a3 3 0 1 1 0-6 3 3 0 0 1 0 6zm3-10H5V5h10v4z" />
            </svg>
            Save
            <span className="text-xs text-neutral-400">⌘S</span>
          </button>
          <button
            onClick={() => void play()}
            title="Run (⌘↵)"
            className="inline-flex items-center gap-2 rounded-md bg-emerald-600 px-3 py-1.5 text-sm font-medium text-white transition-colors hover:bg-emerald-500 active:bg-emerald-700"
          >
            <svg viewBox="0 0 24 24" fill="currentColor" className="h-4 w-4">
              <path d="M8 5v14l11-7z" />
            </svg>
            Play
            <span className="text-xs text-emerald-200/80">⌘↵</span>
          </button>
          <button
            onClick={() => void stop()}
            title="Stop (⌘.)"
            className="inline-flex items-center gap-2 rounded-md bg-red-700 px-3 py-1.5 text-sm font-medium text-white transition-colors hover:bg-red-600 active:bg-red-800"
          >
            <svg viewBox="0 0 24 24" fill="currentColor" className="h-4 w-4">
              <path d="M6 6h12v12H6z" />
            </svg>
            Stop
            <span className="text-xs text-red-200/80">⌘.</span>
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
          title="New tab"
          className="flex items-center px-3 text-neutral-400 transition-colors hover:bg-neutral-900 hover:text-neutral-100"
        >
          <svg viewBox="0 0 24 24" fill="currentColor" className="h-4 w-4">
            <path d="M11 11V5h2v6h6v2h-6v6h-2v-6H5v-2z" />
          </svg>
        </button>
      </div>

      <main className="min-h-0 flex-1">
        <CodeMirror
          key={activeTab.id}
          value={activeTab.content}
          onChange={(value) => updateContent(activeTab.id, value)}
          height="100%"
          theme="dark"
          extensions={[javascript()]}
          className="h-full text-sm"
        />
      </main>
    </div>
  );
}

export default App;

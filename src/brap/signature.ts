import { StateField, type EditorState, type Extension } from "@codemirror/state";
import { EditorView, showTooltip, type Tooltip } from "@codemirror/view";
import { isCallable, requiredArgs, type Builtin, type BuiltinIndex } from "./metadata";

/**
 * Parameter hints: while the cursor sits inside a call, show that call's
 * signature with the argument being typed picked out.
 *
 * CodeMirror ships completion and linting but no signature help, so this is a
 * small hand-rolled one. It works on raw text rather than the syntax tree
 * because a half-typed `lowpass(` has no tree node to hang off.
 */

interface CallSite {
  name: string;
  /** Zero-based index of the argument the cursor is in. */
  argIndex: number;
}

/** Blank out `//` comments, preserving offsets so positions stay valid. */
function stripComments(text: string): string {
  return text.replace(/\/\/[^\n]*/g, (m) => " ".repeat(m.length));
}

/**
 * How far back to look for the opening parenthesis. Runs on every cursor move,
 * so it reads a window rather than the whole document; no real call site is
 * anywhere near this long.
 */
const WINDOW = 4000;

/** Text before the cursor, comment-stripped, with the cursor's offset in it.
 *  Starts at a line boundary so a truncated `//` cannot be misread. */
function lookback(state: EditorState, pos: number): { text: string; pos: number } {
  const from = pos > WINDOW ? state.doc.lineAt(pos - WINDOW).from : 0;
  return { text: stripComments(state.doc.sliceString(from, pos)), pos: pos - from };
}

/**
 * Walk backwards from `pos` to the innermost unclosed `(`, counting the commas
 * that separate it from the cursor.
 *
 * Returns null when the cursor is not inside a call — including when it is
 * inside a list literal, since a `[` reached at depth zero means the nearest
 * enclosing bracket is not a call's parenthesis.
 */
function callAt(text: string, pos: number): CallSite | null {
  let depth = 0;
  let commas = 0;

  for (let i = pos - 1; i >= 0; i--) {
    const c = text[i];

    if (c === ")" || c === "]") {
      depth++;
    } else if (c === "[") {
      // At depth zero the nearest enclosing bracket is a list literal, not a
      // call — `[sin(1), 2]` puts the cursor in a list, not in `sin`.
      if (depth === 0) return null;
      depth--;
    } else if (c === "(") {
      if (depth > 0) {
        depth--;
        continue;
      }

      // Read the identifier immediately before the paren.
      let j = i - 1;
      while (j >= 0 && /\s/.test(text[j])) j--;
      const end = j + 1;
      while (j >= 0 && /[a-zA-Z0-9_]/.test(text[j])) j--;
      const name = text.slice(j + 1, end);
      if (!name || /^[0-9]/.test(name)) return null;

      // `lhs >> f(a)` passes lhs as f's first argument, so what the user is
      // typing after the paren is really the second parameter.
      let k = j;
      while (k >= 0 && /\s/.test(text[k])) k--;
      const piped = k >= 1 && text[k - 1] === ">" && text[k] === ">";

      return { name, argIndex: commas + (piped ? 1 : 0) };
    } else if (c === "," && depth === 0) {
      commas++;
    }
  }

  return null;
}

/** `lowpass(audio, `**`cutoff`**`, q)` as DOM, with the active param marked. */
function render(b: Builtin, argIndex: number): HTMLElement {
  const dom = document.createElement("div");
  dom.className = "cm-brap-signature";

  const name = dom.appendChild(document.createElement("span"));
  name.className = "cm-brap-signature-name";
  name.textContent = b.name;

  dom.appendChild(document.createTextNode("("));

  const required = requiredArgs(b);
  b.params.forEach((param, i) => {
    if (i > 0) dom.appendChild(document.createTextNode(", "));
    const span = dom.appendChild(document.createElement("span"));
    span.textContent = i < required ? param : `${param}?`;
    if (i === argIndex) span.className = "cm-brap-signature-active";
  });

  if (b.variadic) {
    dom.appendChild(document.createTextNode(b.params.length ? ", ..." : "..."));
  }

  dom.appendChild(document.createTextNode(")"));

  if (b.doc) {
    const doc = dom.appendChild(document.createElement("div"));
    doc.className = "cm-brap-signature-doc";
    doc.textContent = b.doc;
  }

  return dom;
}

function tooltips(state: EditorState, index: BuiltinIndex): readonly Tooltip[] {
  const { main } = state.selection;
  if (!main.empty) return [];

  const back = lookback(state, main.head);
  const call = callAt(back.text, back.pos);
  if (!call) return [];

  const builtin = index.get(call.name);
  if (!builtin || !isCallable(builtin)) return [];

  // Past the last parameter there is nothing left to point at — the call is
  // over-applied, which the lowerer will report on its own.
  if (call.argIndex >= builtin.params.length && !builtin.variadic) return [];

  return [
    {
      pos: main.head,
      above: true,
      create: () => ({ dom: render(builtin, call.argIndex) }),
    },
  ];
}

const signatureTheme = EditorView.baseTheme({
  ".cm-brap-signature": {
    padding: "4px 8px",
    fontFamily: "monospace",
    fontSize: "12px",
    maxWidth: "42em",
    borderRadius: "4px",
  },
  ".cm-brap-signature-name": { fontWeight: "bold" },
  ".cm-brap-signature-active": {
    fontWeight: "bold",
    textDecoration: "underline",
  },
  ".cm-brap-signature-doc": {
    marginTop: "4px",
    fontFamily: "sans-serif",
    opacity: "0.75",
    whiteSpace: "normal",
  },
});

/** Exposed for tests; not part of the extension's public surface. */
export const __test = { callAt, stripComments };

export function signatureHelp(index: BuiltinIndex): Extension {
  const field = StateField.define<readonly Tooltip[]>({
    create: (state) => tooltips(state, index),
    update(value, tr) {
      if (!tr.docChanged && !tr.selection) return value;
      return tooltips(tr.state, index);
    },
    provide: (f) => showTooltip.computeN([f], (state) => state.field(f)),
  });

  return [field, signatureTheme];
}

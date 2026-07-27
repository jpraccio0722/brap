import type {
  Completion,
  CompletionContext,
  CompletionResult,
  CompletionSource,
} from "@codemirror/autocomplete";
import type { EditorView } from "@codemirror/view";
import {
  isCallable,
  requiredArgs,
  signature,
  type Builtin,
  type LanguageMetadata,
} from "./metadata";

/**
 * Identifier completion for scree: backend builtins, keywords, and whatever the
 * current buffer defines.
 *
 * Document symbols are scraped with regexes rather than parsed. The text being
 * completed is, by definition, half-written — the real parser would reject it,
 * and a round trip to Rust per keystroke would be worse than imprecise.
 */

const FN = /\bfn\s+([a-zA-Z_]\w*)\s*\(([^)]*)\)/g;
const LET = /\blet\s+([a-zA-Z_]\w*)/g;
const FOR = /\bfor\s+([a-zA-Z_]\w*)\s+in\b/g;
/** Strips `//` comments so definitions inside them are not offered. */
const COMMENT = /\/\/[^\n]*/g;

/** Local symbols rank above builtins; there are fewer and they are yours. The
 *  drawn patterns rank highest: there are fewer still, and the panel is the
 *  only place their names are written down. */
const BOOST = { pattern: 3, local: 2, builtin: 1, keyword: 0 } as const;

interface LocalSymbol {
  name: string;
  detail: string;
  type: "function" | "variable";
}

function scrapeLocals(doc: string): LocalSymbol[] {
  const text = doc.replace(COMMENT, "");
  const found = new Map<string, LocalSymbol>();

  for (const [, name, rawParams] of text.matchAll(FN)) {
    const params = rawParams
      .split(",")
      .map((p) => p.split("=")[0].trim())
      .filter(Boolean);
    found.set(name, {
      name,
      detail: `${name}(${params.join(", ")})`,
      type: "function",
    });

    // A function's own parameters are worth offering while writing its body.
    // Scoping them properly would need the real parser; over-offering a name
    // from a sibling function is a smaller cost than missing the one you want.
    for (const p of params) {
      if (!found.has(p)) found.set(p, { name: p, detail: "parameter", type: "variable" });
    }
  }

  for (const re of [LET, FOR]) {
    for (const [, name] of text.matchAll(re)) {
      if (!found.has(name)) {
        found.set(name, { name, detail: re === FOR ? "loop variable" : "binding", type: "variable" });
      }
    }
  }

  return [...found.values()];
}

/**
 * Insert `name()` and put the cursor where the next thing typed should go:
 * between the parens when there are arguments to write, after them when there
 * are not.
 */
function applyCall(name: string, cursorInside: boolean) {
  return (view: EditorView, _completion: Completion, from: number, to: number) => {
    const insert = `${name}()`;
    view.dispatch({
      changes: { from, to, insert },
      selection: { anchor: from + insert.length - (cursorInside ? 1 : 0) },
    });
  };
}

function builtinCompletion(b: Builtin): Completion {
  const option: Completion = {
    label: b.name,
    detail: signature(b),
    info: b.doc,
    type: isCallable(b) ? "function" : "variable",
    boost: BOOST.builtin,
  };
  if (isCallable(b)) {
    option.apply = applyCall(b.name, requiredArgs(b) > 0 || b.variadic);
  }
  return option;
}

/** Exposed for tests; not part of the extension's public surface. */
export const __test = { scrapeLocals };

/**
 * @param patternNames The drawn patterns' names, read at completion time
 * rather than captured. The panel changes them constantly, and rebuilding this
 * source per edit would mean reconfiguring the editor per keystroke.
 */
export function screeCompletions(
  meta: LanguageMetadata,
  patternNames: () => string[],
): CompletionSource {
  const builtins = meta.builtins.map(builtinCompletion);
  const keywords: Completion[] = meta.keywords.map((label) => ({
    label,
    type: "keyword",
    boost: BOOST.keyword,
  }));

  return (context: CompletionContext): CompletionResult | null => {
    const word = context.matchBefore(/[a-zA-Z_]\w*/);
    // An explicit invocation on empty space should still list everything.
    if (!word && !context.explicit) return null;
    if (word?.from === word?.to && !context.explicit) return null;

    const patterns: Completion[] = patternNames().map((name) => ({
      label: name,
      detail: "graphical pattern",
      info: "Drawn in the side panel. Bound as a list, so it plays like one: play(name, kick)",
      type: "variable",
      boost: BOOST.pattern,
    }));

    // `call_with` tries the builtin tables before the environment, so a local
    // `fn saw` never runs — the UGen does. Drop the shadowed local rather than
    // offering a name whose completion would describe the wrong thing.
    const taken = new Set([
      ...builtins.map((b) => b.label),
      ...keywords.map((k) => k.label),
      // A `let` of a pattern's name shadows the panel, but it is still one
      // name, and the drawn one already says where to go and change it.
      ...patterns.map((p) => p.label),
    ]);
    const locals = scrapeLocals(context.state.doc.toString())
      .filter((s) => !taken.has(s.name))
      .map(
        (s): Completion => ({
          label: s.name,
          detail: s.detail,
          type: s.type,
          boost: BOOST.local,
          apply: s.type === "function" ? applyCall(s.name, true) : undefined,
        }),
      );

    const options = [...patterns, ...locals, ...keywords, ...builtins];

    return {
      from: word?.from ?? context.pos,
      options,
      validFor: /^\w*$/,
    };
  };
}

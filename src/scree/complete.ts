import type {
  Completion,
  CompletionContext,
  CompletionResult,
  CompletionSource,
} from "@codemirror/autocomplete";
import type { EditorView } from "@codemirror/view";
import {
  buildIndex,
  isCallable,
  requiredArgs,
  signature,
  type Builtin,
  type BuiltinIndex,
  type LanguageMetadata,
  type ValueKind,
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
/** A `let`, with as much of its value as fits on the line — enough to guess
 *  whether the name holds a list. */
const LET = /\blet\s+([a-zA-Z_]\w*)\s*(?:=\s*([^\n]*))?/g;
const FOR = /\bfor\s+([a-zA-Z_]\w*)\s+in\b/g;
/**
 * A `use`, split into its path and whatever follows it.
 *
 * What a `use` introduces is readable from the line in every case but the
 * glob, whose names live in a file this side has never read — those appear
 * once the program is run and the name resolves, which is the same moment a
 * mistyped one is caught.
 */
const USE =
  /\buse\s+([a-zA-Z_]\w*(?:::[a-zA-Z_]\w*)*)\s*(?:(::\*)|as\s+([a-zA-Z_]\w*)|::\{([^}]*)\})?/g;
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
  /** A `fn`'s parameter names. Absent for anything that is not one. */
  params?: string[];
  /** True for a name a `use` introduced. It may be a function or a module, and
   *  both are writable after a `.`, so the two cases are not worth splitting. */
  imported?: boolean;
  /** A `let`'s value, as written. Resolved on demand to work out what a dot on
   *  this name may reach — `let riff = [60, 63]` makes `riff.` a list. */
  value?: string;
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
      params,
    });

    // A function's own parameters are worth offering while writing its body.
    // Scoping them properly would need the real parser; over-offering a name
    // from a sibling function is a smaller cost than missing the one you want.
    for (const p of params) {
      if (!found.has(p)) found.set(p, { name: p, detail: "parameter", type: "variable" });
    }
  }

  for (const [, name, value] of text.matchAll(LET)) {
    if (!found.has(name)) {
      found.set(name, { name, detail: "binding", type: "variable", value });
    }
  }

  // A loop variable holds one element, so it is not the list being walked.
  for (const [, name] of text.matchAll(FOR)) {
    if (!found.has(name)) {
      found.set(name, { name, detail: "loop variable", type: "variable" });
    }
  }

  for (const name of importedNames(text)) {
    // A module's name completes as a variable: what follows it is `::`, not
    // `(`, so inserting a call would be inserting the wrong thing.
    if (!found.has(name)) {
      found.set(name, { name, detail: "imported", type: "variable", imported: true });
    }
  }

  return [...found.values()];
}

/** Every name the file's `use` lines make writable, in the spelling it is
 *  written in here — which is the alias, wherever there is one. */
function importedNames(text: string): string[] {
  const names: string[] = [];

  for (const [, path, glob, alias, list] of text.matchAll(USE)) {
    if (list !== undefined) {
      for (const entry of list.split(",")) {
        // `kick` or `kick as thump`; the last word is what it is called here.
        const words = entry.trim().split(/\s+as\s+/);
        const name = words[words.length - 1].trim();
        if (/^[a-zA-Z_]\w*$/.test(name)) names.push(name);
      }
      continue;
    }
    if (glob !== undefined) continue;
    // `use a::b` puts `b` in scope, whether it turns out to be a module or a
    // single name; `as` renames it.
    names.push(alias ?? path.split("::").pop()!);
  }

  return names;
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

// ---------------------------------------------------------------------------
// Method position.
//
// `a.f(b)` is the parser's spelling of `a >> f(b)`, which is the spelling of
// `f(a, b)` — the receiver fills the first parameter. So what may follow a `.`
// is every function of at least one argument, and what should be offered is
// that function minus the parameter already filled: `push(list, value)` reads
// `.push(value)` here, and `rev(list)` needs no parentheses at all.
// ---------------------------------------------------------------------------

/**
 * After a `.` the file's usual local-first order is inverted for the functions
 * that exactly suit the receiver. A list in front of the dot means `push` and
 * `rev` are what is being reached for, whereas a scree file's own `fn`s are
 * instruments — played with `play(pattern, kick)` rather than called on a list.
 */
const METHOD_BOOST = { suited: 3, local: 2, other: 1 } as const;

/** `c4`, `af3`, `gs9` — a note name, which is a number everywhere downstream. */
const NOTE = /^[a-g][sf]?\d$/;

/**
 * Whether a receiver of kind `receiver` may be written to the left of a name
 * declaring `receives`.
 *
 * This mirrors `accepts` in `lang.rs` exactly. That function is the one under
 * test — `every_builtin_receives_what_it_declares` compiles all 103 names
 * against every kind of receiver — so any disagreement here is this file's bug.
 */
function accepts(receives: ValueKind, receiver: ValueKind): boolean {
  // A receiver the tables cannot pin down rules nothing out.
  if (receiver === "any") return true;
  switch (receives) {
    case "signal":
      // A constant is a node input like any other, so a number reads here.
      return receiver === "signal" || receiver === "number";
    case "number":
      return receiver === "number";
    case "list":
      return receiver === "list";
    case "pattern":
      // A bare value is a one-step pattern: `60.play(inst)` sounds once.
      return receiver === "list" || receiver === "number";
    case "play":
      return receiver === "play";
    default:
      // "nothing" takes no argument at all; "any" is only ever a result.
      return false;
  }
}

/** The context an expression's kind is resolved against. */
interface Scope {
  index: BuiltinIndex;
  locals: Map<string, LocalSymbol>;
  patterns: Set<string>;
}

/** The index of the bracket matching the one `text` ends with, or null. */
function matchingOpen(text: string, open: string, close: string): number | null {
  let depth = 0;
  for (let i = text.length - 1; i >= 0; i--) {
    if (text[i] === close) depth++;
    else if (text[i] === open && --depth === 0) return i;
  }
  return null;
}

/**
 * What kind of value an expression produces, read from its last few characters.
 *
 * Like the rest of this file this reads text rather than a tree: the expression
 * is half-written by definition, and the real parser would reject it. So it
 * recognises the shapes that carry their kind on their surface and answers
 * `"any"` — which rules nothing out — to everything else.
 *
 * `depth` bounds the walk through `let` bindings, so a file where two names
 * refer to each other cannot spin.
 */
function kindOf(expr: string, scope: Scope, depth = 4): ValueKind {
  const text = expr.trimEnd();
  if (depth <= 0 || text === "") return "any";

  // `rev([1, 2])` — a call, whose kind is whatever the name answers with.
  if (text.endsWith(")")) {
    const open = matchingOpen(text, "(", ")");
    if (open === null) return "any";
    const name = text.slice(0, open).trimEnd().match(/[a-zA-Z_]\w*$/)?.[0];
    return name === undefined ? "any" : (scope.index.get(name)?.returns ?? "any");
  }

  // `[1, 2, 3]` is a list; `xs[0]` is one element of one, of no known kind.
  // They differ only in whether something indexable sits before the bracket.
  if (text.endsWith("]")) {
    const open = matchingOpen(text, "[", "]");
    if (open === null) return "any";
    return /[a-zA-Z0-9_)\]]$/.test(text.slice(0, open)) ? "any" : "list";
  }

  // `0..=7` is a list of steps, like the literal it stands in for. A range
  // inside brackets or a call was already consumed above, so a `..=` still
  // visible here spans the whole expression.
  if (text.includes("..=")) return "list";

  const word = text.match(/[a-zA-Z_]\w*$/)?.[0];
  if (word === undefined) {
    // `60.m2h`, and `0.5.db`.
    return /\d$/.test(text) ? "number" : "any";
  }

  // A name written after a dot is a call with its arguments omitted, so it is
  // that name's result: in `riff.rev.push(72)`, `rev` is what `push` receives.
  const before = text.slice(0, text.length - word.length);
  if (before.endsWith(".") && !before.endsWith("..")) {
    return scope.index.get(word)?.returns ?? "any";
  }

  // A bare builtin name is that name too — `dur` is the one that matters.
  const builtin = scope.index.get(word);
  if (builtin && !scope.locals.has(word)) return builtin.returns;

  if (scope.patterns.has(word)) return "list";

  const local = scope.locals.get(word);
  if (local?.value !== undefined) return kindOf(local.value, scope, depth - 1);
  // A `for` variable holds one element, and a parameter could be anything.
  if (local !== undefined) return "any";

  if (NOTE.test(word)) return "number";
  return "any";
}

/**
 * The offset of the `.` a name is being written after, or null when the cursor
 * is not in method position.
 *
 * A range's `..=` is the one other place two of these characters meet, and its
 * second dot is followed by `=` rather than by a name — but a range is written
 * one character at a time, so `1..` is a state the completer really sees.
 */
function methodDot(doc: string, from: number): number | null {
  const dot = from - 1;
  if (doc[dot] !== "." || doc[dot - 1] === ".") return null;
  return dot;
}

/**
 * Insert a method: bare when the receiver fills every parameter that must be
 * filled — `xs.rev`, `60.m2h` — and `name()` with the cursor between the
 * parentheses when something is still owed.
 */
function applyMethod(name: string, takesArgs: boolean) {
  return (view: EditorView, _completion: Completion, from: number, to: number) => {
    const insert = takesArgs ? `${name}()` : name;
    view.dispatch({
      changes: { from, to, insert },
      selection: { anchor: from + insert.length - (takesArgs ? 1 : 0) },
    });
  };
}

/** `push(list, value)` as it reads after a dot: `.push(value)`. */
function methodCompletion(b: Builtin, boost: number): Completion {
  // One fewer of each: the receiver has filled the first parameter.
  const rest = b.params.slice(1);
  const required = Math.max(requiredArgs(b) - 1, 0);
  const parts = rest.map((p, i) => (i < required ? p : `${p}?`));
  if (b.variadic) parts.push("...");

  return {
    label: b.name,
    detail: parts.length ? `.${b.name}(${parts.join(", ")})` : `.${b.name}`,
    info: b.doc,
    type: "method",
    boost,
    apply: applyMethod(b.name, required > 0 || b.variadic),
  };
}

/** The same, for a `fn` in the buffer or a name a `use` brought in. */
function localMethodCompletion(s: LocalSymbol): Completion {
  // An imported name may be a module, whose `::` the user writes themselves.
  if (s.params === undefined) {
    return { label: s.name, detail: s.detail, type: "variable", boost: METHOD_BOOST.local };
  }
  const rest = s.params.slice(1);
  return {
    label: s.name,
    detail: rest.length ? `.${s.name}(${rest.join(", ")})` : `.${s.name}`,
    type: "method",
    boost: METHOD_BOOST.local,
    apply: applyMethod(s.name, rest.length > 0),
  };
}

/** Exposed for tests; not part of the extension's public surface. */
export const __test = { scrapeLocals, methodDot, kindOf, accepts };

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

  /** Callable at all, and with a first parameter for a receiver to fill. */
  const methods = meta.builtins.filter((b) => isCallable(b) && b.params.length > 0);
  const index = buildIndex(meta);

  return (context: CompletionContext): CompletionResult | null => {
    const doc = context.state.doc.toString();
    const word = context.matchBefore(/[a-zA-Z_]\w*/);
    const dot = methodDot(doc, word?.from ?? context.pos);

    // A `.` is worth completing on its own: it is the one character after
    // which the set of writable names is both small and hard to remember.
    if (dot === null) {
      // An explicit invocation on empty space should still list everything.
      if (!word && !context.explicit) return null;
      if (word?.from === word?.to && !context.explicit) return null;
    }

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
    const scraped = scrapeLocals(doc);
    const visible = scraped.filter((s) => !taken.has(s.name));

    const options =
      dot === null
        ? [
            ...patterns,
            ...visible.map(
              (s): Completion => ({
                label: s.name,
                detail: s.detail,
                type: s.type,
                boost: BOOST.local,
                apply: s.type === "function" ? applyCall(s.name, true) : undefined,
              }),
            ),
            ...keywords,
            ...builtins,
          ]
        : methodOptions(dot);

    return {
      from: word?.from ?? context.pos,
      options,
      validFor: /^\w*$/,
    };

    /**
     * What may be written after the `.` at `dot`.
     *
     * Keywords, patterns and plain bindings are all absent: none of them is a
     * function, so none can take the receiver. Of the functions, only those
     * whose first parameter accepts what is in front of the dot survive — on a
     * list that drops the 58 UGens and the arithmetic, every one of which would
     * be a compile error rather than merely an unlikely thing to write.
     *
     * A receiver this cannot identify reads as `"any"`, which accepts
     * everything: an unrecognised expression must not hide working names.
     */
    function methodOptions(dot: number): Completion[] {
      const scope: Scope = {
        index,
        locals: new Map(scraped.map((s) => [s.name, s])),
        patterns: new Set(patternNames()),
      };
      const receiver = kindOf(doc.slice(0, dot), scope);

      const fromBuiltins = methods
        .filter((b) => accepts(b.receives, receiver))
        .map((b) =>
          methodCompletion(
            b,
            // An exact match ranks above one that merely accepts: a list takes
            // both `push` and `play`, and `push` is the likelier next word.
            b.receives === receiver ? METHOD_BOOST.suited : METHOD_BOOST.other,
          ),
        );
      // A user `fn` has no declared parameter kinds, so it is always offered.
      const fromLocals = visible
        .filter((s) => (s.params?.length ?? 0) > 0 || s.imported)
        .map(localMethodCompletion);

      return [...fromLocals, ...fromBuiltins];
    }
  };
}

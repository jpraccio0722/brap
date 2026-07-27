import {
  HighlightStyle,
  LanguageSupport,
  StreamLanguage,
  type StreamParser,
  syntaxHighlighting,
} from "@codemirror/language";
import type { CompletionSource } from "@codemirror/autocomplete";
import { Prec } from "@codemirror/state";
import { Tag, tags as t } from "@lezer/highlight";
import type { BuiltinIndex, LanguageMetadata } from "./metadata";

/**
 * A brap tokenizer for CodeMirror, mirroring `src-tauri/src/parser/lex.rs`.
 *
 * A stream tokenizer rather than a Lezer grammar: brap's lexical structure is
 * flat enough that a generated parser would buy nothing but a build step.
 *
 * The one rule worth stating explicitly — **brap has no string literal**. The
 * JavaScript mode this replaces treated the backtick rest token as the start of
 * a template literal, which swallowed the remainder of the file.
 */

/** The rest token: `` ` `` inside a pattern. Its own tag, because it is the
 *  most brap-specific thing on screen and deserves to stand out. */
export const restTag = Tag.define();
/** A name resolved from the backend's builtin table. */
export const builtinTag = Tag.define();

/** Matches `lex.rs`'s number regex, anchored for `StringStream.match`. */
const NUMBER = /^(\d+(\.\d+)?|\.\d+)([eE][+-]?\d+)?/;
const IDENT = /^[a-zA-Z_][a-zA-Z0-9_]*/;
/**
 * Used until the backend's keyword list arrives, so a file is highlighted
 * correctly on the very first frame rather than rendering every keyword as a
 * plain variable. The backend list wins once loaded.
 */
const FALLBACK_KEYWORDS = ["fn", "let", "if", "else", "for", "in"];
/** Longest first, so `..=` beats `.`, `>>` and `>=` beat `>`, `==` beats `=`. */
const OPERATORS = [
  "..=",
  ">>",
  "==",
  "!=",
  "<=",
  ">=",
  "+",
  "-",
  "*",
  "/",
  "%",
  "=",
  "<",
  ">",
];

interface State {
  /** The previous token was `fn`, so the next identifier is a definition. */
  afterFn: boolean;
}

function parser(meta: LanguageMetadata, index: BuiltinIndex): StreamParser<State> {
  const keywords = new Set(meta.keywords.length ? meta.keywords : FALLBACK_KEYWORDS);

  return {
    name: "brap",

    startState: () => ({ afterFn: false }),

    token(stream, state) {
      if (stream.eatSpace()) return null;

      // Comments run to end of line; there is no block comment form.
      if (stream.match("//")) {
        stream.skipToEnd();
        return "comment";
      }

      // Before operators: a number may begin with `.` (`.5`).
      if (stream.match(NUMBER)) {
        state.afterFn = false;
        return "number";
      }

      if (stream.match("`")) {
        state.afterFn = false;
        return "brapRest";
      }

      // `match` is typed as `RegExpMatchArray | true | null`; a regex argument
      // always yields the array form.
      const ident = stream.match(IDENT) as RegExpMatchArray | null;
      if (ident) {
        const name = ident[0];

        if (state.afterFn) {
          state.afterFn = false;
          return "def";
        }
        if (keywords.has(name)) {
          state.afterFn = name === "fn";
          return "keyword";
        }
        if (index.has(name)) return "brapBuiltin";
        // A name in call position is a user function; anything else is a value.
        return stream.match(/^\s*\(/, false) ? "fnName" : "variable";
      }

      state.afterFn = false;

      if (stream.match(/^[[\]{}()]/)) return "bracket";
      if (stream.match(/^[,;:]/)) return "punctuation";

      for (const op of OPERATORS) {
        if (stream.match(op)) return "operator";
      }

      // Unknown character: consume it so the tokenizer always advances.
      stream.next();
      return null;
    },

    languageData: {
      commentTokens: { line: "//" },
    },

    // Only names that are NOT already resolvable are listed here.
    //
    // StreamLanguage resolves a token name against a built-in table of tag
    // names and CodeMirror 5 legacy aliases *before* consulting this one, and
    // the built-in entry wins. `builtin` and `rest` are why the custom names
    // above are prefixed: plain `builtin` is a legacy alias for
    // `variableName.standard`, so an entry for it here is silently ignored.
    // Everything the tokenizer returns unprefixed — `comment`, `number`,
    // `keyword`, `operator`, `bracket`, `punctuation`, `variable`, `def` —
    // resolves through that built-in table on purpose.
    tokenTable: {
      brapBuiltin: builtinTag,
      brapRest: restTag,
      fnName: t.function(t.variableName),
    },
  };
}

/**
 * Colours for the dark editor theme. Deliberately gives `rest` a loud, warm
 * colour: it is a single character that changes what a pattern plays.
 */
export const brapHighlightStyle = HighlightStyle.define([
  // `t.comment`, not `t.lineComment`: the tokenizer's `comment` token resolves
  // to the base tag, and fallback runs specific -> general, never the reverse.
  { tag: t.comment, color: "#6b7280", fontStyle: "italic" },
  { tag: t.number, color: "#f0abfc" },
  { tag: t.keyword, color: "#c084fc" },
  { tag: builtinTag, color: "#5eead4" },
  { tag: t.function(t.variableName), color: "#93c5fd" },
  { tag: t.definition(t.variableName), color: "#fcd34d" },
  { tag: t.variableName, color: "#e5e7eb" },
  { tag: restTag, color: "#fb923c", fontWeight: "bold" },
  { tag: t.operator, color: "#94a3b8" },
  { tag: t.bracket, color: "#94a3b8" },
  { tag: t.punctuation, color: "#94a3b8" },
]);

/**
 * The brap language, with completion attached as language data.
 *
 * Attached here rather than through a second `autocompletion()` extension
 * because the editor's `basicSetup` already installs one; adding another would
 * mount two completion tooltips.
 */
export function brapLanguage(
  meta: LanguageMetadata,
  index: BuiltinIndex,
  autocomplete: CompletionSource,
): LanguageSupport {
  const language = StreamLanguage.define(parser(meta, index));
  return new LanguageSupport(language, [
    language.data.of({ autocomplete }),
    // The editor's `theme="dark"` ships its own highlight style, and the first
    // highlighter with a rule for a tag wins. Without raising precedence only
    // the tags that theme does not define (`rest`, `builtin`) would be ours,
    // leaving the palette half applied.
    Prec.high(syntaxHighlighting(brapHighlightStyle)),
  ]);
}

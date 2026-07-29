import type { Extension } from "@codemirror/state";
import { screeCompletions } from "./complete";
import { errorMarks } from "./errors";
import { builtinHelp } from "./help";
import { screeLanguage } from "./language";
import { buildIndex, type LanguageMetadata } from "./metadata";
import { signatureHelp } from "./signature";

export { EMPTY_METADATA, loadMetadata, type LanguageMetadata } from "./metadata";
export { revealPosition, showErrorLines } from "./errors";

/**
 * Every scree editor extension, built from one metadata snapshot.
 *
 * Call this once per load and memoize the result: CodeMirror reconfigures when
 * the extension array's identity changes, and rebuilding it per render would
 * throw away the completion state on every keystroke.
 *
 * Safe to call with `EMPTY_METADATA` before the backend answers — highlighting
 * works immediately, and only the builtin colouring and completions are absent.
 *
 * @param patternNames The names of the patterns drawn in the side panel, as a
 * getter rather than a value: they change with every edit to the panel, and
 * this array's identity must not.
 * @param openDocs Where a ⌘-clicked builtin goes. Bound by the same rule as
 * `patternNames`: its identity has to hold across renders.
 */
export function screeExtensions(
  meta: LanguageMetadata,
  patternNames: () => string[],
  openDocs: (name: string) => void,
): Extension[] {
  const index = buildIndex(meta);
  return [
    screeLanguage(meta, index, screeCompletions(meta, patternNames)),
    signatureHelp(index),
    builtinHelp(index, openDocs),
    // Holds nothing until a run fails; the marks arrive by transaction, which
    // is what keeps this array's identity out of it.
    errorMarks(),
  ];
}

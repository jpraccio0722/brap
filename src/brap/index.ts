import type { Extension } from "@codemirror/state";
import { brapCompletions } from "./complete";
import { brapLanguage } from "./language";
import { buildIndex, type LanguageMetadata } from "./metadata";
import { signatureHelp } from "./signature";

export { EMPTY_METADATA, loadMetadata, type LanguageMetadata } from "./metadata";

/**
 * Every brap editor extension, built from one metadata snapshot.
 *
 * Call this once per load and memoize the result: CodeMirror reconfigures when
 * the extension array's identity changes, and rebuilding it per render would
 * throw away the completion state on every keystroke.
 *
 * Safe to call with `EMPTY_METADATA` before the backend answers — highlighting
 * works immediately, and only the builtin colouring and completions are absent.
 */
export function brapExtensions(meta: LanguageMetadata): Extension[] {
  const index = buildIndex(meta);
  return [
    brapLanguage(meta, index, brapCompletions(meta)),
    signatureHelp(index),
  ];
}

import { invoke } from "@tauri-apps/api/core";

/**
 * The language's callable surface, served by the `language_metadata` Tauri
 * command from the same tables the lowerer dispatches on. Nothing here is
 * hand-maintained on this side: adding a UGen in Rust adds it to the editor.
 */

export type BuiltinCategory = "ugen" | "list" | "special";

export interface Builtin {
  name: string;
  /** Parameter names, in order. Long enough to cover the largest arity. */
  params: string[];
  /** Every accepted argument count. Empty means the name is not callable. */
  arities: number[];
  /** True when any count above the largest arity is also accepted. */
  variadic: boolean;
  category: BuiltinCategory;
  doc: string;
}

export interface LanguageMetadata {
  builtins: Builtin[];
  keywords: string[];
}

/** Used until the backend answers, so highlighting is correct from frame one. */
export const EMPTY_METADATA: LanguageMetadata = { builtins: [], keywords: [] };

let pending: Promise<LanguageMetadata> | null = null;

/** Fetch the metadata once per session. */
export function loadMetadata(): Promise<LanguageMetadata> {
  pending ??= invoke<LanguageMetadata>("language_metadata");
  return pending;
}

/** Name-keyed lookup, built once per metadata load. */
export type BuiltinIndex = Map<string, Builtin>;

export function buildIndex(meta: LanguageMetadata): BuiltinIndex {
  return new Map(meta.builtins.map((b) => [b.name, b]));
}

/** The smallest number of arguments a call can be made with. */
export function requiredArgs(b: Builtin): number {
  return b.arities.length === 0 ? 0 : Math.min(...b.arities);
}

export function isCallable(b: Builtin): boolean {
  return b.arities.length > 0;
}

/**
 * The display signature: `lowpass(audio, cutoff, q)`.
 *
 * Parameters past the smallest arity are optional and marked `?`; a variadic
 * builtin gets a trailing `...`. `dur` takes no arguments at all and is
 * rendered bare, since it is a binding rather than a function.
 */
export function signature(b: Builtin): string {
  if (!isCallable(b)) return b.name;

  const required = requiredArgs(b);
  const parts = b.params.map((p, i) => (i < required ? p : `${p}?`));
  if (b.variadic) parts.push("...");
  return `${b.name}(${parts.join(", ")})`;
}

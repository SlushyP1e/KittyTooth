import type { Backend } from "./backend";
import { createMockBackend } from "./mock";
import { createTauriBackend } from "./tauri";

export type { Backend } from "./backend";
export { toUserError } from "./backend";

/** True inside the Tauri webview. */
export function inTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

let instance: Backend | null = null;

/** The Tauri backend in the app; the simulated backend in a plain browser (dev, tests, screenshots). */
export function backend(): Backend {
  if (!instance) instance = inTauri() ? createTauriBackend() : createMockBackend();
  return instance;
}

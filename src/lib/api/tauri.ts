import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { Backend, Unlisten } from "./backend";
import { toUserError } from "./backend";
import type {
  BackendEvents,
  Config,
  ConfigResult,
  HistoryPoint,
  InitialState,
  OscLogEntry,
  SecretStatus,
} from "../types";

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    throw toUserError(e);
  }
}

export function createTauriBackend(): Backend {
  return {
    isMock: false,
    getState: () => call<InitialState>("get_state"),
    setConfig: (config: Config) => call<ConfigResult>("set_config", { config }),
    startScan: () => call("start_scan"),
    stopScan: () => call("stop_scan"),
    connect: (id, name) => call("connect", { id, name }),
    disconnect: () => call("disconnect"),
    forget: () => call("forget"),
    selectSource: (kind) => call("select_source", { kind }),
    setSecret: (kind, value) => call<SecretStatus[]>("set_secret", { kind, value }),
    clearSecret: (kind) => call<SecretStatus[]>("clear_secret", { kind }),
    openLink: (link) => call("open_link", { link }),
    testSweep: (on) => call("test_sweep", { on }),
    testAlert: () => call("test_alert"),
    resetSession: () => call("reset_session"),
    getHistory: (sinceMs) => call<HistoryPoint[]>("get_history", { sinceMs }),
    getOscLog: () => call<OscLogEntry[]>("get_osc_log"),
    exportConfig: () => call<string | null>("export_config"),
    importConfig: () => call<ConfigResult | null>("import_config"),
    resetConfig: () => call<ConfigResult>("reset_config"),
    copyDiagnostics: () => call<string>("copy_diagnostics"),
    openMini: () => call("open_mini"),
    closeMini: () => call("close_mini"),
    showMain: () => call("show_main"),
    startDrag: () => getCurrentWindow().startDragging(),
    async on<K extends keyof BackendEvents>(event: K, cb: (p: BackendEvents[K]) => void): Promise<Unlisten> {
      return listen<BackendEvents[K]>(event, (e) => cb(e.payload));
    },
  };
}

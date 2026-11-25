import type {
  BackendEvents,
  ConfigResult,
  Config,
  HistoryPoint,
  InitialState,
  OscLogEntry,
  SecretKind,
  SecretStatus,
  SourceKind,
  UserError,
} from "../types";

/** Help pages the backend may open in the browser. */
export type HelpLink = "pulsoid_token" | "hyperate_api";

export type Unlisten = () => void;

/** Everything the UI can ask of the backend. Implemented by Tauri and by an in-browser mock. */
export interface Backend {
  readonly isMock: boolean;
  getState(): Promise<InitialState>;
  setConfig(config: Config): Promise<ConfigResult>;
  startScan(): Promise<void>;
  stopScan(): Promise<void>;
  connect(id: string, name: string): Promise<void>;
  disconnect(): Promise<void>;
  forget(): Promise<void>;
  selectSource(kind: SourceKind): Promise<void>;
  /** Stores a token/key in the OS credential store; returns masked statuses. */
  setSecret(kind: SecretKind, value: string): Promise<SecretStatus[]>;
  clearSecret(kind: SecretKind): Promise<SecretStatus[]>;
  openLink(link: HelpLink): Promise<void>;
  testSweep(on: boolean): Promise<void>;
  resetSession(): Promise<void>;
  getHistory(sinceMs: number): Promise<HistoryPoint[]>;
  getOscLog(): Promise<OscLogEntry[]>;
  exportConfig(): Promise<string | null>;
  importConfig(): Promise<ConfigResult | null>;
  resetConfig(): Promise<ConfigResult>;
  copyDiagnostics(): Promise<string>;
  openMini(): Promise<void>;
  closeMini(): Promise<void>;
  showMain(): Promise<void>;
  startDrag(): Promise<void>;
  on<K extends keyof BackendEvents>(event: K, cb: (payload: BackendEvents[K]) => void): Promise<Unlisten>;
}

/** Normalises anything thrown by the backend into a plain-language error. Never a stack trace. */
export function toUserError(e: unknown): UserError {
  if (e && typeof e === "object" && "title" in e && "message" in e && "fix" in e) {
    const u = e as UserError;
    return { code: String(u.code ?? "error"), title: String(u.title), message: String(u.message), fix: String(u.fix) };
  }
  return {
    code: "unexpected",
    title: "Something went wrong",
    message: "KittyTooth hit an unexpected problem.",
    fix: "Try again. If it keeps happening, use Copy diagnostics in Settings and report it.",
  };
}

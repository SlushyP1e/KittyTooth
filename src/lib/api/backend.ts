import type {
  BackendEvents,
  ConfigResult,
  Config,
  HistoryPoint,
  InitialState,
  OscLogEntry,
  SecretKind,
  Session,
  SessionSummary,
  SecretStatus,
  SourceKind,
  UpdateInfo,
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
  /** Fires a sample high alert (notification, sound, chatbox if chosen). */
  testAlert(): Promise<void>;
  resetSession(): Promise<void>;
  getHistory(sinceMs: number): Promise<HistoryPoint[]>;
  getOscLog(): Promise<OscLogEntry[]>;
  /** Saved sessions, newest first. */
  listSessions(): Promise<SessionSummary[]>;
  getSession(id: string): Promise<Session>;
  /** Deletes a saved session; returns the updated list. */
  deleteSession(id: string): Promise<SessionSummary[]>;
  /** Saves a session as CSV where the user picks; null if cancelled. */
  exportSession(id: string): Promise<string | null>;
  exportConfig(): Promise<string | null>;
  importConfig(): Promise<ConfigResult | null>;
  resetConfig(): Promise<ConfigResult>;
  copyDiagnostics(): Promise<string>;
  openMini(): Promise<void>;
  closeMini(): Promise<void>;
  showMain(): Promise<void>;
  startDrag(): Promise<void>;
  /** Asks GitHub for a newer version. Null when up to date. Never installs. */
  checkUpdate(): Promise<UpdateInfo | null>;
  /** Downloads, verifies and installs the update from the last check, then restarts. */
  installUpdate(): Promise<void>;
  /** Opens the GitHub release page for `version` (how portable copies update). */
  openRelease(version: string): Promise<void>;
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

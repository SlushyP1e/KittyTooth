import type { Backend, Unlisten } from "../api/backend";
import { toUserError } from "../api/backend";
import { defaultConfig, emptySnapshot } from "../defaults";
import type { AppInfo, Config, DiscoveredDevice, SecretKind, Snapshot, SourceKind, UserError } from "../types";
import { HistoryBuffer } from "./history";

export interface Toast {
  id: number;
  kind: "info" | "success" | "error";
  title: string;
  message?: string;
  fix?: string;
}

/** Global UI state. Fed by backend events; config edits are debounced and saved. */
export class AppStore {
  snapshot = $state<Snapshot>(emptySnapshot());
  config = $state<Config>(defaultConfig());
  devices = $state<DiscoveredDevice[]>([]);
  info = $state<AppInfo | null>(null);
  ready = $state(false);
  isMock = $state(false);
  saving = $state<"idle" | "pending" | "saved" | "error">("idle");
  toasts = $state<Toast[]>([]);
  /** Increments on every beat; components react to it. */
  beatCount = $state(0);
  beatInterval = $state(800);
  historyVersion = $state(0);
  readonly history = new HistoryBuffer();

  private backend: Backend | null = null;
  private unlisten: Unlisten[] = [];
  private saveTimer: ReturnType<typeof setTimeout> | null = null;
  private toastId = 0;
  saveDelayMs = 350;

  get api(): Backend {
    if (!this.backend) throw new Error("store not initialised");
    return this.backend;
  }

  async init(backend: Backend): Promise<void> {
    this.backend = backend;
    this.isMock = backend.isMock;
    this.unlisten.push(
      await backend.on("snapshot", (s) => (this.snapshot = s)),
      await backend.on("devices", (d) => (this.devices = d)),
      await backend.on("config", (c) => {
        if (this.saving !== "pending") this.config = c;
      }),
      await backend.on("beat", (b) => {
        this.beatInterval = b.interval_ms;
        this.beatCount++;
      }),
      await backend.on("sample", (p) => {
        this.history.push(p);
        this.historyVersion = this.history.version;
      }),
    );
    try {
      const st = await backend.getState();
      this.snapshot = st.snapshot;
      this.config = st.config;
      this.devices = st.devices;
      this.info = st.info;
      if (st.info.notice) this.toastError(st.info.notice);
      const hist = await backend.getHistory(0);
      this.history.set(hist);
      this.historyVersion = this.history.version;
    } catch (e) {
      this.toastError(toUserError(e));
    }
    this.ready = true;
  }

  destroy(): void {
    this.unlisten.forEach((u) => u());
    this.unlisten = [];
    if (this.saveTimer) clearTimeout(this.saveTimer);
  }

  /** Applies `fn` to a copy of the config and saves it (debounced). */
  updateConfig(fn: (c: Config) => void): void {
    const next = $state.snapshot(this.config) as Config;
    fn(next);
    this.config = next;
    this.saving = "pending";
    if (this.saveTimer) clearTimeout(this.saveTimer);
    this.saveTimer = setTimeout(() => void this.flushConfig(), this.saveDelayMs);
  }

  async flushConfig(): Promise<void> {
    if (this.saveTimer) {
      clearTimeout(this.saveTimer);
      this.saveTimer = null;
    }
    try {
      const r = await this.api.setConfig($state.snapshot(this.config) as Config);
      this.config = r.config;
      this.info = r.info;
      this.saving = "saved";
    } catch (e) {
      this.saving = "error";
      this.toastError(toUserError(e));
    }
  }

  /** Makes `kind` the main source; pending settings are saved first so it starts with them. */
  async selectSource(kind: SourceKind): Promise<void> {
    if (this.saving === "pending") await this.flushConfig();
    await this.run((a) => a.selectSource(kind));
  }

  /** Saves a token/key to the OS credential store. Returns true on success. */
  async saveSecret(kind: SecretKind, value: string | null): Promise<boolean> {
    const r = await this.run((a) => (value == null ? a.clearSecret(kind) : a.setSecret(kind, value)));
    if (!r) return false;
    if (this.info) this.info = { ...this.info, secrets: r };
    return true;
  }

  /** Runs a backend action, turning failures into an error toast. */
  async run<T>(fn: (api: Backend) => Promise<T>): Promise<T | undefined> {
    try {
      return await fn(this.api);
    } catch (e) {
      this.toastError(toUserError(e));
      return undefined;
    }
  }

  toast(t: Omit<Toast, "id">, ttlMs = 4000): void {
    const id = ++this.toastId;
    this.toasts = [...this.toasts, { ...t, id }];
    if (ttlMs > 0) setTimeout(() => this.dismiss(id), ttlMs);
  }

  toastError(e: UserError): void {
    this.toast({ kind: "error", title: e.title, message: e.message, fix: e.fix }, 9000);
  }

  dismiss(id: number): void {
    this.toasts = this.toasts.filter((t) => t.id !== id);
  }
}

export const app = new AppStore();

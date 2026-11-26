import type { Backend } from "../api/backend";
import { toUserError } from "../api/backend";
import type { UpdateInfo, UpdateProgress, UserError } from "../types";

/** Background checks run this often while the app is open. */
export const CHECK_INTERVAL_MS = 6 * 60 * 60 * 1000;
/** The launch check waits a little so it never competes with connecting the sensor. */
export const LAUNCH_DELAY_MS = 5000;

export type UpdateStatus = "idle" | "checking" | "up_to_date" | "available" | "downloading" | "error";

/**
 * In-app update state. Checks are quiet (no toasts, no dialogs): an available update shows
 * as a pill in the sidebar and in Settings → Updates. Nothing installs without a click.
 */
export class UpdateStore {
  status = $state<UpdateStatus>("idle");
  info = $state<UpdateInfo | null>(null);
  error = $state<UserError | null>(null);
  progress = $state<UpdateProgress | null>(null);
  lastChecked = $state<number | null>(null);

  private api: Backend | null = null;
  private timers: ReturnType<typeof setTimeout>[] = [];
  private unlisten: (() => void) | null = null;

  /** True while a download/install is running; checks are skipped then. */
  get busy(): boolean {
    return this.status === "checking" || this.status === "downloading";
  }

  /**
   * Starts the launch check and the 6-hourly timer. `autoCheck` is read each time, so turning
   * the setting off stops background checks without restarting anything.
   */
  async start(api: Backend, autoCheck: () => boolean, launchDelayMs = LAUNCH_DELAY_MS): Promise<void> {
    this.stop();
    this.api = api;
    this.unlisten = await api.on("update_progress", (p) => (this.progress = p));
    const auto = () => {
      if (autoCheck()) void this.check();
    };
    this.timers.push(setTimeout(auto, launchDelayMs), setInterval(auto, CHECK_INTERVAL_MS));
  }

  stop(): void {
    this.timers.forEach((t) => clearTimeout(t));
    this.timers = [];
    this.unlisten?.();
    this.unlisten = null;
  }

  /** Asks for a newer version. Failures land in `error` (shown in Settings), never a toast. */
  async check(): Promise<void> {
    if (!this.api || this.busy) return;
    const had = this.info;
    this.status = "checking";
    this.error = null;
    try {
      this.info = await this.api.checkUpdate();
      this.status = this.info ? "available" : "up_to_date";
      this.lastChecked = Date.now();
    } catch (e) {
      this.error = toUserError(e);
      // A failed background check keeps showing an update we already know about.
      this.info = had;
      this.status = had ? "available" : "error";
    }
  }

  /** Downloads, installs and restarts. Only ever called from the Install button. */
  async install(): Promise<boolean> {
    if (!this.api || !this.info?.can_install || this.busy) return false;
    this.status = "downloading";
    this.error = null;
    this.progress = null;
    try {
      await this.api.installUpdate();
      // The Tauri backend restarts the app before this returns; the demo backend lands here.
      this.info = null;
      this.status = "up_to_date";
      return true;
    } catch (e) {
      this.error = toUserError(e);
      this.status = "available";
      return false;
    } finally {
      this.progress = null;
    }
  }

  /** Portable copies update by downloading from the release page. */
  async openRelease(): Promise<void> {
    if (!this.api || !this.info) return;
    try {
      await this.api.openRelease(this.info.version);
    } catch (e) {
      this.error = toUserError(e);
    }
  }
}

export const updates = new UpdateStore();

<script lang="ts">
  import { app } from "../stores/app.svelte";
  import Toggle from "../components/Toggle.svelte";
  import Segmented from "../components/Segmented.svelte";
  import ErrorBanner from "../components/ErrorBanner.svelte";
  import { ACCENTS } from "../theme";
  import { ZONE_COLORS, ZONE_NAMES } from "../format";
  import type { Smoothing, Theme } from "../types";

  const c = $derived(app.config);
  const s = $derived(app.snapshot);
  const maxHr = $derived(c.zones.age ? 220 - c.zones.age : c.zones.manual_max_hr);
  let confirmReset = $state(false);
  let copied = $state(false);

  function setThreshold(i: number, v: number) {
    if (!Number.isFinite(v)) return;
    app.updateConfig((cfg) => {
      cfg.zones.thresholds[i] = Math.max(1, Math.min(100, Math.round(v)));
      cfg.zones.thresholds.sort((a, b) => a - b);
    });
  }

  async function copyDiagnostics() {
    const r = await app.run((a) => a.copyDiagnostics());
    if (r !== undefined) {
      copied = true;
      app.toast({ kind: "success", title: "Diagnostics copied", message: "Paste them into your bug report or Discord message." });
      setTimeout(() => (copied = false), 2500);
    }
  }

  async function copyOverlayUrl() {
    const url = app.info?.overlay_url;
    if (!url) return;
    try {
      await navigator.clipboard.writeText(url);
      app.toast({ kind: "success", title: "Overlay URL copied", message: "Add it in OBS as a Browser source." });
    } catch {
      app.toast({ kind: "info", title: "Copy this URL", message: url });
    }
  }

  async function exportCfg() {
    const p = await app.run((a) => a.exportConfig());
    if (p) app.toast({ kind: "success", title: "Settings exported", message: p });
  }

  async function importCfg() {
    const r = await app.run((a) => a.importConfig());
    if (r) {
      app.config = r.config;
      app.info = r.info;
      app.toast({ kind: "success", title: "Settings imported" });
    }
  }

  async function resetCfg() {
    const r = await app.run((a) => a.resetConfig());
    confirmReset = false;
    if (r) {
      app.config = r.config;
      app.info = r.info;
      app.toast({ kind: "success", title: "Settings reset to defaults" });
    }
  }
</script>

<section class="settings" aria-labelledby="set-title">
  <header class="row">
    <h1 id="set-title">Settings</h1>
    <span class="spacer"></span>
    <span class="saved muted" role="status" aria-live="polite">
      {app.saving === "pending" ? "Saving..." : app.saving === "saved" ? "All changes saved" : ""}
    </span>
  </header>

  <div class="card stack">
    <h2>Heart rate</h2>
    <div class="field">
      <span id="smooth-label">Smoothing</span>
      <Segmented
        label="Smoothing"
        value={c.processing.smoothing}
        options={[
          { value: "off", label: "Off" },
          { value: "light", label: "Light" },
          { value: "medium", label: "Medium" },
        ]}
        onchange={(v: Smoothing) => app.updateConfig((cfg) => (cfg.processing.smoothing = v))}
      />
      <span class="hint">Evens out jumpy readings. The raw value is always shown and available to OSC.</span>
    </div>
    <label class="field narrow">
      <span>Treat as disconnected after (seconds without data)</span>
      <input
        type="number"
        min="2"
        max="60"
        value={c.device.stale_timeout_secs}
        onchange={(e) =>
          app.updateConfig(
            (cfg) => (cfg.device.stale_timeout_secs = Math.min(60, Math.max(2, Math.round(Number(e.currentTarget.value) || 5)))),
          )}
      />
    </label>
  </div>

  <div class="card stack">
    <h2>Heart rate zones</h2>
    <div class="grid2">
      <label class="field">
        <span>Age (sets max HR to 220 - age)</span>
        <input
          type="number"
          min="1"
          max="120"
          placeholder="Not set"
          value={c.zones.age ?? ""}
          onchange={(e) => {
            const v = Math.round(Number(e.currentTarget.value));
            app.updateConfig((cfg) => (cfg.zones.age = e.currentTarget.value === "" || !(v >= 1 && v <= 120) ? null : v));
          }}
        />
      </label>
      <label class="field">
        <span>Max heart rate (used when age is empty)</span>
        <input
          type="number"
          min="100"
          max="250"
          disabled={c.zones.age != null}
          value={c.zones.manual_max_hr}
          onchange={(e) =>
            app.updateConfig(
              (cfg) => (cfg.zones.manual_max_hr = Math.min(250, Math.max(100, Math.round(Number(e.currentTarget.value) || 190)))),
            )}
        />
      </label>
    </div>
    <p class="hint">Using max heart rate <strong class="tabular">{maxHr} bpm</strong>. Zone lower bounds in % of max:</p>
    <div class="zones">
      {#each c.zones.thresholds as t, i (i)}
        <label class="field zone" style:--z={ZONE_COLORS[i + 1]}>
          <span>Zone {i + 1} · {ZONE_NAMES[i + 1]}</span>
          <div class="row tight">
            <input type="number" min="1" max="100" value={t} onchange={(e) => setThreshold(i, Number(e.currentTarget.value))} />
            <span class="muted tabular">{Math.round((maxHr * t) / 100)}</span>
          </div>
        </label>
      {/each}
    </div>
  </div>

  <div class="card stack">
    <h2>Appearance</h2>
    <div class="field">
      <span>Theme</span>
      <Segmented
        label="Theme"
        value={c.ui.theme}
        options={[
          { value: "dark", label: "Dark" },
          { value: "light", label: "Light" },
          { value: "system", label: "System" },
        ]}
        onchange={(v: Theme) => app.updateConfig((cfg) => (cfg.ui.theme = v))}
      />
    </div>
    <div class="field">
      <span id="accent-label">Accent color</span>
      <div class="row swatches" role="radiogroup" aria-labelledby="accent-label">
        {#each ACCENTS as a (a)}
          <button
            class="swatch"
            role="radio"
            aria-checked={c.ui.accent.toLowerCase() === a}
            aria-label="Accent {a}"
            style:background={a}
            onclick={() => app.updateConfig((cfg) => (cfg.ui.accent = a))}
          ></button>
        {/each}
        <label class="custom-color">
          <input
            type="color"
            value={c.ui.accent}
            aria-label="Custom accent color"
            oninput={(e) => app.updateConfig((cfg) => (cfg.ui.accent = e.currentTarget.value))}
          />
          <span class="muted">Custom</span>
        </label>
      </div>
    </div>
  </div>

  <div class="card stack">
    <h2>Startup and windows</h2>
    <Toggle
      label="Start with Windows"
      checked={c.ui.autostart}
      onchange={(v) => app.updateConfig((cfg) => (cfg.ui.autostart = v))}
    />
    <Toggle
      label="Start minimized to tray"
      hint="KittyTooth keeps sending OSC from the tray; the window isn't needed."
      checked={c.ui.start_minimized}
      onchange={(v) => app.updateConfig((cfg) => (cfg.ui.start_minimized = v))}
    />
    <Toggle
      label="Closing the window keeps KittyTooth in the tray"
      checked={c.ui.close_to_tray}
      onchange={(v) => app.updateConfig((cfg) => (cfg.ui.close_to_tray = v))}
    />
    <div class="row">
      <button class="btn" onclick={() => app.run((a) => a.openMini())}>Open mini window</button>
      <span class="spacer"></span>
    </div>
    <Toggle
      label="Mini window always on top"
      hint="A compact BPM + heart window for streaming setups."
      checked={c.ui.mini_always_on_top}
      onchange={(v) => app.updateConfig((cfg) => (cfg.ui.mini_always_on_top = v))}
    />
  </div>

  <div class="card stack">
    <h2>OBS overlay</h2>
    <Toggle
      label="Local browser source"
      hint="A transparent overlay for OBS / Streamlabs. Only reachable from this PC."
      checked={c.overlay.enabled}
      onchange={(v) => app.updateConfig((cfg) => (cfg.overlay.enabled = v))}
    />
    {#if c.overlay.enabled}
      <div class="row wrap">
        <label class="field narrow">
          <span>Port</span>
          <input
            type="number"
            min="1024"
            max="65535"
            value={c.overlay.port}
            onchange={(e) => {
              const v = Math.round(Number(e.currentTarget.value));
              if (v >= 1024 && v <= 65535) app.updateConfig((cfg) => (cfg.overlay.port = v));
            }}
          />
        </label>
        {#if app.info?.overlay_url}
          <code class="url mono">{app.info.overlay_url}</code>
          <button class="btn small" onclick={copyOverlayUrl}>Copy URL</button>
        {/if}
      </div>
      {#if app.info?.overlay_error}
        <ErrorBanner error={app.info.overlay_error} />
      {/if}
      <p class="hint">Options: add <span class="mono">?size=120&amp;color=ff6fa8&amp;hide=1</span> to the URL.</p>
    {/if}
  </div>

  <div class="card stack">
    <h2>Your settings</h2>
    <div class="row wrap">
      <button class="btn" onclick={exportCfg}>Export...</button>
      <button class="btn" onclick={importCfg}>Import...</button>
      <span class="spacer"></span>
      {#if confirmReset}
        <span class="muted">Reset everything to defaults?</span>
        <button class="btn danger" onclick={resetCfg}>Yes, reset</button>
        <button class="btn ghost" onclick={() => (confirmReset = false)}>Cancel</button>
      {:else}
        <button class="btn ghost danger" onclick={() => (confirmReset = true)}>Reset to defaults</button>
      {/if}
    </div>
  </div>

  <div class="card stack">
    <h2>Diagnostics</h2>
    <Toggle
      label="Debug logging"
      hint="Logs BLE-to-OSC latency for every packet. Leave off normally."
      checked={c.debug.log_latency}
      onchange={(v) => app.updateConfig((cfg) => (cfg.debug.log_latency = v))}
    />
    <dl class="diag tabular">
      <div><dt>Version</dt><dd>{app.info?.version ?? "--"}</dd></div>
      <div><dt>Platform</dt><dd>{app.info?.platform ?? "--"}</dd></div>
      <div>
        <dt>BLE → OSC latency</dt>
        <dd>
          {s.latency.samples
            ? `${(s.latency.avg_us / 1000).toFixed(2)} ms avg · ${(s.latency.max_us / 1000).toFixed(2)} ms max`
            : "--"}
        </dd>
      </div>
      <div><dt>Packets</dt><dd>{s.packets} · {s.rejected_spikes} spikes rejected</dd></div>
      <div><dt>Logs</dt><dd class="mono">{app.info?.log_dir ?? "--"}</dd></div>
    </dl>
    <div><button class="btn" onclick={copyDiagnostics}>{copied ? "Copied" : "Copy diagnostics"}</button></div>
  </div>
</section>

<style>
  .settings {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .narrow {
    max-width: 320px;
  }
  .grid2 {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 14px;
  }
  .zones {
    display: grid;
    grid-template-columns: repeat(5, minmax(0, 1fr));
    gap: 10px;
  }
  .zone > span {
    border-left: 3px solid var(--z);
    padding-left: 6px;
  }
  .zone input {
    width: 70px;
  }
  .tight {
    gap: 8px;
  }
  .wrap {
    flex-wrap: wrap;
  }
  .swatches {
    gap: 10px;
    flex-wrap: wrap;
  }
  .swatch {
    width: 28px;
    height: 28px;
    border-radius: 50%;
    border: 2px solid transparent;
    cursor: pointer;
    padding: 0;
  }
  .swatch[aria-checked="true"] {
    border-color: var(--text);
    box-shadow: 0 0 0 2px var(--bg) inset;
  }
  .custom-color {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    cursor: pointer;
  }
  input[type="color"] {
    width: 32px;
    height: 32px;
    padding: 0;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: none;
    cursor: pointer;
  }
  .url {
    padding: 6px 10px;
    border-radius: 8px;
    background: var(--surface-2);
    align-self: flex-end;
  }
  .diag {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 8px 20px;
    margin: 0;
  }
  .diag dt {
    font-size: 12px;
    color: var(--muted);
  }
  .diag dd {
    margin: 0;
    word-break: break-all;
  }
  .saved {
    font-size: 13px;
  }
  @media (max-width: 900px) {
    .zones {
      grid-template-columns: repeat(3, minmax(0, 1fr));
    }
    .grid2,
    .diag {
      grid-template-columns: 1fr;
    }
  }
</style>

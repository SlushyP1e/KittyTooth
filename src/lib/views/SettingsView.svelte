<script lang="ts">
  import { app } from "../stores/app.svelte";
  import Toggle from "../components/Toggle.svelte";
  import Choice from "../components/Choice.svelte";
  import ErrorBanner from "../components/ErrorBanner.svelte";
  import UpdatesSection from "../components/UpdatesSection.svelte";
  import { ACCENT_PRESETS, isCustomAccent } from "../theme";
  import StatusChip from "../components/StatusChip.svelte";
  import { ZONE_NAMES, steamVrStatus } from "../format";
  import type { Motion, Smoothing, Theme, VrAttach, VrFadeAngle, VrHand, VrSize } from "../types";

  const c = $derived(app.config);
  const s = $derived(app.snapshot);
  const maxHr = $derived(c.zones.age ? 220 - c.zones.age : c.zones.manual_max_hr);
  const a = $derived(c.alerts);
  const vr = $derived(c.steamvr);
  const vrStatus = $derived(steamVrStatus(app.steamvr));
  const OPACITY_OPTIONS = [40, 60, 80, 90, 100].map((v) => ({ value: String(v), label: `${v}%` }));
  let confirmReset = $state(false);

  function setAlertNum(key: "high_bpm" | "low_bpm" | "sustain_secs", raw: string, min: number, max: number) {
    const v = Math.round(Number(raw));
    if (!Number.isFinite(v)) return;
    app.updateConfig((cfg) => (cfg.alerts[key] = Math.min(max, Math.max(min, v))));
  }

  async function testAlert() {
    if (app.saving === "pending") await app.flushConfig();
    await app.run((api) => api.testAlert());
  }
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
      app.toast({ kind: "success", title: "Diagnostics copied", message: "Paste them into your bug report." });
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

<section class="page" aria-labelledby="set-title">
  <header class="page-head">
    <div class="titles">
      <h1 id="set-title" class="page-title">Settings</h1>
      <p class="subtitle" role="status" aria-live="polite">
        {app.saving === "pending" ? "Saving" : app.saving === "saved" ? "All changes saved" : "Changes save automatically"}
      </p>
    </div>
  </header>

  <div class="section">
    <h2 class="section-label">Heart rate</h2>
    <div class="hairlist">
      <div class="srow">
        <div class="stext">
          <span class="slabel">Smoothing</span>
          <p class="shelp">Evens out jumpy readings. The raw value stays available.</p>
        </div>
        <div class="sctrl">
          <Choice
            label="Smoothing"
            value={c.processing.smoothing}
            options={[
              { value: "off", label: "off" },
              { value: "light", label: "light" },
              { value: "medium", label: "medium" },
            ]}
            onchange={(v: Smoothing) => app.updateConfig((cfg) => (cfg.processing.smoothing = v))}
          />
        </div>
      </div>
      <div class="srow">
        <div class="stext">
          <label class="slabel" for="stale">Stale timeout</label>
          <p class="shelp">Seconds without data before the sensor counts as disconnected.</p>
        </div>
        <div class="sctrl">
          <input
            id="stale"
            type="number"
            min="2"
            max="60"
            value={c.device.stale_timeout_secs}
            onchange={(e) =>
              app.updateConfig(
                (cfg) =>
                  (cfg.device.stale_timeout_secs = Math.min(60, Math.max(2, Math.round(Number(e.currentTarget.value) || 5)))),
              )}
          />
        </div>
      </div>
    </div>
  </div>

  <div class="section">
    <h2 class="section-label">Zones <span class="spacer"></span><span>max hr {maxHr}</span></h2>
    <div class="hairlist">
      <div class="srow">
        <div class="stext">
          <label class="slabel" for="age">Age</label>
          <p class="shelp">Sets max heart rate to 220 minus age. Leave empty to set it yourself.</p>
        </div>
        <div class="sctrl">
          <input
            id="age"
            type="number"
            min="1"
            max="120"
            placeholder="--"
            value={c.zones.age ?? ""}
            onchange={(e) => {
              const v = Math.round(Number(e.currentTarget.value));
              app.updateConfig((cfg) => (cfg.zones.age = e.currentTarget.value === "" || !(v >= 1 && v <= 120) ? null : v));
            }}
          />
        </div>
      </div>
      <div class="srow">
        <div class="stext">
          <label class="slabel" for="maxhr">Max heart rate</label>
          <p class="shelp">Used when age is empty.</p>
        </div>
        <div class="sctrl">
          <input
            id="maxhr"
            type="number"
            min="100"
            max="250"
            disabled={c.zones.age != null}
            value={c.zones.manual_max_hr}
            onchange={(e) =>
              app.updateConfig(
                (cfg) =>
                  (cfg.zones.manual_max_hr = Math.min(250, Math.max(100, Math.round(Number(e.currentTarget.value) || 190)))),
              )}
          />
        </div>
      </div>
      <div class="srow stacked">
        <div class="stext">
          <span class="slabel">Zone thresholds</span>
          <p class="shelp">Lower bound of each zone, in percent of max heart rate.</p>
        </div>
        <div class="zones">
          {#each c.zones.thresholds as t, i (i)}
            <label class="field">
              <span class="flabel">{i + 1} · {ZONE_NAMES[i + 1].toLowerCase()}</span>
              <span class="row">
                <input type="number" min="1" max="100" value={t} onchange={(e) => setThreshold(i, Number(e.currentTarget.value))} />
                <span class="mono muted small">{Math.round((maxHr * t) / 100)}</span>
              </span>
            </label>
          {/each}
        </div>
      </div>
    </div>
  </div>

  <div class="section">
    <h2 class="section-label">Alerts</h2>
    <div class="hairlist">
      <Toggle
        label="Heart rate alerts"
        hint="Warns you when your heart rate stays too high or too low. Not a medical device: use it as a reminder, not for diagnosis."
        checked={a.enabled}
        onchange={(v) => app.updateConfig((cfg) => (cfg.alerts.enabled = v))}
      />
      <div class="srow">
        <div class="stext">
          <label class="slabel" for="al-high">High alert</label>
          <p class="shelp">Alert at or above this heart rate.</p>
        </div>
        <div class="sctrl row">
          <input
            id="al-high"
            type="number"
            min="40"
            max="250"
            disabled={!a.enabled}
            value={a.high_bpm}
            onchange={(e) => setAlertNum("high_bpm", e.currentTarget.value, 40, 250)}
          /><span class="mono muted small">bpm</span>
        </div>
      </div>
      <div class="srow">
        <div class="stext">
          <label class="slabel" for="al-low">Low alert</label>
          <p class="shelp">Alert at or below this heart rate.</p>
        </div>
        <div class="sctrl row">
          <input
            id="al-low"
            type="number"
            min="25"
            max="200"
            disabled={!a.enabled}
            value={a.low_bpm}
            onchange={(e) => setAlertNum("low_bpm", e.currentTarget.value, 25, 200)}
          /><span class="mono muted small">bpm</span>
        </div>
      </div>
      <div class="srow">
        <div class="stext">
          <label class="slabel" for="al-sustain">Wait before alerting</label>
          <p class="shelp">Seconds it must stay out of range first, so a single spike doesn't set it off. 0 alerts right away.</p>
        </div>
        <div class="sctrl row">
          <input
            id="al-sustain"
            type="number"
            min="0"
            max="300"
            disabled={!a.enabled}
            value={a.sustain_secs}
            onchange={(e) => setAlertNum("sustain_secs", e.currentTarget.value, 0, 300)}
          /><span class="mono muted small">s</span>
        </div>
      </div>
      <div class="srow">
        <div class="stext">
          <span class="slabel">Remind me</span>
          <p class="shelp">Repeats the alert while your heart rate stays out of range.</p>
        </div>
        <div class="sctrl">
          <Choice
            label="Remind me"
            value={String(a.repeat_secs)}
            options={[
              { value: "0", label: "once" },
              { value: "60", label: "1 min" },
              { value: "120", label: "2 min" },
              { value: "300", label: "5 min" },
            ]}
            onchange={(v: string) => app.updateConfig((cfg) => (cfg.alerts.repeat_secs = Number(v)))}
          />
        </div>
      </div>
      <Toggle
        label="Desktop notification"
        checked={a.notify}
        disabled={!a.enabled}
        onchange={(v) => app.updateConfig((cfg) => (cfg.alerts.notify = v))}
      />
      <Toggle
        label="Sound"
        hint="A rising chime for high, a falling one for low."
        checked={a.sound}
        disabled={!a.enabled}
        onchange={(v) => app.updateConfig((cfg) => (cfg.alerts.sound = v))}
      />
      <Toggle
        label="Show in VRChat chatbox"
        hint="Posts the alert above your head so friends nearby can see it, with the chatbox sound. Works even when the regular chatbox is off."
        checked={a.chatbox}
        disabled={!a.enabled}
        onchange={(v) => app.updateConfig((cfg) => (cfg.alerts.chatbox = v))}
      />
      {#if a.chatbox}
        <div class="srow stacked">
          <div class="stext">
            <span class="slabel">Chatbox messages</span>
            <p class="shelp">Same tokens as the chatbox template, like {"{hr}"}. Up to 144 characters.</p>
          </div>
          <div class="texts">
            <label class="field">
              <span class="flabel">high</span>
              <input
                type="text"
                maxlength="144"
                disabled={!a.enabled}
                value={a.high_text}
                onchange={(e) => app.updateConfig((cfg) => (cfg.alerts.high_text = e.currentTarget.value))}
              />
            </label>
            <label class="field">
              <span class="flabel">low</span>
              <input
                type="text"
                maxlength="144"
                disabled={!a.enabled}
                value={a.low_text}
                onchange={(e) => app.updateConfig((cfg) => (cfg.alerts.low_text = e.currentTarget.value))}
              />
            </label>
          </div>
        </div>
      {/if}
      <div class="srow">
        <div class="stext">
          <span class="slabel">Try it</span>
          <p class="shelp">
            Fires a sample high alert with your current choices. No heart rate needed. Avatars using the
            KittyTooth preset also get <span class="mono">KittyTooth/Alert</span> (1 high, 2 low) to animate.
          </p>
        </div>
        <div class="sctrl">
          <button class="btn" disabled={!a.enabled} onclick={testAlert}>Test alert</button>
        </div>
      </div>
    </div>
  </div>

  <div class="section">
    <h2 class="section-label">SteamVR <span class="spacer"></span><StatusChip status={vrStatus} /></h2>
    <div class="hairlist">
      <Toggle
        label="Heart rate in your headset"
        hint="A small overlay with your BPM and a controller buzz on alerts, for PC VR through SteamVR. Optional: without SteamVR nothing changes."
        checked={vr.enabled}
        onchange={(v) => app.updateConfig((cfg) => (cfg.steamvr.enabled = v))}
      />
      {#if vr.enabled}
        <div class="srow">
          <div class="stext">
            <span class="slabel">Status</span>
            <p class="shelp" role="status" aria-live="polite">{vrStatus.help}</p>
          </div>
        </div>
        <Toggle
          label="Show overlay"
          hint="Turns red for a high alert and amber for a low one."
          checked={vr.overlay}
          onchange={(v) => app.updateConfig((cfg) => (cfg.steamvr.overlay = v))}
        />
        <div class="srow">
          <div class="stext">
            <span class="slabel">Position</span>
            <p class="shelp">On a controller like a watch, or always in view.</p>
          </div>
          <div class="sctrl">
            <Choice
              label="Overlay position"
              value={vr.attach}
              options={[
                { value: "left_wrist", label: "left wrist" },
                { value: "right_wrist", label: "right wrist" },
                { value: "hud", label: "in view" },
              ]}
              onchange={(v: VrAttach) => app.updateConfig((cfg) => (cfg.steamvr.attach = v))}
            />
          </div>
        </div>
        <div class="srow">
          <div class="stext"><span class="slabel">Size</span></div>
          <div class="sctrl">
            <Choice
              label="Overlay size"
              value={vr.size}
              options={[
                { value: "small", label: "small" },
                { value: "medium", label: "medium" },
                { value: "large", label: "large" },
              ]}
              onchange={(v: VrSize) => app.updateConfig((cfg) => (cfg.steamvr.size = v))}
            />
          </div>
        </div>
        <div class="srow">
          <div class="stext"><span class="slabel">Opacity</span></div>
          <div class="sctrl">
            <Choice
              label="Overlay opacity"
              value={String(vr.opacity)}
              options={OPACITY_OPTIONS}
              onchange={(v: string) => app.updateConfig((cfg) => (cfg.steamvr.opacity = Number(v)))}
            />
          </div>
        </div>
        {#if vr.attach !== "hud"}
          <Toggle
            label="Fade when not looking"
            hint="Like a watch: the wrist overlay fades in when you look at it and out when you look away. Alerts always show."
            checked={vr.fade}
            onchange={(v) => app.updateConfig((cfg) => (cfg.steamvr.fade = v))}
          />
          {#if vr.fade}
            <div class="srow">
              <div class="stext">
                <span class="slabel">Look angle</span>
                <p class="shelp">How close to the middle of your view it must be to show.</p>
              </div>
              <div class="sctrl">
                <Choice
                  label="Look angle"
                  value={vr.fade_angle}
                  options={[
                    { value: "narrow", label: "narrow" },
                    { value: "normal", label: "normal" },
                    { value: "wide", label: "wide" },
                  ]}
                  onchange={(v: VrFadeAngle) => app.updateConfig((cfg) => (cfg.steamvr.fade_angle = v))}
                />
              </div>
            </div>
          {/if}
        {/if}
        <Toggle
          label="Buzz on alerts"
          hint="A heartbeat-like double pulse when a heart rate alert fires or reminds you. Uses the alert settings above; Test alert tries it."
          checked={vr.haptics}
          onchange={(v) => app.updateConfig((cfg) => (cfg.steamvr.haptics = v))}
        />
        <div class="srow">
          <div class="stext"><span class="slabel">Buzz which hand</span></div>
          <div class="sctrl">
            <Choice
              label="Buzz which hand"
              value={vr.haptics_hand}
              options={[
                { value: "left", label: "left" },
                { value: "right", label: "right" },
                { value: "both", label: "both" },
              ]}
              onchange={(v: VrHand) => app.updateConfig((cfg) => (cfg.steamvr.haptics_hand = v))}
            />
          </div>
        </div>
      {/if}
    </div>
  </div>

  <div class="section">
    <h2 class="section-label">Appearance</h2>
    <div class="hairlist">
      <div class="srow">
        <div class="stext"><span class="slabel">Theme</span></div>
        <div class="sctrl">
          <Choice
            label="Theme"
            value={c.ui.theme}
            options={[
              { value: "dark", label: "dark" },
              { value: "light", label: "light" },
              { value: "system", label: "system" },
            ]}
            onchange={(v: Theme) => app.updateConfig((cfg) => (cfg.ui.theme = v))}
          />
        </div>
      </div>
      <div class="srow">
        <div class="stext">
          <span class="slabel">Motion</span>
          <p class="shelp">Springy animations. "system" follows the Windows animation effects setting.</p>
        </div>
        <div class="sctrl">
          <Choice
            label="Motion"
            value={c.ui.motion ?? "on"}
            options={[
              { value: "on", label: "on" },
              { value: "reduced", label: "reduced" },
              { value: "system", label: "system" },
            ]}
            onchange={(v: Motion) => app.updateConfig((cfg) => (cfg.ui.motion = v))}
          />
        </div>
      </div>
      <div class="srow">
        <div class="stext">
          <span class="slabel" id="accent-label">Accent</span>
          <p class="shelp">One accent color for the whole app, tray icon included.</p>
        </div>
        <div class="sctrl swatches" role="radiogroup" aria-labelledby="accent-label">
          {#each ACCENT_PRESETS as a (a)}
            <button
              class="swatch"
              role="radio"
              aria-checked={c.ui.accent === a}
              aria-label="Accent {a}"
              title={a}
              style:background="var(--swatch-{a})"
              onclick={() => app.updateConfig((cfg) => (cfg.ui.accent = a))}
            ></button>
          {/each}
          <label class="custom" class:on={isCustomAccent(c.ui.accent)} title="Custom color">
            <input
              type="color"
              value={isCustomAccent(c.ui.accent) ? c.ui.accent : undefined}
              aria-label="Custom accent color"
              oninput={(e) => app.updateConfig((cfg) => (cfg.ui.accent = e.currentTarget.value))}
            />
            <span class="ring" aria-hidden="true" style:background={isCustomAccent(c.ui.accent) ? c.ui.accent : undefined}></span>
            <span class="mono small">custom</span>
          </label>
        </div>
      </div>
    </div>
  </div>

  <div class="section">
    <h2 class="section-label">Startup and windows</h2>
    <div class="hairlist">
      <Toggle label="Start with Windows" checked={c.ui.autostart} onchange={(v) => app.updateConfig((cfg) => (cfg.ui.autostart = v))} />
      <Toggle
        label="Start minimized to tray"
        hint="OSC keeps running from the tray; the window isn't needed."
        checked={c.ui.start_minimized}
        onchange={(v) => app.updateConfig((cfg) => (cfg.ui.start_minimized = v))}
      />
      <Toggle
        label="Close to tray"
        hint="Closing the window keeps KittyTooth running in the tray."
        checked={c.ui.close_to_tray}
        onchange={(v) => app.updateConfig((cfg) => (cfg.ui.close_to_tray = v))}
      />
      <div class="srow">
        <div class="stext">
          <span class="slabel">Mini window</span>
          <p class="shelp">Just the heart and BPM, for streaming setups.</p>
        </div>
        <div class="sctrl"><button class="btn" onclick={() => app.run((a) => a.openMini())}>Open mini window</button></div>
      </div>
      <Toggle
        label="Mini window always on top"
        checked={c.ui.mini_always_on_top}
        onchange={(v) => app.updateConfig((cfg) => (cfg.ui.mini_always_on_top = v))}
      />
    </div>
  </div>

  <UpdatesSection />

  <div class="section">
    <h2 class="section-label">OBS overlay</h2>
    <div class="hairlist">
      <Toggle
        label="Local browser source"
        hint="A transparent overlay for OBS. Only reachable from this PC."
        checked={c.overlay.enabled}
        onchange={(v) => app.updateConfig((cfg) => (cfg.overlay.enabled = v))}
      />
      {#if c.overlay.enabled}
        <div class="srow">
          <div class="stext">
            <label class="slabel" for="ovport">Port</label>
            <p class="shelp">Add ?size=120&amp;hide=1&amp;pill=1 to the URL to customize.</p>
          </div>
          <div class="sctrl">
            <input
              id="ovport"
              type="number"
              min="1024"
              max="65535"
              value={c.overlay.port}
              onchange={(e) => {
                const v = Math.round(Number(e.currentTarget.value));
                if (v >= 1024 && v <= 65535) app.updateConfig((cfg) => (cfg.overlay.port = v));
              }}
            />
          </div>
        </div>
        {#if app.info?.overlay_url}
          <div class="srow">
            <div class="stext">
              <span class="slabel">URL</span>
              <p class="shelp mono url">{app.info.overlay_url}</p>
            </div>
            <div class="sctrl"><button class="btn small" onclick={copyOverlayUrl}>Copy URL</button></div>
          </div>
        {/if}
      {/if}
    </div>
    {#if c.overlay.enabled && app.info?.overlay_error}
      <ErrorBanner error={app.info.overlay_error} />
    {/if}
  </div>

  <div class="section">
    <h2 class="section-label">Your settings</h2>
    <div class="hairlist">
      <div class="srow">
        <div class="stext">
          <span class="slabel">Backup</span>
          <p class="shelp">Save your settings to a file or load them on another PC.</p>
        </div>
        <div class="sctrl">
          <button class="btn" onclick={exportCfg}>Export</button>
          <button class="btn" onclick={importCfg}>Import</button>
        </div>
      </div>
      <div class="srow">
        <div class="stext">
          <span class="slabel">Reset</span>
          <p class="shelp">{confirmReset ? "This forgets your sensor and every setting." : "Back to defaults."}</p>
        </div>
        <div class="sctrl">
          {#if confirmReset}
            <button class="btn danger" onclick={resetCfg}>Yes, reset</button>
            <button class="btn quiet" onclick={() => (confirmReset = false)}>Cancel</button>
          {:else}
            <button class="btn danger" onclick={() => (confirmReset = true)}>Reset to defaults</button>
          {/if}
        </div>
      </div>
    </div>
  </div>

  <div class="section">
    <h2 class="section-label">Diagnostics</h2>
    <div class="hairlist">
      <Toggle
        label="Debug logging"
        hint="Logs BLE to OSC latency for every packet. Leave off normally."
        checked={c.debug.log_latency}
        onchange={(v) => app.updateConfig((cfg) => (cfg.debug.log_latency = v))}
      />
      <div class="srow facts">
        <dl class="mono">
          <div><dt>version</dt><dd>{app.info?.version ?? "--"}</dd></div>
          <div><dt>platform</dt><dd>{app.info?.platform ?? "--"}</dd></div>
          <div>
            <dt>ble to osc</dt>
            <dd>
              {s.latency.samples
                ? `${(s.latency.avg_us / 1000).toFixed(2)} ms avg / ${(s.latency.max_us / 1000).toFixed(2)} max`
                : "--"}
            </dd>
          </div>
          <div><dt>packets</dt><dd>{s.packets} · {s.rejected_spikes} spikes rejected</dd></div>
          <div class="wide"><dt>logs</dt><dd>{app.info?.log_dir ?? "--"}</dd></div>
        </dl>
      </div>
      <div class="srow">
        <div class="stext">
          <span class="slabel">Copy diagnostics</span>
          <p class="shelp">Status, settings and recent log lines, ready for a bug report.</p>
        </div>
        <div class="sctrl"><button class="btn" onclick={copyDiagnostics}>{copied ? "Copied" : "Copy diagnostics"}</button></div>
      </div>
    </div>
  </div>
</section>

<style>
  .zones {
    display: grid;
    grid-template-columns: repeat(5, minmax(0, 1fr));
    gap: var(--s3);
  }
  .zones input {
    width: 64px;
  }
  .small {
    font-size: 12px;
  }
  .texts {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--s3);
  }
  .texts input {
    width: 100%;
  }
  .swatches {
    gap: 10px;
  }
  .swatch {
    width: 22px;
    height: 22px;
    border-radius: 50%;
    border: 0;
    padding: 0;
    cursor: pointer;
  }
  .swatch[aria-checked="true"] {
    box-shadow:
      0 0 0 2px var(--slate-1),
      0 0 0 4px var(--text);
  }
  .custom {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--muted);
    cursor: pointer;
    position: relative;
  }
  .custom.on {
    color: var(--text);
  }
  input[type="color"] {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    opacity: 0;
    cursor: pointer;
  }
  .ring {
    width: 24px;
    height: 24px;
    border-radius: 50%;
    border: 1px dashed var(--muted);
  }
  .custom.on .ring {
    border: 0;
    box-shadow:
      0 0 0 2px var(--slate-1),
      0 0 0 3.5px var(--text);
  }
  .custom:has(input:focus-visible) {
    outline: 2px solid var(--pink);
    outline-offset: 3px;
    border-radius: var(--r-btn);
  }
  .url {
    color: var(--pink);
  }
  .facts dl {
    flex: 1;
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--s3) var(--s5);
    margin: 0;
    font-size: 12px;
  }
  .facts dt {
    color: var(--muted);
    font-size: 11px;
  }
  .facts dd {
    margin: 2px 0 0;
    color: var(--body);
    word-break: break-all;
  }
  .facts .wide {
    grid-column: 1 / -1;
  }
  @media (max-width: 980px) {
    .zones {
      grid-template-columns: repeat(3, minmax(0, 1fr));
    }
  }
</style>

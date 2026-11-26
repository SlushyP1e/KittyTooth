<script lang="ts">
  import { app } from "../stores/app.svelte";
  import Toggle from "../components/Toggle.svelte";
  import Choice from "../components/Choice.svelte";
  import StatusChip from "../components/StatusChip.svelte";
  import ErrorBanner from "../components/ErrorBanner.svelte";
  import { connectionStatus, SOURCE_INFO, SOURCE_ORDER } from "../format";
  import type { ConnState, SecretKind, SlotStatus, SourceKind } from "../types";

  let { navigate }: { navigate: (v: string) => void } = $props();

  const s = $derived(app.snapshot);
  const cfg = $derived(app.config.source);
  const active = $derived(cfg.active);
  const primary = $derived<SlotStatus | undefined>(s.sources.find((x) => x.role === "primary"));
  const backup = $derived<SlotStatus | undefined>(s.sources.find((x) => x.role === "backup"));
  const running = $derived(!!primary);
  const secret = (k: SecretKind) => app.info?.secrets.find((x) => x.kind === k);

  type Dot = "live" | "ok" | "warn" | "err" | "idle";
  function dotFor(slot: SlotStatus | undefined): Dot {
    if (!slot) return "idle";
    if (slot.error && (slot.state === "idle" || slot.state === "reconnecting")) return "err";
    if (slot.live && (s.data_valid || s.test_sweep)) return "live";
    const st: ConnState = slot.state;
    return st === "connected" ? "ok" : st === "idle" ? "idle" : "warn";
  }
  function slotOf(k: SourceKind) {
    return s.sources.find((x) => x.kind === k);
  }

  const subtitle = $derived(
    s.fallback_active && backup
      ? `${SOURCE_INFO[active].name} went quiet, reading from ${SOURCE_INFO[backup.kind].name} for now`
      : running
        ? `Reading from ${SOURCE_INFO[active].name}`
        : "Choose where your heart rate comes from",
  );

  // Secret entry: the value only lives in this input until it's saved to the credential store.
  let tokenDraft = $state("");
  let keyDraft = $state("");
  let editingToken = $state(false);
  let editingKey = $state(false);
  async function saveSecret(kind: SecretKind, value: string | null) {
    const ok = await app.saveSecret(kind, value);
    if (!ok) return;
    if (kind === "pulsoid_token") {
      tokenDraft = "";
      editingToken = false;
    } else {
      keyDraft = "";
      editingKey = false;
    }
    app.toast({ kind: "success", title: value == null ? "Removed" : "Saved to the credential store" }, 2500);
  }

  const oscConflict = $derived(
    app.config.osc.targets.some(
      (t) =>
        t.enabled &&
        t.port === cfg.osc_input.port &&
        (t.host === "127.0.0.1" || t.host === "localhost" || cfg.osc_input.lan),
    ),
  );
  const backupOptions = $derived(
    SOURCE_ORDER.filter((k) => k !== active).map((k) => ({ value: k, label: SOURCE_INFO[k].tag })),
  );
</script>

<section class="page" aria-labelledby="src-title">
  <header class="page-head">
    <div class="titles">
      <h1 id="src-title" class="page-title">Source</h1>
      <p class="subtitle" role="status">{subtitle}</p>
    </div>
    {#if running}
      <button class="btn" onclick={() => app.run((a) => a.disconnect())}>Stop</button>
    {:else}
      <button class="btn primary" onclick={() => app.selectSource(active)}>Start</button>
    {/if}
  </header>

  {#if primary?.error}
    <ErrorBanner error={primary.error} />
  {/if}

  <div class="section">
    <h2 class="section-label">Heart rate source</h2>
    <ul class="hairlist" aria-label="Heart rate sources">
      {#each SOURCE_ORDER as k (k)}
        {@const slot = slotOf(k)}
        {@const isMain = k === active}
        <li class="srow src" class:main={isMain}>
          <span class="dot {dotFor(slot)}" aria-hidden="true"></span>
          <div class="stext">
            <p class="slabel">
              {SOURCE_INFO[k].name}
              <span class="tag neutral mono kind">{SOURCE_INFO[k].tag}</span>
            </p>
            <p class="shelp">
              {#if k === "ble" && app.config.device.last_device && !app.config.device.last_device.id.startsWith("demo:")}
                {app.config.device.last_device.name}
              {:else}
                {SOURCE_INFO[k].help}
              {/if}
            </p>
          </div>
          <div class="sctrl">
            {#if slot}
              <StatusChip status={connectionStatus({
                  ...s,
                  state: slot.state,
                  error: slot.error,
                  data_valid: slot.live && s.data_valid,
                  test_sweep: slot.live && s.test_sweep,
                })} />
            {/if}
            {#if isMain}
              <span class="tag role">main</span>
            {:else if backup?.kind === k}
              <span class="tag neutral role">backup</span>
            {/if}
            {#if !isMain || !running}
              <button
                class="btn small"
                class:primary={!running && isMain}
                aria-label="Use {SOURCE_INFO[k].name}"
                onclick={() => app.selectSource(k)}>Use</button
              >
            {/if}
          </div>
        </li>
      {/each}
    </ul>
  </div>

  <div class="section">
    <h2 class="section-label">{SOURCE_INFO[active].name} settings</h2>
    <div class="hairlist">
      {#if active === "ble"}
        <div class="srow">
          <div class="stext">
            <p class="slabel">
              {app.config.device.last_device && !app.config.device.last_device.id.startsWith("demo:")
                ? app.config.device.last_device.name
                : "No sensor chosen yet"}
            </p>
            <p class="shelp">Scan and pick your strap or watch on the Devices page. KittyTooth remembers it.</p>
          </div>
          <div class="sctrl"><button class="btn" onclick={() => navigate("devices")}>Open Devices</button></div>
        </div>
      {:else if active === "pulsoid"}
        <div class="srow stacked">
          <div class="stext">
            <label class="slabel" for="pulsoid-token">Access token</label>
            <p class="shelp">
              Create a token on Pulsoid (BRO plan or trial), then paste it here. It's kept in the Windows Credential
              Manager, never in your settings file.
            </p>
          </div>
          {#if secret("pulsoid_token")?.present && !editingToken}
            <div class="secret-row">
              <span class="mono masked" aria-label="Saved token">{secret("pulsoid_token")?.masked}</span>
              <span class="spacer"></span>
              <button class="btn small" onclick={() => (editingToken = true)}>Replace</button>
              <button class="btn small quiet" onclick={() => saveSecret("pulsoid_token", null)}>Remove</button>
            </div>
          {:else}
            <div class="secret-row">
              <input
                id="pulsoid-token"
                class="mono grow"
                type="password"
                autocomplete="off"
                spellcheck="false"
                placeholder="paste your token"
                bind:value={tokenDraft}
                onkeydown={(e) => e.key === "Enter" && tokenDraft.trim() && saveSecret("pulsoid_token", tokenDraft)}
              />
              <button class="btn primary small" disabled={!tokenDraft.trim()} onclick={() => saveSecret("pulsoid_token", tokenDraft)}
                >Save token</button
              >
            </div>
          {/if}
          <div>
            <button class="btn small" onclick={() => app.run((a) => a.openLink("pulsoid_token"))}>Get token</button>
          </div>
        </div>
      {:else if active === "hyperate"}
        <div class="srow">
          <div class="stext">
            <label class="slabel" for="hr-device">Device ID</label>
            <p class="shelp">The ID the HypeRate app shows, letters and digits only.</p>
          </div>
          <div class="sctrl">
            <input
              id="hr-device"
              class="mono"
              type="text"
              maxlength="32"
              spellcheck="false"
              autocomplete="off"
              placeholder="e.g. 1A2B3C"
              value={cfg.hyperate.device_id}
              oninput={(e) => app.updateConfig((c) => (c.source.hyperate.device_id = e.currentTarget.value.trim()))}
            />
          </div>
        </div>
        <div class="srow stacked">
          <div class="stext">
            <label class="slabel" for="hr-key">API key override</label>
            <p class="shelp">
              {app.info?.hyperate_builtin_key
                ? "This build includes a HypeRate key, so you can leave this empty."
                : "This build has no HypeRate key. Paste a developer key from hyperate.io/api."}
              Kept in the Windows Credential Manager.
            </p>
          </div>
          {#if secret("hyperate_key")?.present && !editingKey}
            <div class="secret-row">
              <span class="mono masked" aria-label="Saved API key">{secret("hyperate_key")?.masked}</span>
              <span class="spacer"></span>
              <button class="btn small" onclick={() => (editingKey = true)}>Replace</button>
              <button class="btn small quiet" onclick={() => saveSecret("hyperate_key", null)}>Remove</button>
            </div>
          {:else}
            <div class="secret-row">
              <input
                id="hr-key"
                class="mono grow"
                type="password"
                autocomplete="off"
                spellcheck="false"
                placeholder="optional"
                bind:value={keyDraft}
                onkeydown={(e) => e.key === "Enter" && keyDraft.trim() && saveSecret("hyperate_key", keyDraft)}
              />
              <button class="btn small" disabled={!keyDraft.trim()} onclick={() => saveSecret("hyperate_key", keyDraft)}
                >Save key</button
              >
            </div>
          {/if}
        </div>
      {:else if active === "ant"}
        <div class="srow">
          <div class="stext">
            <p class="slabel">
              {primary?.kind === "ant" && s.device?.id.startsWith("ant:") ? s.device.name : "Any ANT+ heart rate monitor"}
            </p>
            <p class="shelp">
              Plug in a USB ANT+ stick and KittyTooth pairs with the first heart rate monitor it hears. On a Garmin watch,
              turn on Broadcast Heart Rate. ANT+ carries beat timing, so HRV works even where Bluetooth gives none.
            </p>
          </div>
        </div>
        <div class="srow">
          <div class="stext">
            <p class="slabel">Stick not found or busy?</p>
            <p class="shelp">
              Only one app can use the stick at a time, so close Garmin Express or Zwift. On Windows the stick needs the
              WinUSB driver; Zadig can install it.
            </p>
          </div>
        </div>
      {:else if active === "osc"}
        <div class="srow">
          <div class="stext">
            <label class="slabel" for="oscin-port">Listen port</label>
            <p class="shelp">The UDP port the other app sends to.</p>
          </div>
          <div class="sctrl">
            <input
              id="oscin-port"
              type="number"
              min="1024"
              max="65535"
              value={cfg.osc_input.port}
              aria-invalid={oscConflict}
              onchange={(e) => app.updateConfig((c) => (c.source.osc_input.port = Number(e.currentTarget.value) || 9010))}
            />
          </div>
        </div>
        {#if oscConflict}
          <p class="field-error" role="alert">
            Port {cfg.osc_input.port} is also an OSC output target, so KittyTooth would hear itself. Pick another port.
          </p>
        {/if}
        <div class="srow">
          <div class="stext">
            <label class="slabel" for="oscin-addr">Address</label>
            <p class="shelp">Where the heart rate arrives, as an int or float. A bare name gets /avatar/parameters/.</p>
          </div>
          <div class="sctrl">
            <input
              id="oscin-addr"
              class="mono wide"
              type="text"
              spellcheck="false"
              value={cfg.osc_input.address}
              onchange={(e) => app.updateConfig((c) => (c.source.osc_input.address = e.currentTarget.value.trim()))}
            />
          </div>
        </div>
        <Toggle
          label="Accept from other devices"
          hint="Listen on your network, not just this PC. Leave off unless the sender runs on another machine."
          checked={cfg.osc_input.lan}
          onchange={(v) => app.updateConfig((c) => (c.source.osc_input.lan = v))}
        />
      {:else}
        <div class="srow">
          <div class="stext">
            <p class="slabel">Simulated strap</p>
            <p class="shelp">Rest, a workout ramp, a dropout and a contact loss on a six-minute loop. Good for testing avatars.</p>
          </div>
        </div>
      {/if}
    </div>
  </div>

  <div class="section">
    <h2 class="section-label">Fallback</h2>
    <div class="hairlist">
      <Toggle
        label="Fall back to a backup source"
        hint="If the main source goes quiet, switch to the backup and come back once the main one recovers."
        checked={cfg.fallback.enabled}
        onchange={(v) => app.updateConfig((c) => (c.source.fallback.enabled = v))}
      />
      {#if cfg.fallback.enabled}
        <div class="srow">
          <div class="stext">
            <span class="slabel">Backup source</span>
            <p class="shelp">Connected alongside the main source so it can take over instantly.</p>
          </div>
          <div class="sctrl">
            <Choice
              label="Backup source"
              value={cfg.fallback.backup === active ? backupOptions[0].value : cfg.fallback.backup}
              options={backupOptions}
              onchange={(v: SourceKind) => app.updateConfig((c) => (c.source.fallback.backup = v))}
            />
          </div>
        </div>
        <div class="srow">
          <div class="stext">
            <label class="slabel" for="fb-after">Switch after</label>
            <p class="shelp">Seconds without a reading from the main source.</p>
          </div>
          <div class="sctrl">
            <input
              id="fb-after"
              type="number"
              min="2"
              max="120"
              value={cfg.fallback.after_secs}
              onchange={(e) =>
                app.updateConfig((c) => (c.source.fallback.after_secs = Math.min(120, Math.max(2, Number(e.currentTarget.value) || 5))))}
            />
          </div>
        </div>
        {#if backup?.error}
          <ErrorBanner error={backup.error} />
        {/if}
      {/if}
    </div>
  </div>
</section>

<style>
  .page-head .btn {
    align-self: center;
  }
  .src {
    gap: var(--s4);
  }
  .src .slabel {
    display: flex;
    align-items: center;
    gap: var(--s2);
  }
  .kind {
    height: 20px;
    font-size: 10.5px;
  }
  .role {
    height: 20px;
    font-size: 11px;
  }
  .dot {
    flex: none;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--slate-3);
  }
  .dot.live {
    background: var(--pink);
    box-shadow: 0 0 8px var(--pink);
  }
  .dot.ok {
    background: var(--green);
  }
  .dot.warn {
    background: var(--amber);
  }
  .dot.err {
    background: var(--red);
  }
  .secret-row {
    display: flex;
    align-items: center;
    gap: var(--s2);
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  .masked {
    color: var(--text);
    letter-spacing: 0.04em;
  }
  input.wide {
    width: 240px;
  }
  .field-error {
    padding: var(--s2) var(--s4);
    background: var(--slate-1);
    color: var(--red);
    font-size: 12.5px;
  }
</style>

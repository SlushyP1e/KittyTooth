<script lang="ts">
  import { onMount } from "svelte";
  import { app } from "../stores/app.svelte";
  import RssiBars from "../components/RssiBars.svelte";
  import Toggle from "../components/Toggle.svelte";
  import StatusChip from "../components/StatusChip.svelte";
  import ErrorBanner from "../components/ErrorBanner.svelte";
  import { connectionStatus, rssiLabel, timeAgo } from "../format";
  import type { DiscoveredDevice, UserError } from "../types";

  const s = $derived(app.snapshot);
  const remembered = $derived(app.config.device.last_device);
  const busyWith = $derived(s.state !== "idle" ? s.target?.id : undefined);
  let now = $state(Date.now());

  const adapterError = $derived<UserError | null>(
    s.adapter === "off"
      ? {
          code: "radio_off",
          title: "Bluetooth is turned off",
          message: "Sensors can't be found while the Bluetooth radio is off.",
          fix: "Turn Bluetooth on in Windows Settings > Bluetooth & devices. The list fills in automatically.",
        }
      : s.adapter === "missing"
        ? {
            code: "adapter_missing",
            title: "No Bluetooth adapter",
            message: "KittyTooth could not find a Bluetooth LE adapter on this PC.",
            fix: "Plug in a Bluetooth 4.0+ dongle or enable Bluetooth in Device Manager. The demo sensor still works.",
          }
        : null,
  );
  // Show scan errors that aren't already covered by the adapter banner.
  const scanError = $derived(
    s.scan_error && (!adapterError || s.scan_error.code !== adapterError.code) ? s.scan_error : null,
  );

  onMount(() => {
    void app.run((a) => a.startScan());
    const t = setInterval(() => (now = Date.now()), 1000);
    return () => {
      clearInterval(t);
      void app.run((a) => a.stopScan());
    };
  });

  function connect(d: DiscoveredDevice) {
    void app.run((a) => a.connect(d.id, d.name));
  }
</script>

<section class="devices" aria-labelledby="dev-title">
  <header class="row">
    <h1 id="dev-title">Devices</h1>
    <span class="spacer"></span>
    {#if s.scanning}
      <span class="scanning muted" role="status"><span class="spinner" aria-hidden="true"></span>Scanning</span>
    {/if}
    <button class="btn" onclick={() => app.run((a) => (s.scanning ? a.stopScan() : a.startScan()))}>
      {s.scanning ? "Stop scan" : "Scan"}
    </button>
  </header>

  {#if adapterError}
    <ErrorBanner error={adapterError} />
  {:else if scanError}
    <ErrorBanner error={scanError} />
  {/if}

  <div class="card stack">
    <h2>Your sensor</h2>
    {#if remembered}
      <div class="row remembered">
        <div class="stack tight">
          <p class="name">{remembered.name}</p>
          <p class="muted mono">{remembered.id}</p>
        </div>
        <span class="spacer"></span>
        {#if s.target?.id === remembered.id && s.state !== "idle"}
          <StatusChip status={connectionStatus(s)} />
          <button class="btn" onclick={() => app.run((a) => a.disconnect())}>Disconnect</button>
        {:else}
          <button class="btn primary" onclick={() => app.run((a) => a.connect(remembered.id, remembered.name))}>Connect</button>
        {/if}
        <button class="btn ghost danger" onclick={() => app.run((a) => a.forget())}>Forget</button>
      </div>
      <Toggle
        label="Connect automatically on launch"
        hint="KittyTooth reconnects by itself whenever the sensor drops out, until you press Disconnect."
        checked={app.config.device.auto_connect}
        onchange={(v) => app.updateConfig((c) => (c.device.auto_connect = v))}
      />
    {:else}
      <p class="muted">No sensor remembered yet. Pick one below; KittyTooth will remember it and reconnect automatically.</p>
    {/if}
  </div>

  <div class="card stack">
    <div class="row">
      <h2>Nearby</h2>
      <span class="spacer"></span>
      <Toggle
        label="Show all devices"
        checked={app.config.device.show_all_devices}
        onchange={(v) => app.updateConfig((c) => (c.device.show_all_devices = v))}
      />
    </div>
    {#if app.devices.length === 0}
      <div class="empty muted">
        {#if s.scanning}
          Looking for heart rate sensors... Wet your strap's electrodes and put it on, or turn on "HR broadcast" on your watch.
        {:else}
          Press Scan to look for sensors.
        {/if}
      </div>
    {:else}
      <ul class="list" aria-label="Nearby devices">
        {#each app.devices as d (d.id)}
          <li class="item" class:dim={!d.has_hr_service}>
            <span class="kind" aria-hidden="true">{d.has_hr_service ? "♥" : "•"}</span>
            <div class="info">
              <p class="name">
                {d.name}
                {#if d.demo}<span class="tag">demo</span>{/if}
                {#if !d.has_hr_service}<span class="tag plain">no heart rate</span>{/if}
              </p>
              <p class="muted small tabular">
                {rssiLabel(d.rssi)}{d.rssi != null ? ` · ${d.rssi} dBm` : ""} · seen {timeAgo(d.last_seen_ms, now)}
              </p>
            </div>
            <RssiBars rssi={d.rssi} />
            {#if busyWith === d.id}
              <button class="btn small" onclick={() => app.run((a) => a.disconnect())}>Disconnect</button>
            {:else}
              <button
                class="btn small"
                class:primary={d.has_hr_service}
                disabled={!d.has_hr_service}
                aria-label="Connect to {d.name}"
                onclick={() => connect(d)}>Connect</button
              >
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
  </div>

  <div class="card stack">
    <Toggle
      label="Show demo sensor"
      hint="A simulated strap for trying KittyTooth and testing avatars without hardware."
      checked={app.config.device.demo_device}
      onchange={(v) => app.updateConfig((c) => (c.device.demo_device = v))}
    />
  </div>
</section>

<style>
  .devices {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .scanning {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
  }
  .spinner {
    width: 14px;
    height: 14px;
    border-radius: 50%;
    border: 2px solid var(--surface-3);
    border-top-color: var(--accent);
    animation: spin 0.9s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .tight {
    gap: 2px;
  }
  .name {
    font-weight: 600;
  }
  .remembered {
    flex-wrap: wrap;
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .item {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 10px 12px;
    border-radius: var(--radius-sm);
    background: var(--surface-2);
    border: 1px solid transparent;
  }
  .item:hover {
    border-color: var(--border);
  }
  .item.dim {
    opacity: 0.65;
  }
  .kind {
    width: 28px;
    height: 28px;
    display: grid;
    place-items: center;
    border-radius: 50%;
    background: var(--accent-soft);
    color: var(--accent);
    font-size: 14px;
  }
  .dim .kind {
    background: var(--surface-3);
    color: var(--muted);
  }
  .info {
    flex: 1;
    min-width: 0;
  }
  .small {
    font-size: 12.5px;
  }
  .tag {
    margin-left: 6px;
    font-size: 11px;
    font-weight: 650;
    padding: 1px 6px;
    border-radius: 6px;
    background: var(--accent-soft);
    color: var(--accent);
  }
  .tag.plain {
    background: var(--surface-3);
    color: var(--muted);
  }
  .empty {
    padding: 24px 8px;
    text-align: center;
  }
</style>

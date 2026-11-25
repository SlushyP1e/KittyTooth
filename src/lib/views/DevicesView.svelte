<script lang="ts">
  import { onMount } from "svelte";
  import { app } from "../stores/app.svelte";
  import RssiBars from "../components/RssiBars.svelte";
  import Toggle from "../components/Toggle.svelte";
  import StatusChip from "../components/StatusChip.svelte";
  import ErrorBanner from "../components/ErrorBanner.svelte";
  import { connectionStatus, timeAgo } from "../format";
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

<section class="page" aria-labelledby="dev-title">
  <header class="page-head">
    <div class="titles">
      <h1 id="dev-title" class="page-title">Devices</h1>
      <p class="subtitle">
        {s.scanning ? "Scanning for heart rate sensors nearby" : "Scan to find heart rate sensors nearby"}
      </p>
    </div>
    {#if s.scanning}<span class="scanning mono" role="status"><span class="pulse" aria-hidden="true"></span>scanning</span>{/if}
    <button class="btn" onclick={() => app.run((a) => (s.scanning ? a.stopScan() : a.startScan()))}>
      {s.scanning ? "Stop scan" : "Scan"}
    </button>
  </header>

  {#if adapterError}
    <ErrorBanner error={adapterError} />
  {:else if scanError}
    <ErrorBanner error={scanError} />
  {/if}

  <div class="section">
    <h2 class="section-label">Your sensor</h2>
    <div class="hairlist">
      {#if remembered}
        <div class="srow">
          <div class="stext">
            <p class="slabel">{remembered.name}</p>
            <p class="shelp mono">{remembered.id}</p>
          </div>
          <div class="sctrl">
            {#if s.target?.id === remembered.id && s.state !== "idle"}
              <StatusChip status={connectionStatus(s)} />
              <button class="btn" onclick={() => app.run((a) => a.disconnect())}>Disconnect</button>
            {:else}
              <button class="btn primary" onclick={() => app.run((a) => a.connect(remembered.id, remembered.name))}
                >Connect</button
              >
            {/if}
            <button class="btn quiet" onclick={() => app.run((a) => a.forget())}>Forget</button>
          </div>
        </div>
        <Toggle
          label="Connect automatically on launch"
          hint="Reconnects on its own whenever the sensor drops out, until you press Disconnect."
          checked={app.config.device.auto_connect}
          onchange={(v) => app.updateConfig((c) => (c.device.auto_connect = v))}
        />
      {:else}
        <div class="srow">
          <div class="stext">
            <p class="slabel">No sensor remembered yet</p>
            <p class="shelp">Pick one below. KittyTooth remembers it and reconnects automatically.</p>
          </div>
        </div>
      {/if}
    </div>
  </div>

  <div class="section">
    <h2 class="section-label">Nearby <span class="spacer"></span><span>{app.devices.length} found</span></h2>
    {#if app.devices.length === 0}
      <div class="raised">
        <p class="shelp">
          {#if s.scanning}
            Looking for sensors. Wet your strap's electrodes and put it on, or turn on heart rate broadcast on your watch.
          {:else}
            Press Scan to look for sensors.
          {/if}
        </p>
      </div>
    {:else}
      <ul class="hairlist" aria-label="Nearby devices">
        {#each app.devices as d (d.id)}
          <li class="srow dev" class:dim={!d.has_hr_service}>
            <div class="stext">
              <p class="slabel">
                {d.name}
                {#if d.demo}<span class="tag neutral">demo</span>{/if}
                {#if !d.has_hr_service}<span class="tag neutral">no heart rate</span>{/if}
              </p>
              <p class="shelp mono">
                {d.rssi != null ? `${d.rssi} dBm` : "signal unknown"} · seen {timeAgo(d.last_seen_ms, now)}
              </p>
            </div>
            <div class="sctrl">
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
            </div>
          </li>
        {/each}
      </ul>
    {/if}
  </div>

  <div class="section">
    <h2 class="section-label">Options</h2>
    <div class="hairlist">
      <Toggle
        label="Show all devices"
        hint="Include Bluetooth devices that don't advertise a heart rate service."
        checked={app.config.device.show_all_devices}
        onchange={(v) => app.updateConfig((c) => (c.device.show_all_devices = v))}
      />
      <Toggle
        label="Show demo sensor"
        hint="A simulated strap for trying KittyTooth and testing avatars without hardware."
        checked={app.config.device.demo_device}
        onchange={(v) => app.updateConfig((c) => (c.device.demo_device = v))}
      />
    </div>
  </div>
</section>

<style>
  .scanning {
    display: inline-flex;
    align-items: center;
    gap: var(--s2);
    font-size: 12px;
    color: var(--muted);
    align-self: center;
  }
  .pulse {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--pink);
    animation: blink 1.2s ease-in-out infinite;
  }
  @keyframes blink {
    50% {
      opacity: 0.25;
    }
  }
  .page-head .btn {
    align-self: center;
  }
  .slabel {
    display: flex;
    align-items: center;
    gap: var(--s2);
    flex-wrap: wrap;
  }
  .dev {
    min-height: 58px;
    transition: background 120ms;
  }
  .dev:hover {
    background: var(--slate-2);
  }
  .dev.dim .slabel {
    color: var(--muted);
  }
</style>

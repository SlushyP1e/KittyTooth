<script lang="ts">
  import { app } from "../stores/app.svelte";
  import Heart from "../components/Heart.svelte";
  import HrGraph from "../components/HrGraph.svelte";
  import StatusChip from "../components/StatusChip.svelte";
  import Battery from "../components/Battery.svelte";
  import ZoneBand from "../components/ZoneBand.svelte";
  import StatTile from "../components/StatTile.svelte";
  import Segmented from "../components/Segmented.svelte";
  import ErrorBanner from "../components/ErrorBanner.svelte";
  import { connectionStatus, contactLabel, formatBpm, formatMs, locationLabel, zoneColor } from "../format";
  import type { GraphRange } from "../types";

  let { navigate }: { navigate: (v: string) => void } = $props();

  const s = $derived(app.snapshot);
  const live = $derived(s.data_valid || s.test_sweep);
  const status = $derived(connectionStatus(s));
  const smoothingOn = $derived(app.config.processing.smoothing !== "off");
  const range = $derived(app.config.ui.graph_range);
  let now = $state(Date.now());
  const retryIn = $derived(s.retry_at_ms ? Math.max(0, Math.ceil((s.retry_at_ms - now) / 1000)) : null);

  $effect(() => {
    if (!s.retry_at_ms) return;
    const t = setInterval(() => (now = Date.now()), 500);
    return () => clearInterval(t);
  });

  function setRange(r: GraphRange) {
    app.updateConfig((c) => (c.ui.graph_range = r));
  }
</script>

<section class="live" aria-labelledby="live-title">
  <h1 id="live-title" class="sr-only">Live heart rate</h1>

  {#if s.error}
    <ErrorBanner error={s.error}>
      {#snippet actions()}
        <button class="btn small" onclick={() => navigate("devices")}>Devices</button>
      {/snippet}
    </ErrorBanner>
  {/if}

  {#if !s.target && s.state === "idle" && !s.test_sweep}
    <div class="card empty">
      <Heart active={false} size={84} label="No sensor connected" />
      <div class="stack">
        <h2>No sensor connected yet</h2>
        <p class="muted">Put on your strap or enable heart rate broadcast on your watch, then pick it from the list.</p>
        <div class="row">
          <button class="btn primary" onclick={() => navigate("devices")}>Find a sensor</button>
          {#if app.config.device.demo_device}
            <button class="btn" onclick={() => app.run((a) => a.connect("demo:strap", "KittyTooth Demo Strap"))}
              >Try the demo strap</button
            >
          {/if}
        </div>
      </div>
    </div>
  {/if}

  <div class="hero card">
    <div class="heart-wrap" style:--zone={zoneColor(s.zone)}>
      <Heart beat={app.beatCount} interval={app.beatInterval} active={live} size={132} label="Heartbeat" />
    </div>
    <div class="reading">
      <div class="bpm-line" aria-live="off">
        <span class="bpm tabular" class:idle={!live} data-testid="bpm">{live ? formatBpm(s.bpm) : "--"}</span>
        <span class="unit">bpm</span>
      </div>
      {#if smoothingOn && live && s.raw_bpm != null}
        <p class="raw muted tabular">raw {s.raw_bpm}</p>
      {/if}
      <span class="sr-only" aria-live="polite">{live ? `${formatBpm(s.bpm)} beats per minute` : "No reading"}</span>
    </div>
    <div class="meta">
      <StatusChip {status} />
      <p class="device">
        {#if s.device}
          {s.device.name}{#if s.device.demo}<span class="tag">demo</span>{/if}
        {:else if s.target}
          {s.target.name}
        {:else}
          <span class="muted">No device</span>
        {/if}
      </p>
      <div class="row small-meta">
        <Battery level={s.battery} />
        {#if locationLabel(s.device?.location ?? null)}
          <span class="muted">{locationLabel(s.device?.location ?? null)}</span>
        {/if}
      </div>
      {#if contactLabel(s.contact) && s.device}
        <p class="contact" class:bad={s.contact === "not_detected"}>{contactLabel(s.contact)}</p>
      {/if}
      {#if s.state === "reconnecting" && retryIn != null}
        <p class="muted tabular">Next try in {retryIn}s (attempt {s.attempt})</p>
      {/if}
    </div>
  </div>

  <div class="card">
    <ZoneBand bpm={s.bpm} maxHr={s.max_hr} thresholds={app.config.zones.thresholds} zone={s.zone} active={live} />
  </div>

  <div class="card graph-card">
    <div class="row graph-head">
      <h2>Heart rate</h2>
      <span class="spacer"></span>
      <Segmented
        label="Graph range"
        value={range}
        options={[
          { value: "60s", label: "60 s" },
          { value: "5m", label: "5 min" },
          { value: "session", label: "Session" },
        ]}
        onchange={setRange}
      />
    </div>
    <div class="graph-area">
      <HrGraph
        history={app.history}
        version={app.historyVersion}
        {range}
        live={s.data_valid}
        beat={app.beatCount}
        maxHr={s.max_hr}
        thresholds={app.config.zones.thresholds}
      />
    </div>
  </div>

  <div class="stats">
    <StatTile label="Session min" value={formatBpm(s.stats.min)} unit="bpm" />
    <StatTile label="Average" value={formatBpm(s.stats.avg)} unit="bpm" />
    <StatTile label="Session max" value={formatBpm(s.stats.max)} unit="bpm" />
    <StatTile
      label="HRV (RMSSD)"
      value={live ? formatMs(s.hrv_ms) : "--"}
      unit="ms"
      hint="Root mean square of successive RR differences over the last 30 seconds. Needs a sensor that sends RR intervals."
    />
  </div>
</section>

<style>
  .live {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .empty {
    display: flex;
    gap: 20px;
    align-items: center;
  }
  .hero {
    display: grid;
    grid-template-columns: auto 1fr auto;
    gap: 24px;
    align-items: center;
    padding: 22px 26px;
  }
  .heart-wrap {
    width: 140px;
    height: 140px;
    display: grid;
    place-items: center;
    border-radius: 50%;
    background: radial-gradient(circle, color-mix(in srgb, var(--accent) 14%, transparent), transparent 70%);
  }
  .bpm-line {
    display: flex;
    align-items: baseline;
    gap: 10px;
  }
  .bpm {
    font-family: var(--font-display);
    font-size: 104px;
    line-height: 0.95;
    font-weight: 700;
    letter-spacing: -0.04em;
    min-width: 3ch;
  }
  .bpm.idle {
    color: var(--faint);
    font-weight: 300;
  }
  .unit {
    font-size: 22px;
    font-weight: 600;
    color: var(--muted);
  }
  .raw {
    margin-top: 4px;
  }
  .meta {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 8px;
    text-align: right;
    max-width: 260px;
  }
  .device {
    font-weight: 600;
  }
  .tag {
    margin-left: 6px;
    font-size: 11px;
    font-weight: 650;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    padding: 2px 6px;
    border-radius: 6px;
    background: var(--accent-soft);
    color: var(--accent);
  }
  .small-meta {
    gap: 12px;
    font-size: 12.5px;
  }
  .contact {
    font-size: 12.5px;
    color: var(--ok);
  }
  .contact.bad {
    color: var(--warn);
  }
  .graph-card {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .graph-area {
    height: 240px;
  }
  .stats {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 12px;
  }
  @media (max-width: 900px) {
    .hero {
      grid-template-columns: auto 1fr;
    }
    .meta {
      grid-column: 1 / -1;
      align-items: flex-start;
      text-align: left;
      max-width: none;
    }
    .stats {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
    .bpm {
      font-size: 84px;
    }
  }
</style>

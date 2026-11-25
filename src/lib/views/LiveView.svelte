<script lang="ts">
  import { app } from "../stores/app.svelte";
  import Heart from "../components/Heart.svelte";
  import HrGraph from "../components/HrGraph.svelte";
  import StatusChip from "../components/StatusChip.svelte";
  import ZoneBand from "../components/ZoneBand.svelte";
  import Choice from "../components/Choice.svelte";
  import ErrorBanner from "../components/ErrorBanner.svelte";
  import { connectionStatus, formatBpm, formatMs, locationLabel } from "../format";
  import type { GraphRange } from "../types";

  let { navigate }: { navigate: (v: string) => void } = $props();

  const s = $derived(app.snapshot);
  const live = $derived(s.data_valid || s.test_sweep);
  const status = $derived(connectionStatus(s));
  const smoothingOn = $derived(app.config.processing.smoothing !== "off");
  const range = $derived(app.config.ui.graph_range);
  let now = $state(Date.now());
  const retryIn = $derived(s.retry_at_ms ? Math.max(0, Math.ceil((s.retry_at_ms - now) / 1000)) : null);
  const firstTarget = $derived(s.targets.find((t) => t.enabled && t.dest));
  const subtitle = $derived(
    s.test_sweep
      ? "Sending a test sweep to your avatar"
      : live && firstTarget
        ? `Streaming to ${firstTarget.name} on ${firstTarget.dest}`
        : s.target
          ? `Waiting for ${s.target.name}`
          : "No sensor connected",
  );
  const location = $derived(locationLabel(s.device?.location ?? null));

  $effect(() => {
    if (!s.retry_at_ms) return;
    const t = setInterval(() => (now = Date.now()), 500);
    return () => clearInterval(t);
  });

  function setRange(r: GraphRange) {
    app.updateConfig((c) => (c.ui.graph_range = r));
  }
</script>

<section class="page" aria-labelledby="live-title">
  <header class="page-head">
    <div class="titles">
      <h1 id="live-title" class="page-title">Live</h1>
      <p class="subtitle">{subtitle}</p>
    </div>
  </header>

  {#if s.error}
    <ErrorBanner error={s.error}>
      {#snippet actions()}
        <button class="btn small" onclick={() => navigate("devices")}>Devices</button>
      {/snippet}
    </ErrorBanner>
  {/if}

  {#if !s.target && s.state === "idle" && !s.test_sweep}
    <div class="raised empty">
      <div class="stext">
        <p class="slabel">No sensor connected yet</p>
        <p class="shelp">Put on your strap or turn on heart rate broadcast on your watch, then pick it from Devices.</p>
      </div>
      <div class="row">
        <button class="btn primary" onclick={() => navigate("devices")}>Find a sensor</button>
        {#if app.config.device.demo_device}
          <button class="btn" onclick={() => app.run((a) => a.connect("demo:strap", "KittyTooth Demo Strap"))}
            >Try the demo strap</button
          >
        {/if}
      </div>
    </div>
  {/if}

  <div class="hero">
    <Heart beat={app.beatCount} active={live} size={92} label="Heartbeat" />
    <div class="reading">
      <span class="bpm tabular" class:idle={!live} data-testid="bpm">{live ? formatBpm(s.bpm) : "--"}</span>
      <span class="unit mono">bpm</span>
      <span class="sr-only" aria-live="polite">{live ? `${formatBpm(s.bpm)} beats per minute` : "No reading"}</span>
    </div>
    <div class="meta">
      <p class="device">
        {#if s.device}{s.device.name}{:else if s.target}{s.target.name}{:else}<span class="muted">No device</span>{/if}
      </p>
      <div class="tags">
        <StatusChip {status} />
        {#if s.device?.demo}<span class="tag neutral">demo</span>{/if}
        {#if s.battery != null}<span class="tag {s.battery <= 15 ? 'err' : 'neutral'}">{s.battery}% battery</span>{/if}
        {#if location && s.device}<span class="tag neutral">{location.toLowerCase()}</span>{/if}
        {#if s.device && s.contact === "detected"}<span class="tag neutral">contact ok</span>{/if}
        {#if s.device && s.contact === "not_detected"}<span class="tag warn">no skin contact</span>{/if}
      </div>
      {#if smoothingOn && live && s.raw_bpm != null}
        <p class="mono small muted">raw {s.raw_bpm} bpm</p>
      {/if}
      {#if s.state === "reconnecting" && retryIn != null}
        <p class="mono small muted">next try in {retryIn}s · attempt {s.attempt}</p>
      {/if}
    </div>
  </div>

  <ZoneBand bpm={s.bpm} maxHr={s.max_hr} thresholds={app.config.zones.thresholds} zone={s.zone} active={live} />

  <div class="section">
    <div class="section-label">
      <span>Heart rate</span>
      <span class="spacer"></span>
      <Choice
        variant="tabs"
        label="Graph range"
        value={range}
        options={[
          { value: "60s", label: "60 s" },
          { value: "5m", label: "5 min" },
          { value: "session", label: "session" },
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

  <dl class="stats">
    <div>
      <dt class="mono">session min</dt>
      <dd><span class="val tabular">{formatBpm(s.stats.min)}</span><span class="mono u">bpm</span></dd>
    </div>
    <div>
      <dt class="mono">average</dt>
      <dd><span class="val tabular">{formatBpm(s.stats.avg)}</span><span class="mono u">bpm</span></dd>
    </div>
    <div>
      <dt class="mono">session max</dt>
      <dd><span class="val tabular">{formatBpm(s.stats.max)}</span><span class="mono u">bpm</span></dd>
    </div>
    <div title="RMSSD of RR intervals over the last 30 seconds. Needs a sensor that sends RR intervals.">
      <dt class="mono">hrv (rmssd)</dt>
      <dd><span class="val tabular">{live ? formatMs(s.hrv_ms) : "--"}</span><span class="mono u">ms</span></dd>
    </div>
  </dl>
</section>

<style>
  .empty {
    display: flex;
    align-items: center;
    gap: var(--s5);
    flex-wrap: wrap;
  }
  .empty .stext {
    flex: 1;
    min-width: 240px;
  }
  .hero {
    display: grid;
    grid-template-columns: auto 1fr auto;
    align-items: center;
    gap: var(--s6);
    padding: var(--s3) 0;
  }
  .reading {
    display: flex;
    align-items: baseline;
    gap: var(--s2);
  }
  .bpm {
    font-family: var(--display);
    font-weight: 800;
    font-size: 120px;
    line-height: 0.9;
    letter-spacing: -0.04em;
    color: var(--text);
    min-width: 2.5ch;
    text-align: right;
  }
  .bpm.idle {
    color: var(--slate-3);
  }
  .unit {
    font-size: 13px;
    color: var(--muted);
  }
  .meta {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: var(--s2);
    max-width: 300px;
  }
  .device {
    font-weight: 600;
    color: var(--text);
    text-align: right;
  }
  .tags {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: 6px;
  }
  .small {
    font-size: 12px;
  }
  .graph-area {
    height: 250px;
  }
  .stats {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    margin: 0;
  }
  .stats > div {
    padding: var(--s1) var(--s5);
    border-left: 1px solid var(--slate-3);
  }
  .stats > div:first-child {
    border-left: 0;
    padding-left: 0;
  }
  dt {
    font-size: 11px;
    color: var(--muted);
    letter-spacing: 0.04em;
  }
  dd {
    margin: var(--s1) 0 0;
    display: flex;
    align-items: baseline;
    gap: 6px;
  }
  .val {
    font-family: var(--display);
    font-weight: 700;
    font-size: 30px;
    color: var(--text);
    letter-spacing: -0.02em;
  }
  .u {
    font-size: 12px;
    color: var(--muted);
  }
  @media (max-width: 980px) {
    .hero {
      grid-template-columns: auto 1fr;
      gap: var(--s5);
    }
    .meta {
      grid-column: 1 / -1;
      align-items: flex-start;
      max-width: none;
    }
    .device {
      text-align: left;
    }
    .tags {
      justify-content: flex-start;
    }
    .bpm {
      font-size: 96px;
    }
    .stats {
      grid-template-columns: repeat(2, minmax(0, 1fr));
      row-gap: var(--s5);
    }
    .stats > div:nth-child(3) {
      border-left: 0;
      padding-left: 0;
    }
  }
</style>

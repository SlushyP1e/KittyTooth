<script lang="ts">
  import { onMount } from "svelte";
  import { app } from "../stores/app.svelte";
  import Toggle from "../components/Toggle.svelte";
  import Choice from "../components/Choice.svelte";
  import SessionGraph, { type Marker } from "../components/SessionGraph.svelte";
  import { stagger } from "../motion";
  import { formatBpm, SOURCE_INFO, zoneName } from "../format";
  import { formatBytes, formatDay, formatSpan, formatTime, RETENTION_LABELS, sourceLabel } from "../sessions";
  import type { Retention, Session, SessionSummary } from "../types";

  // Replaced, never edited in place. A session can hold tens of thousands of samples.
  let list = $state.raw<SessionSummary[] | null>(null);
  let selected = $state.raw<Session | null>(null);
  let loadingId = $state<string | null>(null);
  let confirmDelete = $state(false);
  let dir = $state(1);
  const h = $derived(app.config.history);
  const totalBytes = $derived((list ?? []).reduce((a, s) => a + s.bytes, 0));

  async function refresh() {
    const r = await app.run((a) => a.listSessions());
    if (r) list = r;
  }

  async function open(id: string) {
    loadingId = id;
    const s = await app.run((a) => a.getSession(id));
    loadingId = null;
    if (s) {
      dir = 1;
      confirmDelete = false;
      selected = s;
      document.getElementById("main-content")?.scrollTo({ top: 0 });
    }
  }

  function back() {
    dir = -1;
    selected = null;
    confirmDelete = false;
  }

  async function exportCsv() {
    if (!selected) return;
    const p = await app.run((a) => a.exportSession(selected!.summary.id));
    if (p) app.toast({ kind: "success", title: "Session exported", message: p });
  }

  async function remove() {
    if (!selected) return;
    const r = await app.run((a) => a.deleteSession(selected!.summary.id));
    if (r) {
      list = r;
      back();
      app.toast({ kind: "success", title: "Session deleted" });
    }
  }

  onMount(() => {
    void refresh();
    let off: (() => void) | undefined;
    void app.api.on("sessions", () => void refresh()).then((u) => (off = u));
    // Keep the duration of a session that is still recording roughly current.
    const t = setInterval(() => list?.some((s) => s.active) && void refresh(), 15_000);
    return () => {
      off?.();
      clearInterval(t);
    };
  });

  // ---------- detail ----------
  const sum = $derived(selected?.summary);
  const hrT = $derived(selected ? selected.samples.map((p) => p.t_ms) : []);
  const hrV = $derived(selected ? selected.samples.map((p) => p.bpm) : []);
  const hrvT = $derived(selected ? selected.hrv.map((p) => p.t_ms) : []);
  const hrvV = $derived(selected ? selected.hrv.map((p) => p.rmssd_ms) : []);
  const markers = $derived<Marker[]>(
    selected
      ? selected.alerts.map((a) => ({
          t: a.t_ms,
          tone: a.kind,
          label: `${a.kind === "high" ? "high" : "low"} alert ${a.bpm} bpm`,
        }))
      : [],
  );
  const zoneTotal = $derived(selected ? selected.zone_secs.reduce((a, b) => a + b, 0) : 0);
  const hrvAvg = $derived(hrvV.length ? hrvV.reduce((a, b) => a + b, 0) / hrvV.length : null);
</script>

{#key selected?.summary.id ?? "list"}
  <div use:stagger={{ dir }}>
    {#if selected && sum}
      <section class="page" aria-labelledby="hist-title">
        <header class="page-head">
          <div class="titles">
            <button class="btn quiet small back" onclick={back} aria-label="Back to all sessions">
              <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M15 6l-6 6 6 6" /></svg>
              All sessions
            </button>
            <h1 id="hist-title" class="page-title">{formatDay(sum.start_ms)}, {formatTime(sum.start_ms)}</h1>
            <p class="subtitle">
              {formatTime(sum.start_ms)} to {formatTime(sum.end_ms)} · {sourceLabel(sum)}
              <span class="tag neutral mono src">{SOURCE_INFO[sum.source].tag}</span>
              {#if sum.active}<span class="tag mono src">recording</span>{/if}
            </p>
          </div>
          <div class="row">
            <button class="btn" onclick={exportCsv}>Export CSV</button>
            {#if confirmDelete}
              <button class="btn danger" onclick={remove}>Delete for good</button>
              <button class="btn quiet" onclick={() => (confirmDelete = false)}>Keep</button>
            {:else}
              <button
                class="btn danger"
                disabled={sum.active}
                title={sum.active ? "Still recording. Reset the session on the Live page first." : undefined}
                onclick={() => (confirmDelete = true)}>Delete</button
              >
            {/if}
          </div>
        </header>

        <dl class="stats">
          <div>
            <dt class="mono">duration</dt>
            <dd><span class="val tabular">{formatSpan(sum.end_ms - sum.start_ms)}</span></dd>
          </div>
          <div>
            <dt class="mono">min</dt>
            <dd><span class="val tabular">{formatBpm(sum.min_bpm)}</span><span class="mono u">bpm</span></dd>
          </div>
          <div>
            <dt class="mono">average</dt>
            <dd><span class="val tabular">{formatBpm(sum.avg_bpm)}</span><span class="mono u">bpm</span></dd>
          </div>
          <div>
            <dt class="mono">max</dt>
            <dd><span class="val tabular">{formatBpm(sum.max_bpm)}</span><span class="mono u">bpm</span></dd>
          </div>
        </dl>

        <div class="section">
          <h2 class="section-label">
            <span>Heart rate</span>
            <span class="spacer"></span>
            {#if markers.length}
              <span class="legend"><i class="flag high"></i>high alert</span>
              {#if markers.some((m) => m.tone === "low")}<span class="legend"><i class="flag low"></i>low alert</span>{/if}
            {/if}
          </h2>
          <div class="graph-area">
            <SessionGraph t={hrT} v={hrV} {markers} />
          </div>
        </div>

        <div class="section">
          <h2 class="section-label">Time in zones <span class="spacer"></span><span>max hr {selected.max_hr}</span></h2>
          <ul class="hairlist zones">
            {#each [5, 4, 3, 2, 1, 0] as z (z)}
              {@const secs = selected.zone_secs[z]}
              {@const pct = zoneTotal ? (secs / zoneTotal) * 100 : 0}
              <li class="zrow" style:--zc="var(--zone-{z})">
                <span class="zname"><i class="zdot"></i>{zoneName(z)}</span>
                <span class="mono zrange"
                  >{z === 0
                    ? `< ${selected.zones.thresholds[0]}%`
                    : z === 5
                      ? `${selected.zones.thresholds[4]}%+`
                      : `${selected.zones.thresholds[z - 1]}-${selected.zones.thresholds[z]}%`}</span
                >
                <span class="zbar" aria-hidden="true"><span style:width="{pct}%"></span></span>
                <span class="mono ztime">{secs ? formatSpan(secs * 1000) : "-"}</span>
                <span class="mono zpct">{Math.round(pct)}%</span>
              </li>
            {/each}
          </ul>
        </div>

        <div class="section">
          <h2 class="section-label">
            <span>Heart rate variability (rmssd)</span>
            <span class="spacer"></span>
            {#if hrvAvg != null}<span>average {Math.round(hrvAvg)} ms</span>{/if}
          </h2>
          {#if selected.hrv.length}
            <div class="graph-area small">
              <SessionGraph t={hrvT} v={hrvV} unit="ms" label="HRV" color="--zone-1" minSpan={30} gapMs={60_000} />
            </div>
            <p class="hint">RMSSD of beat intervals over the minute before each point. Higher is generally more relaxed.</p>
          {:else}
            <div class="raised">
              <p class="shelp">
                {sum.has_rr
                  ? "Not enough clean beat intervals to work out HRV for this session."
                  : "This source didn't send beat intervals, so HRV isn't available for this session."}
              </p>
            </div>
          {/if}
        </div>

        {#if selected.alerts.length}
          <div class="section">
            <h2 class="section-label">Alerts</h2>
            <ul class="hairlist">
              {#each selected.alerts as a, i (i)}
                <li class="srow alert-row">
                  <span class="tag {a.kind === 'high' ? 'err' : 'warn'}">{a.kind}</span>
                  <span class="slabel">{a.bpm} bpm</span>
                  <span class="spacer"></span>
                  <span class="mono muted small"
                    >{a.kind === "high" ? "above" : "below"} {a.threshold} bpm · {formatTime(a.t_ms)}</span
                  >
                </li>
              {/each}
            </ul>
          </div>
        {/if}

        <p class="privacy">
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="5" y="11" width="14" height="9" rx="2" /><path d="M8 11V8a4 4 0 0 1 8 0v3" /></svg>
          Saved only on this computer. Export CSV writes a copy where you choose, for example to show your doctor.
        </p>
      </section>
    {:else}
      <section class="page" aria-labelledby="hist-title">
        <header class="page-head">
          <div class="titles">
            <h1 id="hist-title" class="page-title">History</h1>
            <p class="subtitle">
              {#if list == null}Loading{:else if !h.enabled}Saving sessions is off{:else}{list.length}
                {list.length === 1 ? "session" : "sessions"} saved on this computer{/if}
            </p>
          </div>
        </header>

        <div class="section">
          <h2 class="section-label">Sessions</h2>
          {#if list && list.length}
            <ul class="hairlist">
              {#each list as s (s.id)}
                <li>
                  <button class="session" onclick={() => open(s.id)} aria-busy={loadingId === s.id}>
                    <span class="stext">
                      <span class="slabel">
                        {formatDay(s.start_ms)}
                        <span class="when">{formatTime(s.start_ms)}</span>
                        {#if s.active}<span class="tag mono rec">recording</span>{/if}
                      </span>
                      <span class="shelp">{formatSpan(s.end_ms - s.start_ms)} · {sourceLabel(s)}</span>
                    </span>
                    <span class="nums mono">
                      <span><em>{s.min_bpm}</em> min</span>
                      <span><em>{s.avg_bpm}</em> avg</span>
                      <span><em>{s.max_bpm}</em> max</span>
                    </span>
                    {#if s.alerts}
                      <span class="tag err alerts">{s.alerts} {s.alerts === 1 ? "alert" : "alerts"}</span>
                    {:else}
                      <span class="alerts"></span>
                    {/if}
                    <svg class="chev" viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M9 6l6 6-6 6" /></svg>
                  </button>
                </li>
              {/each}
            </ul>
          {:else if list}
            <div class="raised">
              <p class="slabel">No saved sessions yet</p>
              <p class="shelp">
                {h.enabled
                  ? "A session is saved automatically while heart rate data comes in, and ends after a few quiet minutes or when you reset the session."
                  : "Turn on Save sessions below to keep a record of your heart rate."}
              </p>
            </div>
          {/if}
        </div>

        <div class="section">
          <h2 class="section-label">Storage <span class="spacer"></span><span>{formatBytes(totalBytes)} used</span></h2>
          <div class="hairlist">
            <Toggle
              label="Save sessions"
              hint="Keeps about one reading per second, beat intervals when the sensor sends them, and any alerts."
              checked={h.enabled}
              onchange={(v) => app.updateConfig((c) => (c.history.enabled = v))}
            />
            <div class="srow">
              <div class="stext">
                <span class="slabel">Keep sessions for</span>
                <p class="shelp">Older sessions are deleted automatically.</p>
              </div>
              <div class="sctrl">
                <Choice
                  label="Keep sessions for"
                  value={h.retention}
                  options={(Object.keys(RETENTION_LABELS) as Retention[]).map((r) => ({ value: r, label: RETENTION_LABELS[r] }))}
                  onchange={(v: Retention) => app.updateConfig((c) => (c.history.retention = v))}
                />
              </div>
            </div>
            <div class="srow">
              <div class="stext">
                <label class="slabel" for="hist-cap">Space limit</label>
                <p class="shelp">When sessions take more than this, the oldest are deleted first. An hour takes about 70 KB.</p>
              </div>
              <div class="sctrl row">
                <input
                  id="hist-cap"
                  type="number"
                  min="10"
                  max="10000"
                  step="10"
                  value={h.max_mb}
                  onchange={(e) =>
                    app.updateConfig(
                      (c) => (c.history.max_mb = Math.min(10_000, Math.max(10, Math.round(Number(e.currentTarget.value) || 200)))),
                    )}
                /><span class="mono muted small">MB</span>
              </div>
            </div>
          </div>
        </div>

        <p class="privacy">
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="5" y="11" width="14" height="9" rx="2" /><path d="M8 11V8a4 4 0 0 1 8 0v3" /></svg>
          <span
            >Your sessions stay on this computer and are never uploaded{#if app.info?.history_dir}, in
              <span class="mono path">{app.info.history_dir}</span>{/if}. Not a medical device.</span
          >
        </p>
      </section>
    {/if}
  </div>
{/key}

<style>
  .back {
    margin: 0 0 var(--s2) calc(var(--s3) * -1);
    gap: 4px;
  }
  .src {
    height: 20px;
    font-size: 10.5px;
    margin-left: var(--s1);
    vertical-align: 1px;
  }
  .session {
    appearance: none;
    border: 0;
    width: 100%;
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto 76px 16px;
    align-items: center;
    gap: var(--s5);
    padding: var(--s3) var(--s4);
    min-height: 60px;
    background: var(--card);
    color: inherit;
    font: inherit;
    text-align: left;
    cursor: pointer;
    transition: background 200ms;
  }
  .session:hover {
    background: color-mix(in srgb, var(--card), var(--text) 3%);
  }
  .session:active .chev {
    translate: 2px 0;
  }
  .session[aria-busy="true"] {
    opacity: 0.6;
  }
  .when {
    margin-left: var(--s1);
    font-weight: 500;
    color: var(--muted);
  }
  .rec {
    height: 18px;
    font-size: 10.5px;
    margin-left: var(--s2);
    vertical-align: 1px;
  }
  .nums {
    display: flex;
    gap: var(--s4);
    font-size: 11.5px;
    color: var(--muted);
  }
  .nums span {
    min-width: 52px;
    text-align: right;
  }
  .nums em {
    font-style: normal;
    font-size: 14px;
    color: var(--text);
  }
  .alerts {
    justify-self: end;
  }
  .chev {
    color: var(--dim);
    transition: translate 300ms var(--spring);
  }
  .graph-area {
    height: 240px;
  }
  .graph-area.small {
    height: 150px;
  }
  .legend {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    text-transform: none;
    letter-spacing: 0;
  }
  .flag {
    width: 0;
    height: 0;
    border-left: 4.5px solid transparent;
    border-right: 4.5px solid transparent;
  }
  .flag.high {
    border-top: 7px solid var(--red);
  }
  .flag.low {
    border-bottom: 7px solid var(--amber);
  }
  .zrow {
    display: grid;
    grid-template-columns: 110px 72px minmax(0, 1fr) 84px 40px;
    align-items: center;
    gap: var(--s4);
    padding: var(--s3) var(--s4);
    font-size: 13px;
  }
  .zname {
    display: inline-flex;
    align-items: center;
    gap: var(--s2);
    color: var(--text);
    font-weight: 500;
  }
  .zdot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--zc);
  }
  .zrange,
  .zpct {
    font-size: 11.5px;
    color: var(--muted);
  }
  .zpct,
  .ztime {
    text-align: right;
  }
  .ztime {
    font-size: 12px;
    color: var(--body);
  }
  .zbar {
    height: 6px;
    border-radius: var(--r-pill);
    background: var(--slate-2);
    overflow: hidden;
  }
  .zbar span {
    display: block;
    height: 100%;
    border-radius: inherit;
    background: var(--zc);
    transition: width 600ms var(--spring);
  }
  .alert-row {
    min-height: 48px;
    gap: var(--s3);
  }
  .small {
    font-size: 12px;
  }
  .privacy {
    display: flex;
    align-items: flex-start;
    gap: var(--s2);
    font-size: 12.5px;
    color: var(--muted);
  }
  .privacy svg {
    flex: none;
    margin-top: 2px;
    color: var(--pink);
  }
  .path {
    font-size: 11.5px;
    word-break: break-all;
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
    font-size: 28px;
    color: var(--text);
    letter-spacing: -0.02em;
  }
  .u {
    font-size: 12px;
    color: var(--muted);
  }
  @media (max-width: 760px) {
    .session {
      grid-template-columns: minmax(0, 1fr) auto 16px;
    }
    .session .alerts {
      display: none;
    }
    .nums span:first-child {
      display: none;
    }
    .zrow {
      grid-template-columns: 96px minmax(0, 1fr) 72px 36px;
    }
    .zrange {
      display: none;
    }
    .stats {
      grid-template-columns: repeat(2, minmax(0, 1fr));
      row-gap: var(--s4);
    }
    .stats > div:nth-child(3) {
      border-left: 0;
      padding-left: 0;
    }
  }
</style>

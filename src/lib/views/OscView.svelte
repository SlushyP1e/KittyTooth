<script lang="ts">
  import { onMount } from "svelte";
  import { app } from "../stores/app.svelte";
  import TargetCard from "../components/TargetCard.svelte";
  import Toggle from "../components/Toggle.svelte";
  import Segmented from "../components/Segmented.svelte";
  import { formatClock, renderChatbox } from "../format";
  import { defaultTarget } from "../defaults";
  import type { DisconnectBehavior, OscLogEntry } from "../types";

  const s = $derived(app.snapshot);
  const cb = $derived(app.config.osc.chatbox);
  let log = $state<OscLogEntry[]>([]);
  let paused = $state(false);
  const TOKENS = ["{hr}", "{heart}", "{min}", "{avg}", "{max}", "{hrv}", "{zone}", "{battery}"];

  onMount(() => {
    let alive = true;
    const poll = async () => {
      if (!alive) return;
      if (!paused && !document.hidden) {
        const entries = await app.run((a) => a.getOscLog());
        if (entries && alive) log = entries.slice().reverse();
      }
      setTimeout(poll, 250);
    };
    void poll();
    return () => {
      alive = false;
    };
  });

  function addTarget() {
    app.updateConfig((c) => {
      const n = c.osc.targets.length + 1;
      const t = defaultTarget(`target-${Date.now().toString(36)}`, `Target ${n}`);
      t.port = 9000 + n;
      t.use_oscquery = false;
      t.chatbox = false;
      c.osc.targets.push(t);
    });
  }

  function insertToken(tok: string) {
    app.updateConfig((c) => (c.osc.chatbox.template = (c.osc.chatbox.template + " " + tok).trim().slice(0, 144)));
  }
</script>

<section class="osc" aria-labelledby="osc-title">
  <header class="row">
    <h1 id="osc-title">OSC output</h1>
    <span class="spacer"></span>
    <button
      class="btn primary"
      onclick={() => app.run((a) => a.testSweep(!s.test_sweep))}
      aria-pressed={s.test_sweep}
      title="Sends 60 → 180 → 60 bpm with beats over 16 seconds so you can check your avatar without wearing the strap"
    >
      {s.test_sweep ? "Stop test" : "Send test sweep"}
    </button>
  </header>

  <div class="card discovery row" role="status">
    <span class="dot" class:on={!!s.vrchat}></span>
    {#if !app.config.osc.oscquery}
      <p>OSCQuery is off. Sending to the manual ports below.</p>
    {:else if s.vrchat}
      <p>
        VRChat found via OSCQuery on <span class="mono">{s.vrchat.ip}:{s.vrchat.port}</span>{s.vrchat.local ? " (this PC)" : ""}.
      </p>
    {:else if s.oscquery.error}
      <p>OSCQuery discovery unavailable ({s.oscquery.error}). Using the manual ports below.</p>
    {:else}
      <p class="muted">Looking for VRChat... Until it's found, KittyTooth sends to the manual ports below.</p>
    {/if}
    <span class="spacer"></span>
    <Toggle
      label="OSCQuery"
      checked={app.config.osc.oscquery}
      onchange={(v) => app.updateConfig((c) => (c.osc.oscquery = v))}
      id="oscq-global"
    />
  </div>

  {#each app.config.osc.targets as t, i (t.id)}
    <TargetCard target={t} index={i} status={s.targets.find((x) => x.id === t.id)} />
  {/each}
  <div><button class="btn" onclick={addTarget}>Add target</button></div>

  <div class="card stack">
    <h2>Sending</h2>
    <div class="grid3">
      <label class="field">
        <span>Keepalive (ms, 0 = off)</span>
        <input
          type="number"
          min="0"
          max="10000"
          step="100"
          value={app.config.osc.keepalive_ms}
          onchange={(e) => app.updateConfig((c) => (c.osc.keepalive_ms = Math.max(0, Number(e.currentTarget.value) || 0)))}
        />
      </label>
      <label class="field">
        <span>Max messages / second / target</span>
        <input
          type="number"
          min="1"
          max="20"
          value={app.config.osc.max_rate}
          onchange={(e) =>
            app.updateConfig((c) => (c.osc.max_rate = Math.min(20, Math.max(1, Number(e.currentTarget.value) || 20))))}
        />
      </label>
      <div class="field">
        <span>When the sensor disconnects</span>
        <Segmented
          label="When the sensor disconnects"
          value={app.config.osc.on_disconnect}
          options={[
            { value: "hold_last", label: "Hold last value" },
            { value: "send_zero", label: "Send 0" },
          ]}
          onchange={(v: DisconnectBehavior) => app.updateConfig((c) => (c.osc.on_disconnect = v))}
        />
      </div>
    </div>
    <p class="hint">Values are sent when they change, plus a keepalive. Connected=false is sent once on disconnect.</p>
  </div>

  <div class="card stack">
    <Toggle
      label="VRChat chatbox"
      hint="Shows your heart rate in the chatbox above your head. Sent at most every 2 seconds."
      checked={cb.enabled}
      onchange={(v) => app.updateConfig((c) => (c.osc.chatbox.enabled = v))}
      id="chatbox-en"
    />
    {#if cb.enabled}
      <label class="field">
        <span>Template</span>
        <input
          type="text"
          maxlength="144"
          value={cb.template}
          oninput={(e) => app.updateConfig((c) => (c.osc.chatbox.template = e.currentTarget.value))}
        />
      </label>
      <div class="row tokens" aria-label="Insert token">
        {#each TOKENS as tok (tok)}
          <button class="btn small" onclick={() => insertToken(tok)}>{tok}</button>
        {/each}
      </div>
      <p class="preview" aria-label="Chatbox preview">{renderChatbox(cb.template, s)}</p>
      <div class="grid3">
        <label class="field">
          <span>Seconds between messages (min 2)</span>
          <input
            type="number"
            min="2"
            max="60"
            step="0.5"
            value={cb.interval_secs}
            onchange={(e) =>
              app.updateConfig((c) => (c.osc.chatbox.interval_secs = Math.max(2, Number(e.currentTarget.value) || 2)))}
          />
        </label>
        <Toggle
          label="Silent (no notification sound)"
          checked={cb.silent}
          onchange={(v) => app.updateConfig((c) => (c.osc.chatbox.silent = v))}
          id="chatbox-silent"
        />
      </div>
    {/if}
  </div>

  <div class="card stack">
    <div class="row">
      <h2>Live log</h2>
      <span class="muted small">last 50 messages</span>
      <span class="spacer"></span>
      <button class="btn small" aria-pressed={paused} onclick={() => (paused = !paused)}>{paused ? "Resume" : "Pause"}</button>
    </div>
    <div class="log" role="log" aria-label="OSC messages sent">
      {#if log.length === 0}
        <p class="muted empty">Nothing sent yet. Connect a sensor or press Send test sweep.</p>
      {:else}
        <table>
          <thead>
            <tr><th>Time</th><th>Target</th><th>Address</th><th>Value</th><th>Type</th></tr>
          </thead>
          <tbody>
            {#each log as e (e.seq)}
              <tr class:test={e.test}>
                <td class="mono tabular">{formatClock(e.t_ms)}</td>
                <td>{e.target}</td>
                <td class="mono addr">{e.address}</td>
                <td class="mono tabular">{e.value}</td>
                <td class="muted">{e.kind}{e.test ? " · test" : ""}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}
    </div>
  </div>
</section>

<style>
  .osc {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .discovery {
    padding: 12px 16px;
  }
  .dot {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: var(--faint);
    flex: none;
  }
  .dot.on {
    background: var(--ok);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--ok) 25%, transparent);
  }
  .grid3 {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 14px;
    align-items: end;
  }
  .tokens {
    flex-wrap: wrap;
    gap: 6px;
  }
  .preview {
    padding: 10px 14px;
    border-radius: 12px;
    background: var(--surface-2);
    font-weight: 560;
    min-height: 40px;
  }
  .log {
    max-height: 320px;
    overflow: auto;
    border-radius: var(--radius-sm);
    border: 1px solid var(--border);
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 12.5px;
  }
  th {
    position: sticky;
    top: 0;
    background: var(--surface-2);
    text-align: left;
    font-weight: 600;
    color: var(--muted);
    padding: 7px 10px;
  }
  td {
    padding: 5px 10px;
    border-top: 1px solid var(--border);
    white-space: nowrap;
  }
  td.addr {
    white-space: normal;
    word-break: break-all;
  }
  tr.test td {
    color: var(--busy);
  }
  .empty {
    padding: 18px;
    text-align: center;
  }
  .small {
    font-size: 12.5px;
  }
  @media (max-width: 900px) {
    .grid3 {
      grid-template-columns: 1fr;
    }
  }
</style>

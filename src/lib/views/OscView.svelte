<script lang="ts">
  import { onMount } from "svelte";
  import { app } from "../stores/app.svelte";
  import TargetCard from "../components/TargetCard.svelte";
  import Toggle from "../components/Toggle.svelte";
  import Choice from "../components/Choice.svelte";
  import { formatClock, renderChatbox } from "../format";
  import { defaultTarget } from "../defaults";
  import type { DisconnectBehavior, OscLogEntry } from "../types";

  const s = $derived(app.snapshot);
  const cb = $derived(app.config.osc.chatbox);
  let log = $state<OscLogEntry[]>([]);
  let paused = $state(false);
  const TOKENS = ["{hr}", "{heart}", "{min}", "{avg}", "{max}", "{hrv}", "{zone}", "{battery}"];

  const subtitle = $derived(
    !app.config.osc.oscquery
      ? "OSCQuery is off, sending to the manual ports below"
      : s.vrchat
        ? `VRChat found via OSCQuery on ${s.vrchat.ip}:${s.vrchat.port}${s.vrchat.local ? " (this PC)" : ""}`
        : s.oscquery.error
          ? "OSCQuery discovery unavailable, using the manual ports below"
          : "Looking for VRChat; until it's found, KittyTooth uses the manual ports",
  );

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

  function shortAddr(a: string) {
    return a.startsWith("/avatar/parameters/") ? a.slice("/avatar/parameters/".length) : a;
  }
</script>

<section class="page" aria-labelledby="osc-title">
  <header class="page-head">
    <div class="titles">
      <h1 id="osc-title" class="page-title">OSC output</h1>
      <p class="subtitle" role="status">{subtitle}</p>
    </div>
    <button
      class="btn primary"
      onclick={() => app.run((a) => a.testSweep(!s.test_sweep))}
      aria-pressed={s.test_sweep}
      title="Sends 60 to 180 to 60 bpm with beats over 16 seconds, so you can check your avatar without the strap"
    >
      {s.test_sweep ? "Stop test" : "Send test sweep"}
    </button>
  </header>

  <div class="section">
    <h2 class="section-label">Discovery</h2>
    <div class="hairlist">
      <Toggle
        id="oscq-global"
        label="OSCQuery"
        hint="Advertise KittyTooth and find VRChat's OSC port automatically."
        checked={app.config.osc.oscquery}
        onchange={(v) => app.updateConfig((c) => (c.osc.oscquery = v))}
      />
    </div>
  </div>

  <div class="section">
    <h2 class="section-label">
      Targets <span class="spacer"></span>
      <button class="btn small" onclick={addTarget}>Add target</button>
    </h2>
    <div class="targets">
      {#each app.config.osc.targets as t, i (t.id)}
        <TargetCard target={t} index={i} status={s.targets.find((x) => x.id === t.id)} />
      {/each}
    </div>
  </div>

  <div class="section">
    <h2 class="section-label">Sending</h2>
    <div class="hairlist">
      <div class="srow">
        <div class="stext">
          <label class="slabel" for="keepalive">Keepalive</label>
          <p class="shelp">Resend unchanged values this often, in ms. 0 turns it off.</p>
        </div>
        <div class="sctrl">
          <input
            id="keepalive"
            type="number"
            min="0"
            max="10000"
            step="100"
            value={app.config.osc.keepalive_ms}
            onchange={(e) => app.updateConfig((c) => (c.osc.keepalive_ms = Math.max(0, Number(e.currentTarget.value) || 0)))}
          />
        </div>
      </div>
      <div class="srow">
        <div class="stext">
          <label class="slabel" for="maxrate">Rate limit</label>
          <p class="shelp">Messages per second per target, never more than 20.</p>
        </div>
        <div class="sctrl">
          <input
            id="maxrate"
            type="number"
            min="1"
            max="20"
            value={app.config.osc.max_rate}
            onchange={(e) =>
              app.updateConfig((c) => (c.osc.max_rate = Math.min(20, Math.max(1, Number(e.currentTarget.value) || 20))))}
          />
        </div>
      </div>
      <div class="srow">
        <div class="stext">
          <span class="slabel">When the sensor disconnects</span>
          <p class="shelp">Connected=false is always sent once.</p>
        </div>
        <div class="sctrl">
          <Choice
            label="When the sensor disconnects"
            value={app.config.osc.on_disconnect}
            options={[
              { value: "hold_last", label: "hold last" },
              { value: "send_zero", label: "send 0" },
            ]}
            onchange={(v: DisconnectBehavior) => app.updateConfig((c) => (c.osc.on_disconnect = v))}
          />
        </div>
      </div>
    </div>
  </div>

  <div class="section">
    <h2 class="section-label">Chatbox</h2>
    <div class="hairlist">
      <Toggle
        id="chatbox-en"
        label="VRChat chatbox"
        hint="Shows your heart rate above your head. Sent at most every 2 seconds."
        checked={cb.enabled}
        onchange={(v) => app.updateConfig((c) => (c.osc.chatbox.enabled = v))}
      />
      {#if cb.enabled}
        <div class="srow stacked">
          <div class="stext">
            <label class="slabel" for="cb-template">Template</label>
            <p class="shelp">Up to 144 characters. Click a token to add it.</p>
          </div>
          <input
            id="cb-template"
            class="mono"
            type="text"
            maxlength="144"
            value={cb.template}
            oninput={(e) => app.updateConfig((c) => (c.osc.chatbox.template = e.currentTarget.value))}
          />
          <div class="chips" aria-label="Insert token">
            {#each TOKENS as tok (tok)}
              <button class="chip" onclick={() => insertToken(tok)}>{tok}</button>
            {/each}
          </div>
          <p class="preview" aria-label="Chatbox preview">{renderChatbox(cb.template, s)}</p>
        </div>
        <div class="srow">
          <div class="stext">
            <label class="slabel" for="cb-interval">Interval</label>
            <p class="shelp">Seconds between messages, minimum 2.</p>
          </div>
          <div class="sctrl">
            <input
              id="cb-interval"
              type="number"
              min="2"
              max="60"
              step="0.5"
              value={cb.interval_secs}
              onchange={(e) =>
                app.updateConfig((c) => (c.osc.chatbox.interval_secs = Math.max(2, Number(e.currentTarget.value) || 2)))}
            />
          </div>
        </div>
        <Toggle
          id="chatbox-silent"
          label="Silent"
          hint="Send without the chatbox notification sound."
          checked={cb.silent}
          onchange={(v) => app.updateConfig((c) => (c.osc.chatbox.silent = v))}
        />
      {/if}
    </div>
  </div>

  <div class="section">
    <h2 class="section-label">
      Live log <span class="muted">last 50</span><span class="spacer"></span>
      <button class="btn small" aria-pressed={paused} onclick={() => (paused = !paused)}>{paused ? "Resume" : "Pause"}</button>
    </h2>
    <div class="log" role="log" aria-label="OSC messages sent">
      {#if log.length === 0}
        <p class="shelp empty">Nothing sent yet. Connect a sensor or press Send test sweep.</p>
      {:else}
        <table class="mono">
          <thead>
            <tr><th>time</th><th>target</th><th>address</th><th class="num">value</th><th>type</th></tr>
          </thead>
          <tbody>
            {#each log as e (e.seq)}
              <tr>
                <td class="t">{formatClock(e.t_ms)}</td>
                <td>{e.target}</td>
                <td class="addr" title={e.address}>{shortAddr(e.address)}</td>
                <td class="num">{e.value}</td>
                <td><span class="tag neutral">{e.kind}</span>{#if e.test}<span class="tag">test</span>{/if}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}
    </div>
  </div>
</section>

<style>
  .page-head .btn {
    align-self: center;
  }
  .targets {
    display: flex;
    flex-direction: column;
    gap: var(--s4);
  }
  .preview {
    padding: var(--s3) var(--s4);
    border-radius: var(--r-btn);
    background: var(--slate-2);
    color: var(--text);
    min-height: 40px;
  }
  .log {
    max-height: 340px;
    overflow: auto;
    border-radius: var(--r-row);
    border: 1px solid var(--slate-3);
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }
  th {
    position: sticky;
    top: 0;
    background: var(--slate-1);
    text-align: left;
    font-weight: 500;
    font-size: 11px;
    letter-spacing: 0.04em;
    color: var(--muted);
    padding: var(--s2) var(--s3);
    border-bottom: 1px solid var(--slate-3);
  }
  td {
    padding: 5px var(--s3);
    white-space: nowrap;
    color: var(--body);
  }
  tbody tr:nth-child(even) td {
    background: color-mix(in srgb, var(--slate-2) 50%, transparent);
  }
  td.t {
    color: var(--muted);
  }
  td.addr {
    color: var(--pink);
    max-width: 320px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .num {
    text-align: right;
  }
  td .tag {
    height: 18px;
    font-size: 10.5px;
    margin-right: 4px;
  }
  .empty {
    padding: var(--s5);
    text-align: center;
  }
</style>

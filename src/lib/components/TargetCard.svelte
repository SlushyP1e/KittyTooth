<script lang="ts">
  import { app } from "../stores/app.svelte";
  import Toggle from "./Toggle.svelte";
  import { METRIC_LABELS, PRESET_LABELS, isValidHost, isValidPort, normalizeOscAddress } from "../format";
  import { PRESET_DEFAULT_RANGE } from "../defaults";
  import type { Metric, OscTarget, Preset, TargetStatus, ValueKind } from "../types";

  let { target, index, status }: { target: OscTarget; index: number; status: TargetStatus | undefined } = $props();

  // Local drafts so invalid input never gets saved, but is still shown with an error.
  let hostDraft = $state("");
  let portDraft = $state("");
  $effect(() => {
    hostDraft = target.host;
    portDraft = String(target.port);
  });
  const hostOk = $derived(isValidHost(hostDraft));
  const portOk = $derived(isValidPort(Number(portDraft)));
  const range = $derived(target.range ?? PRESET_DEFAULT_RANGE[target.preset]);
  const idp = $derived(`target-${target.id}`);

  function update(fn: (t: OscTarget) => void) {
    app.updateConfig((c) => {
      const t = c.osc.targets[index];
      if (t) fn(t);
    });
  }

  function setRange(lo: number, hi: number) {
    if (Number.isFinite(lo) && Number.isFinite(hi) && lo >= 0 && hi <= 300 && lo < hi) update((t) => (t.range = [lo, hi]));
  }

  const metrics = Object.keys(METRIC_LABELS) as Metric[];
</script>

<article class="target card" aria-label="OSC target {target.name}">
  <div class="row top">
    <input
      class="name"
      type="text"
      aria-label="Target name"
      value={target.name}
      maxlength="40"
      oninput={(e) => update((t) => (t.name = e.currentTarget.value || "Target"))}
    />
    <span class="spacer"></span>
    {#if status?.dest && target.enabled}
      <span class="dest mono muted" title="Sending to">{status.dest}{status.via_oscquery ? " (OSCQuery)" : ""}</span>
    {/if}
    <Toggle label="Enabled" checked={target.enabled} onchange={(v) => update((t) => (t.enabled = v))} id="{idp}-en" />
  </div>

  <div class="grid">
    <label class="field">
      <span>Host</span>
      <input
        type="text"
        value={hostDraft}
        aria-invalid={!hostOk}
        oninput={(e) => {
          hostDraft = e.currentTarget.value;
          if (isValidHost(hostDraft)) update((t) => (t.host = hostDraft.trim()));
        }}
      />
      {#if !hostOk}<span class="invalid-msg">Enter an IP address or hostname, e.g. 127.0.0.1</span>{/if}
    </label>
    <label class="field">
      <span>Port</span>
      <input
        type="number"
        min="1"
        max="65535"
        value={portDraft}
        aria-invalid={!portOk}
        oninput={(e) => {
          portDraft = e.currentTarget.value;
          if (isValidPort(Number(portDraft))) update((t) => (t.port = Number(portDraft)));
        }}
      />
      {#if !portOk}<span class="invalid-msg">Port must be 1-65535 (VRChat uses 9000)</span>{/if}
    </label>
    <label class="field preset">
      <span>Parameter preset</span>
      <select value={target.preset} onchange={(e) => update((t) => (t.preset = e.currentTarget.value as Preset))}>
        {#each Object.entries(PRESET_LABELS) as [v, l] (v)}
          <option value={v}>{l}</option>
        {/each}
      </select>
    </label>
    <div class="field">
      <span>Normalize range (bpm)</span>
      <div class="row tight">
        <input
          type="number"
          aria-label="Range minimum"
          value={range[0]}
          onchange={(e) => setRange(Number(e.currentTarget.value), range[1])}
        />
        <span class="muted">to</span>
        <input
          type="number"
          aria-label="Range maximum"
          value={range[1]}
          onchange={(e) => setRange(range[0], Number(e.currentTarget.value))}
        />
      </div>
    </div>
  </div>

  <div class="toggles">
    <Toggle
      id="{idp}-oscq"
      label="Use VRChat's port from OSCQuery"
      hint="If VRChat advertises a different OSC port, send there instead of the port above."
      checked={target.use_oscquery}
      onchange={(v) => update((t) => (t.use_oscquery = v))}
    />
    <Toggle
      id="{idp}-chat"
      label="Send chatbox messages here"
      checked={target.chatbox}
      onchange={(v) => update((t) => (t.chatbox = v))}
    />
  </div>

  {#if target.preset === "custom"}
    <div class="custom stack">
      <h3 class="label">Custom parameters</h3>
      {#if target.custom.length === 0}
        <p class="hint">Map any value to any address. Bare names become /avatar/parameters/&lt;name&gt;.</p>
      {/if}
      {#each target.custom as m, i (i)}
        {@const ok = normalizeOscAddress(m.address) != null}
        <div class="mapping">
          <input
            type="text"
            aria-label="OSC address"
            placeholder="MyHeartRate or /full/address"
            value={m.address}
            aria-invalid={!ok}
            oninput={(e) => {
              const v = e.currentTarget.value;
              if (normalizeOscAddress(v)) update((t) => (t.custom[i].address = v));
            }}
          />
          <select
            aria-label="Value"
            value={m.metric}
            onchange={(e) => update((t) => (t.custom[i].metric = e.currentTarget.value as Metric))}
          >
            {#each metrics as k (k)}<option value={k}>{METRIC_LABELS[k]}</option>{/each}
          </select>
          <select
            aria-label="Type"
            value={m.kind}
            onchange={(e) => update((t) => (t.custom[i].kind = e.currentTarget.value as ValueKind))}
          >
            <option value="int">int</option>
            <option value="float">float</option>
            <option value="bool">bool</option>
          </select>
          <button class="btn ghost icon small" aria-label="Remove mapping" onclick={() => update((t) => t.custom.splice(i, 1))}
            >✕</button
          >
        </div>
      {/each}
      <div>
        <button
          class="btn small"
          onclick={() =>
            update((t) => t.custom.push({ address: `KittyTooth/Custom${t.custom.length + 1}`, metric: "hr", kind: "int" }))}
          >Add parameter</button
        >
      </div>
    </div>
  {/if}

  <div class="row foot">
    {#if status && target.enabled}
      <span class="muted small tabular"
        >{status.stats.sent} sent{status.stats.errors ? ` · ${status.stats.errors} failed` : ""}</span
      >
      {#if status.stats.last_error}<span class="invalid-msg">{status.stats.last_error}</span>{/if}
    {/if}
    <span class="spacer"></span>
    {#if app.config.osc.targets.length > 1}
      <button class="btn ghost danger small" onclick={() => app.updateConfig((c) => c.osc.targets.splice(index, 1))}
        >Remove target</button
      >
    {/if}
  </div>
</article>

<style>
  .target {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .top {
    flex-wrap: wrap;
  }
  .name {
    font-weight: 650;
    font-size: 15px;
    background: transparent;
    border-color: transparent;
    padding-left: 4px;
    max-width: 240px;
  }
  .name:hover {
    border-color: var(--border);
  }
  .grid {
    display: grid;
    grid-template-columns: 1.3fr 0.7fr 1.4fr 1.2fr;
    gap: 12px;
  }
  .tight {
    gap: 6px;
  }
  .tight input {
    width: 100%;
  }
  .toggles {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px 24px;
  }
  .mapping {
    display: grid;
    grid-template-columns: 1.4fr 1.2fr 0.6fr auto;
    gap: 8px;
  }
  .custom {
    padding-top: 6px;
    border-top: 1px solid var(--border);
  }
  .dest {
    font-size: 12px;
  }
  .small {
    font-size: 12.5px;
  }
  .foot {
    min-height: 28px;
  }
  @media (max-width: 900px) {
    .grid {
      grid-template-columns: 1fr 1fr;
    }
    .toggles {
      grid-template-columns: 1fr;
    }
  }
</style>

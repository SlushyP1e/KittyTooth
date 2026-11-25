<script lang="ts">
  import { app } from "../stores/app.svelte";
  import Toggle from "./Toggle.svelte";
  import Choice from "./Choice.svelte";
  import { METRIC_LABELS, isValidHost, isValidPort, normalizeOscAddress } from "../format";
  import { PRESET_DEFAULT_RANGE } from "../defaults";
  import type { Metric, OscTarget, Preset, TargetStatus, ValueKind } from "../types";

  let { target, index, status }: { target: OscTarget; index: number; status: TargetStatus | undefined } = $props();

  // Local drafts so invalid input is shown with an error but never saved.
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

  const PRESETS: { value: Preset; label: string }[] = [
    { value: "kitty_tooth", label: "KittyTooth" },
    { value: "hr_to_vrchat", label: "HRtoVRChat" },
    { value: "vrcosc", label: "VRCOSC" },
    { value: "custom", label: "Custom" },
  ];

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

<article class="hairlist target" aria-label="OSC target {target.name}">
  <div class="srow head">
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
      <span class="mono dest" title="Sending to">{status.dest}</span>
      {#if status.via_oscquery}<span class="tag">oscquery</span>{/if}
    {:else if !target.enabled}
      <span class="tag neutral">off</span>
    {/if}
    <Toggle bare label="Enabled" checked={target.enabled} onchange={(v) => update((t) => (t.enabled = v))} id="{idp}-en" />
  </div>

  <div class="srow">
    <div class="stext">
      <span class="slabel">Address</span>
      <p class="shelp">VRChat on this PC listens on 127.0.0.1, port 9000.</p>
    </div>
    <div class="sctrl addr">
      <label class="sr-only" for="{idp}-host">Host</label>
      <input
        id="{idp}-host"
        class="mono host"
        type="text"
        value={hostDraft}
        aria-invalid={!hostOk}
        oninput={(e) => {
          hostDraft = e.currentTarget.value;
          if (isValidHost(hostDraft)) update((t) => (t.host = hostDraft.trim()));
        }}
      />
      <span class="muted mono">:</span>
      <label class="sr-only" for="{idp}-port">Port</label>
      <input
        id="{idp}-port"
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
    </div>
  </div>
  {#if !hostOk || !portOk}
    <div class="srow msgrow">
      {#if !hostOk}<span class="invalid-msg">Enter an IP address or hostname, e.g. 127.0.0.1</span>{/if}
      {#if !portOk}<span class="invalid-msg">Port must be 1-65535 (VRChat uses 9000)</span>{/if}
    </div>
  {/if}

  <div class="srow">
    <div class="stext">
      <span class="slabel">Parameter preset</span>
      <p class="shelp">Match what your avatar was built for.</p>
    </div>
    <div class="sctrl">
      <Choice
        label="Parameter preset"
        value={target.preset}
        options={PRESETS}
        onchange={(v: Preset) => update((t) => (t.preset = v))}
      />
    </div>
  </div>

  <div class="srow">
    <div class="stext">
      <span class="slabel">Normalize range</span>
      <p class="shelp">BPM mapped to 0..1 for float parameters.</p>
    </div>
    <div class="sctrl">
      <input
        type="number"
        aria-label="Range minimum"
        value={range[0]}
        onchange={(e) => setRange(Number(e.currentTarget.value), range[1])}
      />
      <span class="muted mono">to</span>
      <input
        type="number"
        aria-label="Range maximum"
        value={range[1]}
        onchange={(e) => setRange(range[0], Number(e.currentTarget.value))}
      />
      <span class="muted mono">bpm</span>
    </div>
  </div>

  <Toggle
    id="{idp}-oscq"
    label="Use VRChat's port from OSCQuery"
    hint="If VRChat advertises a different OSC port, send there instead."
    checked={target.use_oscquery}
    onchange={(v) => update((t) => (t.use_oscquery = v))}
  />
  <Toggle
    id="{idp}-chat"
    label="Send chatbox messages here"
    checked={target.chatbox}
    onchange={(v) => update((t) => (t.chatbox = v))}
  />

  {#if target.preset === "custom"}
    <div class="srow stacked">
      <div class="stext">
        <span class="slabel">Custom parameters</span>
        <p class="shelp">Bare names become /avatar/parameters/&lt;name&gt;.</p>
      </div>
      {#each target.custom as m, i (i)}
        {@const ok = normalizeOscAddress(m.address) != null}
        <div class="mapping">
          <input
            class="mono"
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
            class="mono"
            aria-label="Type"
            value={m.kind}
            onchange={(e) => update((t) => (t.custom[i].kind = e.currentTarget.value as ValueKind))}
          >
            <option value="int">int</option>
            <option value="float">float</option>
            <option value="bool">bool</option>
          </select>
          <button class="btn quiet small" aria-label="Remove mapping" onclick={() => update((t) => t.custom.splice(i, 1))}
            >Remove</button
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

  <div class="srow foot">
    {#if status && target.enabled}
      <span class="mono muted small">{status.stats.sent} sent{status.stats.errors ? ` · ${status.stats.errors} failed` : ""}</span>
      {#if status.stats.last_error}<span class="invalid-msg">{status.stats.last_error}</span>{/if}
    {/if}
    <span class="spacer"></span>
    {#if app.config.osc.targets.length > 1}
      <button class="btn quiet small danger" onclick={() => app.updateConfig((c) => c.osc.targets.splice(index, 1))}
        >Remove target</button
      >
    {/if}
  </div>
</article>

<style>
  .head {
    gap: var(--s3);
  }
  .name {
    font-family: var(--sans);
    font-weight: 600;
    font-size: 15px;
    color: var(--text);
    background: transparent;
    border-color: transparent;
    padding-left: 0;
    max-width: 260px;
  }
  .name:hover {
    border-color: var(--slate-3);
    padding-left: var(--s3);
  }
  .name:focus-visible {
    padding-left: var(--s3);
  }
  .dest {
    font-size: 12px;
    color: var(--muted);
  }
  .addr {
    gap: 6px;
  }
  .host {
    width: 170px;
  }
  .msgrow {
    min-height: 0;
    padding-top: var(--s2);
    padding-bottom: var(--s2);
    flex-wrap: wrap;
  }
  .mapping {
    display: grid;
    grid-template-columns: 1.4fr 1.2fr 0.5fr auto;
    gap: var(--s2);
  }
  .foot {
    min-height: 44px;
    padding-top: var(--s2);
    padding-bottom: var(--s2);
  }
  .small {
    font-size: 12px;
  }
  @media (max-width: 980px) {
    .srow {
      flex-wrap: wrap;
    }
  }
</style>

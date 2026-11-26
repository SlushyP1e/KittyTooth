<script lang="ts">
  // Settings → Updates: status, Check now, release notes and Install and restart (or the
  // release page for portable copies), plus the auto-check switch.
  import { app } from "../stores/app.svelte";
  import { updates } from "../stores/update.svelte";
  import { formatBytes, notesToBlocks, progressPercent } from "../update";
  import Toggle from "./Toggle.svelte";
  import ErrorBanner from "./ErrorBanner.svelte";

  const u = $derived(updates);
  const info = $derived(u.info);
  const current = $derived(info?.current ?? app.info?.version ?? "");
  const notes = $derived(notesToBlocks(info?.notes));
  /** Heart rate is going out right now, so installing would interrupt it. */
  const live = $derived(app.snapshot.data_valid);
  let confirmInstall = $state(false);

  const pct = $derived(u.progress ? progressPercent(u.progress.downloaded, u.progress.total) : null);

  const title = $derived.by(() => {
    switch (u.status) {
      case "checking":
        return "Checking for updates";
      case "available":
        return `v${info?.version} is available`;
      case "downloading":
        return `Downloading v${info?.version}`;
      case "up_to_date":
        return "You're up to date";
      case "error":
        return "Couldn't check for updates";
      default:
        return current ? `KittyTooth v${current}` : "KittyTooth";
    }
  });

  const help = $derived.by(() => {
    if (u.status === "downloading") {
      if (!u.progress) return "Starting the download.";
      const size = u.progress.total ? ` of ${formatBytes(u.progress.total)}` : "";
      return `${formatBytes(u.progress.downloaded)}${size}${pct != null ? ` (${pct}%)` : ""}. KittyTooth restarts when it's done.`;
    }
    if (u.status === "available" && info && !info.can_install) {
      return "This copy can't update itself. Download the new version from the release page.";
    }
    if (u.status === "available") return "Installs only when you click. Your settings are kept.";
    if (u.lastChecked) return `Last checked ${new Date(u.lastChecked).toLocaleString()}.`;
    return "New versions are published on GitHub.";
  });

  async function install() {
    if (live && !confirmInstall) {
      confirmInstall = true;
      return;
    }
    confirmInstall = false;
    if (app.saving === "pending") await app.flushConfig();
    const version = info?.version ?? "";
    const ok = await updates.install();
    if (ok && app.isMock) app.toast({ kind: "success", title: `Updated to v${version}`, message: "Demo mode: nothing was installed." });
  }

  function setAuto(v: boolean) {
    app.updateConfig((cfg) => (cfg.updates.auto_check = v));
    if (v && u.status === "idle") void updates.check();
  }
</script>

<div class="section" id="updates">
  <h2 class="section-label">Updates <span class="spacer"></span>{#if current}<span class="ver">v{current}</span>{/if}</h2>
  <div class="hairlist">
    <div class="srow">
      <div class="stext">
        <span class="slabel">{title}</span>
        <p class="shelp" role="status" aria-live="polite">{help}</p>
      </div>
      <div class="sctrl">
        <button class="btn" onclick={() => updates.check()} disabled={u.busy}>{u.status === "checking" ? "Checking" : "Check now"}</button>
      </div>
    </div>
    {#if info && (u.status === "available" || u.status === "downloading")}
      {#if u.status === "downloading"}
        <div class="bar" role="progressbar" aria-label="Download progress" aria-valuemin={0} aria-valuemax={100} aria-valuenow={pct ?? undefined}>
          <span class="fill" class:indeterminate={pct == null} style:width={pct != null ? `${pct}%` : undefined}></span>
        </div>
      {/if}
      {#if notes.length}
        <div class="srow stacked">
          <span class="slabel">What's new in v{info.version}</span>
          <div class="notes">
            {#each notes as n, i (i)}
              {#if n.kind === "heading"}
                <p class="nh">{n.text}</p>
              {:else if n.kind === "item"}
                <p class="ni">{n.text}</p>
              {:else}
                <p class="nt">{n.text}</p>
              {/if}
            {/each}
          </div>
        </div>
      {/if}
      <div class="srow">
        <div class="stext">
          {#if info.can_install}
            <span class="slabel">{confirmInstall ? "Heart rate is live" : "Install and restart"}</span>
            <p class="shelp">
              {confirmInstall
                ? "OSC stops for a few seconds while KittyTooth restarts. Install now, or wait until you're done."
                : "Downloads, checks the signature and restarts KittyTooth on the new version."}
            </p>
          {:else}
            <span class="slabel">Portable version</span>
            <p class="shelp">Replace your KittyTooth exe with the one on the release page.</p>
          {/if}
        </div>
        <div class="sctrl">
          {#if info.can_install}
            {#if confirmInstall}
              <button class="btn primary" onclick={install}>Install now</button>
              <button class="btn quiet" onclick={() => (confirmInstall = false)}>Later</button>
            {:else}
              <button class="btn primary" onclick={install} disabled={u.busy}>
                {u.status === "downloading" ? "Installing" : "Install and restart"}
              </button>
            {/if}
          {:else}
            <button class="btn primary" onclick={() => updates.openRelease()}>Open release page</button>
          {/if}
        </div>
      </div>
    {/if}
    <Toggle
      label="Check for updates automatically"
      hint="On launch and every 6 hours. Nothing installs without your click."
      checked={app.config.updates.auto_check}
      onchange={setAuto}
    />
  </div>
  {#if u.error}
    <ErrorBanner error={u.error}>
      {#snippet actions()}
        {#if info}<button class="btn small" onclick={() => updates.openRelease()}>Release page</button>{/if}
      {/snippet}
    </ErrorBanner>
  {/if}
</div>

<style>
  .ver {
    text-transform: none;
  }
  .notes {
    display: flex;
    flex-direction: column;
    gap: 4px;
    max-height: 220px;
    overflow: auto;
    font-size: 13px;
    color: var(--body);
  }
  .notes p {
    margin: 0;
    overflow-wrap: anywhere;
  }
  .nh {
    margin-top: var(--s2) !important;
    font-family: var(--mono);
    font-size: 11px;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--dim);
  }
  .nh:first-child {
    margin-top: 0 !important;
  }
  .ni {
    position: relative;
    padding-left: 14px;
  }
  .ni::before {
    content: "";
    position: absolute;
    left: 2px;
    top: 0.6em;
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: var(--pink);
  }
  .nt {
    color: var(--muted);
  }
  .bar {
    position: relative;
    height: 3px;
    min-height: 0;
    overflow: hidden;
    background: var(--slate-2);
  }
  .fill {
    position: absolute;
    inset: 0 auto 0 0;
    background: var(--pink);
    transition: width 200ms;
  }
  .fill.indeterminate {
    width: 30%;
    animation: slide 1.1s ease-in-out infinite;
  }
  @keyframes slide {
    from {
      left: -30%;
    }
    to {
      left: 100%;
    }
  }
</style>

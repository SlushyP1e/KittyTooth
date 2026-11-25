<script lang="ts">
  import { onMount } from "svelte";
  import { app } from "./lib/stores/app.svelte";
  import { backend } from "./lib/api";
  import { applyTheme } from "./lib/theme";
  import { connectionStatus, formatBpm } from "./lib/format";
  import Logo from "./lib/components/Logo.svelte";
  import Toasts from "./lib/components/Toasts.svelte";
  import LiveView from "./lib/views/LiveView.svelte";
  import DevicesView from "./lib/views/DevicesView.svelte";
  import OscView from "./lib/views/OscView.svelte";
  import SettingsView from "./lib/views/SettingsView.svelte";
  import MiniView from "./lib/views/MiniView.svelte";

  type View = "live" | "devices" | "osc" | "settings" | "mini";
  const VIEWS: { id: Exclude<View, "mini">; label: string; icon: string }[] = [
    { id: "live", label: "Live", icon: "M3 12h4l2-5 4 10 2-5h6" },
    { id: "devices", label: "Devices", icon: "M7 7l10 10-5 5V2l5 5L7 17" },
    { id: "osc", label: "OSC", icon: "M4 12a8 8 0 0 1 16 0M7.5 12a4.5 4.5 0 0 1 9 0M12 12v8" },
    { id: "settings", label: "Settings", icon: "M4 6h10M18 6h2M4 12h4M12 12h8M4 18h12M20 18h0M14 4v4M8 10v4M16 16v4" },
  ];

  function parseHash(): View {
    const h = location.hash.replace(/^#\/?/, "");
    return (["live", "devices", "osc", "settings", "mini"] as const).find((v) => v === h) ?? "live";
  }

  let view = $state<View>(parseHash());
  const s = $derived(app.snapshot);
  const status = $derived(connectionStatus(s));
  const live = $derived(s.data_valid || s.test_sweep);
  let mainEl: HTMLElement | undefined = $state();

  function navigate(v: string) {
    location.hash = `#/${v}`;
  }

  $effect(() => {
    applyTheme(app.config.ui.theme, app.config.ui.accent);
  });

  $effect(() => {
    document.documentElement.classList.toggle("mini-mode", view === "mini");
  });

  onMount(() => {
    const onHash = () => {
      view = parseHash();
      mainEl?.scrollTo({ top: 0 });
    };
    window.addEventListener("hashchange", onHash);
    const mq = window.matchMedia?.("(prefers-color-scheme: light)");
    const onScheme = () => applyTheme(app.config.ui.theme, app.config.ui.accent);
    mq?.addEventListener?.("change", onScheme);
    void app.init(backend());
    return () => {
      window.removeEventListener("hashchange", onHash);
      mq?.removeEventListener?.("change", onScheme);
      app.destroy();
    };
  });
</script>

{#if view === "mini"}
  <MiniView />
{:else}
  <a class="skip" href="#main-content" onclick={(e) => { e.preventDefault(); mainEl?.focus(); }}>Skip to content</a>
  <div class="shell">
    <nav class="sidebar" aria-label="Main">
      <div class="brand">
        <Logo size={30} />
        <span class="brand-name">KittyTooth</span>
      </div>
      <ul>
        {#each VIEWS as v (v.id)}
          <li>
            <a href="#/{v.id}" class="nav-item" aria-current={view === v.id ? "page" : undefined}>
              <svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true">
                <path d={v.icon} fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" />
              </svg>
              <span>{v.label}</span>
            </a>
          </li>
        {/each}
      </ul>
      <div class="spacer"></div>
      <a href="#/live" class="pulse-pill {status.tone}" aria-label="Current heart rate">
        <span class="dot" aria-hidden="true"></span>
        <span class="tabular">{live ? `${formatBpm(s.bpm)} bpm` : status.label}</span>
      </a>
      {#if app.isMock}
        <p class="demo-note">Demo mode: simulated data</p>
      {/if}
    </nav>
    <main id="main-content" bind:this={mainEl} tabindex="-1">
      <div class="content">
        {#if !app.ready}
          <p class="muted loading">Loading...</p>
        {:else if view === "live"}
          <LiveView {navigate} />
        {:else if view === "devices"}
          <DevicesView />
        {:else if view === "osc"}
          <OscView />
        {:else if view === "settings"}
          <SettingsView />
        {/if}
      </div>
    </main>
  </div>
  <Toasts />
{/if}

<style>
  .shell {
    display: grid;
    grid-template-columns: 208px 1fr;
    height: 100%;
  }
  .sidebar {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 18px 12px;
    border-right: 1px solid var(--border);
    background: color-mix(in srgb, var(--surface) 55%, transparent);
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 4px 10px 18px;
  }
  .brand-name {
    font-family: var(--font-display);
    font-weight: 700;
    font-size: 17px;
    letter-spacing: -0.01em;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .nav-item {
    display: flex;
    align-items: center;
    gap: 12px;
    height: 38px;
    padding: 0 12px;
    border-radius: 10px;
    color: var(--muted);
    text-decoration: none;
    font-weight: 560;
  }
  .nav-item:hover {
    background: var(--surface-2);
    color: var(--text);
  }
  .nav-item[aria-current="page"] {
    background: var(--accent-soft);
    color: var(--text);
  }
  .nav-item[aria-current="page"] svg {
    color: var(--accent);
  }
  .spacer {
    flex: 1;
  }
  .pulse-pill {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 9px 12px;
    border-radius: 12px;
    background: var(--surface-2);
    border: 1px solid var(--border);
    color: var(--text);
    text-decoration: none;
    font-weight: 600;
    font-size: 13px;
  }
  .pulse-pill .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--faint);
  }
  .pulse-pill.ok .dot {
    background: var(--ok);
  }
  .pulse-pill.warn .dot {
    background: var(--warn);
  }
  .pulse-pill.error .dot {
    background: var(--error);
  }
  .pulse-pill.busy .dot {
    background: var(--busy);
  }
  .demo-note {
    font-size: 11.5px;
    color: var(--faint);
    padding: 4px 12px 0;
  }
  main {
    overflow: auto;
    outline: none;
  }
  .content {
    max-width: 1040px;
    margin: 0 auto;
    padding: 24px 28px 48px;
  }
  .loading {
    padding: 40px;
  }
  .skip {
    position: absolute;
    left: 12px;
    top: -60px;
    z-index: 100;
    padding: 8px 14px;
    border-radius: 10px;
    background: var(--accent);
    color: var(--accent-text);
    font-weight: 600;
  }
  .skip:focus {
    top: 12px;
  }
  @media (max-width: 860px) {
    .shell {
      grid-template-columns: 64px 1fr;
    }
    .brand-name,
    .nav-item span,
    .pulse-pill span:not(.dot),
    .demo-note {
      display: none;
    }
    .nav-item {
      justify-content: center;
      padding: 0;
    }
    .brand {
      justify-content: center;
      padding: 4px 0 18px;
    }
    .pulse-pill {
      justify-content: center;
    }
    .content {
      padding: 18px 16px 40px;
    }
  }
</style>

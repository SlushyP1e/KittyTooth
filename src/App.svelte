<script lang="ts">
  import { onMount } from "svelte";
  import { app } from "./lib/stores/app.svelte";
  import { backend } from "./lib/api";
  import { applyTheme } from "./lib/theme";
  import { formatBpm } from "./lib/format";
  import Logo from "./lib/components/Logo.svelte";
  import Toasts from "./lib/components/Toasts.svelte";
  import LiveView from "./lib/views/LiveView.svelte";
  import DevicesView from "./lib/views/DevicesView.svelte";
  import OscView from "./lib/views/OscView.svelte";
  import SettingsView from "./lib/views/SettingsView.svelte";
  import MiniView from "./lib/views/MiniView.svelte";

  type View = "live" | "devices" | "osc" | "settings" | "mini";
  type Dot = "live" | "ok" | "warn" | "err" | "idle";

  function parseHash(): View {
    const h = location.hash.replace(/^#\/?/, "");
    return (["live", "devices", "osc", "settings", "mini"] as const).find((v) => v === h) ?? "live";
  }

  let view = $state<View>(parseHash());
  const s = $derived(app.snapshot);
  const live = $derived(s.data_valid || s.test_sweep);
  let mainEl: HTMLElement | undefined = $state();

  const sensorDot = $derived<Dot>(
    s.error && s.state === "reconnecting"
      ? "err"
      : live
        ? "live"
        : s.state === "connecting" || s.state === "reconnecting" || s.state === "no_data"
          ? "warn"
          : "idle",
  );
  const enabledTargets = $derived(app.config.osc.targets.filter((t) => t.enabled).length);
  const groups = $derived([
    {
      label: "Sensor",
      items: [
        { id: "live", label: "Live", dot: sensorDot, count: null as number | null },
        { id: "devices", label: "Devices", dot: s.device ? sensorDot : ("idle" as Dot), count: s.scanning ? app.devices.length : null },
      ],
    },
    {
      label: "Output",
      items: [{ id: "osc", label: "OSC", dot: (live && enabledTargets ? "live" : "idle") as Dot, count: enabledTargets }],
    },
    { label: "App", items: [{ id: "settings", label: "Settings", dot: "idle" as Dot, count: null }] },
  ]);

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
  <a
    class="skip"
    href="#main-content"
    onclick={(e) => {
      e.preventDefault();
      mainEl?.focus();
    }}>Skip to content</a
  >
  <div class="shell">
    <nav class="sidebar" aria-label="Main">
      <div class="brand">
        <Logo size={22} />
        <span class="brand-name">KittyTooth</span>
      </div>
      {#each groups as g (g.label)}
        <div class="group">
          <p class="glabel">{g.label}</p>
          <ul>
            {#each g.items as it (it.id)}
              <li>
                <a href="#/{it.id}" class="nav-item" aria-current={view === it.id ? "page" : undefined}>
                  <span class="dot {it.dot}" aria-hidden="true"></span>
                  <span class="nlabel">{it.label}</span>
                  {#if it.count != null}<span class="count mono">{it.count}</span>{/if}
                </a>
              </li>
            {/each}
          </ul>
        </div>
      {/each}
      <div class="spacer"></div>
      <a href="#/live" class="now" aria-label="Current heart rate">
        <span class="now-bpm tabular">{live ? formatBpm(s.bpm) : "--"}</span>
        <span class="now-unit mono">bpm</span>
      </a>
      <p class="foot mono">{app.isMock ? "demo data" : `v${app.info?.version ?? ""}`}</p>
    </nav>
    <div class="main-wrap">
      <main id="main-content" bind:this={mainEl} tabindex="-1">
        <div class="content" class:wide={view === "live"}>
          {#if !app.ready}
            <p class="muted loading">Loading</p>
          {:else}
            {#key view}
              {#if view === "live"}
                <LiveView {navigate} />
              {:else if view === "devices"}
                <DevicesView />
              {:else if view === "osc"}
                <OscView />
              {:else if view === "settings"}
                <SettingsView />
              {/if}
            {/key}
          {/if}
        </div>
      </main>
    </div>
  </div>
  <Toasts />
{/if}

<style>
  .shell {
    display: grid;
    grid-template-columns: 200px 1fr;
    height: 100%;
    background: var(--slate-0);
  }
  .sidebar {
    display: flex;
    flex-direction: column;
    gap: var(--s5);
    padding: var(--s5) var(--s3) var(--s4);
    border-right: 1px solid var(--slate-3);
    background: var(--slate-0);
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 var(--s2);
  }
  .brand-name {
    font-family: var(--display);
    font-weight: 700;
    font-size: 18px;
    color: var(--text);
    letter-spacing: -0.01em;
  }
  .group {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .glabel {
    padding: 0 var(--s2);
    font-family: var(--mono);
    font-size: 11px;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--muted);
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
    gap: 10px;
    height: 34px;
    padding: 0 var(--s2);
    border-radius: var(--r-btn);
    color: var(--body);
    text-decoration: none;
    font-weight: 500;
    transition: background 120ms;
  }
  .nav-item:hover {
    background: var(--slate-2);
  }
  .nav-item[aria-current="page"] {
    background: var(--pink-soft);
    color: var(--pink);
  }
  .nlabel {
    flex: 1;
  }
  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--slate-3);
    flex: none;
  }
  .dot.live {
    background: var(--pink);
    box-shadow: 0 0 8px var(--pink);
  }
  .dot.ok {
    background: var(--green);
  }
  .dot.warn {
    background: var(--amber);
  }
  .dot.err {
    background: var(--red);
  }
  .count {
    min-width: 20px;
    height: 18px;
    padding: 0 6px;
    border-radius: var(--r-pill);
    background: var(--pink);
    color: var(--slate-0);
    font-size: 11px;
    font-weight: 500;
    display: inline-grid;
    place-items: center;
  }
  .now {
    display: flex;
    align-items: baseline;
    gap: 6px;
    padding: 0 var(--s2);
    text-decoration: none;
  }
  .now-bpm {
    font-family: var(--display);
    font-weight: 700;
    font-size: 26px;
    color: var(--text);
    letter-spacing: -0.02em;
  }
  .now-unit,
  .foot {
    font-size: 11px;
    color: var(--muted);
  }
  .foot {
    padding: 0 var(--s2);
  }
  .main-wrap {
    padding: 10px;
    min-width: 0;
    min-height: 0;
  }
  main {
    height: 100%;
    overflow: auto;
    background: var(--slate-1);
    border-radius: var(--r-panel);
  }
  main:focus-visible {
    outline-offset: -2px;
  }
  .content {
    max-width: 860px;
    margin: 0 auto;
    padding: var(--s6) var(--s6) 56px;
  }
  .content.wide {
    max-width: 1000px;
  }
  .loading {
    padding: 40px;
  }
  .skip {
    position: absolute;
    left: 12px;
    top: -60px;
    z-index: 100;
    padding: var(--s2) var(--s4);
    border-radius: var(--r-btn);
    background: var(--pink);
    color: var(--on-pink);
    font-weight: 600;
  }
  .skip:focus {
    top: 12px;
  }
  @media (max-width: 860px) {
    .shell {
      grid-template-columns: 168px 1fr;
    }
    .content {
      padding: var(--s5) var(--s5) 48px;
    }
  }
</style>

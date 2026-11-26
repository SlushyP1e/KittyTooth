<script lang="ts">
  import { onMount } from "svelte";
  import { app } from "./lib/stores/app.svelte";
  import { backend } from "./lib/api";
  import { applyMotion, applyTheme } from "./lib/theme";
  import { formatBpm } from "./lib/format";
  import Logo from "./lib/components/Logo.svelte";
  import Toasts from "./lib/components/Toasts.svelte";
  import LiveView from "./lib/views/LiveView.svelte";
  import DevicesView from "./lib/views/DevicesView.svelte";
  import SourceView from "./lib/views/SourceView.svelte";
  import OscView from "./lib/views/OscView.svelte";
  import SettingsView from "./lib/views/SettingsView.svelte";
  import MiniView from "./lib/views/MiniView.svelte";
  import { spring, stagger } from "./lib/motion";

  type View = "live" | "source" | "devices" | "osc" | "settings" | "mini";
  type Dot = "live" | "ok" | "warn" | "err" | "idle";

  function parseHash(): View {
    const h = location.hash.replace(/^#\/?/, "");
    return (["live", "source", "devices", "osc", "settings", "mini"] as const).find((v) => v === h) ?? "live";
  }

  let view = $state<View>(parseHash());
  const s = $derived(app.snapshot);
  const live = $derived(s.data_valid || s.test_sweep);
  let mainEl: HTMLElement | undefined = $state();
  let navEl: HTMLElement | undefined = $state();
  let gliderEl: HTMLElement | undefined = $state();
  let rolledBpm = $state(0);
  const ORDER: View[] = ["live", "source", "devices", "osc", "settings"];
  let dir = $state(1);

  // Line icons per nav item (24-unit grid, stroked with currentColor).
  // Each icon is a path plus optional circles [cx, cy, r].
  const ICONS: Record<string, { d: string; c?: [number, number, number][] }> = {
    live: { d: "M3 12h4l2-5 4 10 2-5h6" },
    source: {
      d: "M8.5 8.5a5 5 0 0 0 0 7M15.5 8.5a5 5 0 0 1 0 7M5.5 5.5a9 9 0 0 0 0 13M18.5 5.5a9 9 0 0 1 0 13",
      c: [[12, 12, 2]],
    },
    devices: { d: "m7 7 10 10-5 4V3l5 4L7 17" },
    osc: {
      d: "M4 12h3M17 12h3",
      c: [
        [12, 12, 5],
        [12, 12, 1.5],
      ],
    },
    settings: {
      d: "M4 7h10M18 7h2M4 17h4M12 17h8",
      c: [
        [16, 7, 2],
        [10, 17, 2],
      ],
    },
  };

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
        { id: "source", label: "Source", dot: sensorDot, count: null },
        {
          id: "devices",
          label: "Devices",
          dot: s.device && (s.source === "ble" || s.source === "demo") ? sensorDot : ("idle" as Dot),
          count: s.scanning ? app.devices.length : null,
        },
      ],
    },
    {
      label: "Output",
      items: [{ id: "osc", label: "OSC", dot: (live && enabledTargets ? "live" : "idle") as Dot, count: enabledTargets }],
    },
    { label: "App", items: [{ id: "settings", label: "Settings", dot: "idle" as Dot, count: null }] },
  ]);

  // The selection highlight glides between items on a spring and squishes slightly on arrival.
  const gy = spring(0, (v) => {
    if (gliderEl) gliderEl.style.translate = `0 ${v}px`;
  });
  const gs = spring(
    1,
    (v) => {
      if (gliderEl) gliderEl.style.scale = `${v}`;
    },
    { k: 500, d: 22 },
  );
  let placed = false;
  function placeGlider() {
    const a = navEl?.querySelector<HTMLElement>('.nav-item[aria-current="page"]');
    if (!a || !gliderEl) return;
    gliderEl.style.opacity = "1";
    if (!placed) {
      gy.snap(a.offsetTop);
      placed = true;
    } else {
      gy.set(a.offsetTop);
      gs.kick(0.94);
    }
  }
  $effect(() => {
    void view;
    void navEl;
    queueMicrotask(placeGlider);
  });

  // Live numbers roll instead of jumping.
  const bpmS = spring(0, (v) => (rolledBpm = Math.round(v)), { k: 60, d: 14 });
  let bpmSeen = $state(false);
  $effect(() => {
    const b = s.bpm;
    if (!live || b == null) {
      bpmSeen = false;
      return;
    }
    if (!bpmSeen) {
      bpmS.snap(b);
      bpmSeen = true;
    } else bpmS.set(b);
  });

  function navigate(v: string) {
    location.hash = `#/${v}`;
  }

  $effect(() => {
    applyTheme(app.config.ui.theme, app.config.ui.accent);
  });

  $effect(() => {
    applyMotion(app.config.ui.motion ?? "on");
  });

  $effect(() => {
    document.documentElement.classList.toggle("mini-mode", view === "mini");
  });

  onMount(() => {
    const onHash = () => {
      const next = parseHash();
      dir = ORDER.indexOf(next) >= ORDER.indexOf(view) ? 1 : -1;
      view = next;
      mainEl?.scrollTo({ top: 0 });
    };
    window.addEventListener("hashchange", onHash);
    const mq = window.matchMedia?.("(prefers-color-scheme: light)");
    const onScheme = () => applyTheme(app.config.ui.theme, app.config.ui.accent);
    mq?.addEventListener?.("change", onScheme);
    const mqMotion = window.matchMedia?.("(prefers-reduced-motion: reduce)");
    const onMotion = () => applyMotion(app.config.ui.motion ?? "on");
    mqMotion?.addEventListener?.("change", onMotion);
    void app.init(backend());
    return () => {
      window.removeEventListener("hashchange", onHash);
      mq?.removeEventListener?.("change", onScheme);
      mqMotion?.removeEventListener?.("change", onMotion);
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
    <nav class="sidebar" aria-label="Main" bind:this={navEl}>
      <div class="brand">
        <Logo size={22} spark />
        <span class="brand-name">KittyTooth</span>
      </div>
      <div class="glider" bind:this={gliderEl} aria-hidden="true"></div>
      {#each groups as g (g.label)}
        <div class="group">
          <p class="glabel">{g.label}</p>
          <ul>
            {#each g.items as it (it.id)}
              <li>
                <a href="#/{it.id}" class="nav-item" aria-current={view === it.id ? "page" : undefined}>
                  <svg
                    class="icon"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="1.8"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    aria-hidden="true"
                    ><path d={ICONS[it.id].d} />{#each ICONS[it.id].c ?? [] as [cx, cy, r], ci (ci)}<circle
                        {cx}
                        {cy}
                        {r}
                      />{/each}</svg
                  >
                  <span class="nlabel">{it.label}</span>
                  {#if it.count != null}<span class="count mono">{it.count}</span>{/if}
                  <span class="dot {it.dot}" aria-hidden="true"></span>
                </a>
              </li>
            {/each}
          </ul>
        </div>
      {/each}
      <div class="spacer"></div>
      <div class="rule" aria-hidden="true"></div>
      <a href="#/live" class="now" aria-label="Current heart rate">
        {#if live}
          <span class="now-bpm tabular">{bpmSeen ? rolledBpm : formatBpm(s.bpm)}</span>
        {:else}
          <span class="now-bpm tabular">--</span>
        {/if}
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
              <div class="view" use:stagger={{ dir }}>
              {#if view === "live"}
                <LiveView {navigate} />
              {:else if view === "source"}
                <SourceView {navigate} />
              {:else if view === "devices"}
                <DevicesView />
              {:else if view === "osc"}
                <OscView />
              {:else if view === "settings"}
                <SettingsView />
              {/if}
              </div>
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
    position: relative;
    display: flex;
    flex-direction: column;
    gap: var(--s5);
    padding: var(--s5) var(--s3) var(--s4);
    border-right: 1px solid var(--line);
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
    color: var(--dim);
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
    color: var(--muted);
    text-decoration: none;
    font-weight: 500;
    position: relative;
    z-index: 1;
    transition:
      color 200ms,
      background 200ms,
      transform 300ms var(--spring);
  }
  .nav-item:hover {
    color: var(--text);
    background: var(--hover);
  }
  .nav-item:active {
    transform: scale(0.97);
  }
  .nav-item[aria-current="page"] {
    color: var(--pink);
    background: transparent;
  }
  .icon {
    width: 16px;
    height: 16px;
    flex: none;
  }
  .glider {
    position: absolute;
    left: var(--s3);
    right: var(--s3);
    top: 0;
    height: 34px;
    border-radius: var(--r-btn);
    background: var(--pink-soft);
    box-shadow: inset 0 0 0 1px var(--pink-line);
    pointer-events: none;
    opacity: 0;
    z-index: 0;
  }
  .rule {
    height: 1px;
    margin: 0 var(--s2) calc(var(--s2) * -1);
    background: var(--line);
  }
  .nlabel {
    flex: 1;
  }
  .dot {
    width: 6px;
    height: 6px;
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
    border: 1px solid var(--line);
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

<script lang="ts" module>
  export interface Marker {
    t: number;
    label: string;
    tone: "high" | "low";
  }
</script>

<script lang="ts">
  import { onMount } from "svelte";
  import { cssVar, withAlpha } from "../theme";

  // A whole saved session on a canvas: clock-time axis, gaps left open, optional markers
  // (alerts) and a hover readout. Same look as the live graph, but static.

  let {
    t,
    v,
    unit = "bpm",
    label = "Heart rate",
    color = "--pink",
    markers = [],
    minSpan = 40,
    gapMs = 10_000,
  }: {
    /** Unix ms, ascending. */
    t: number[];
    v: number[];
    unit?: string;
    label?: string;
    /** Token name for the line colour. */
    color?: string;
    markers?: Marker[];
    /** Smallest y range shown, in units. */
    minSpan?: number;
    /** Points further apart than this are drawn as a gap. */
    gapMs?: number;
  } = $props();

  let canvas: HTMLCanvasElement | undefined = $state();
  let ctx: CanvasRenderingContext2D | null = null;
  let cssW = 0;
  let cssH = 0;
  let dpr = 1;
  let hoverX = $state<number | null>(null);
  let readout = $state<{ x: number; text: string } | null>(null);
  const colors = { line: "", grid: "", text: "", high: "", low: "" };
  const PAD = { l: 34, r: 14, t: 14, b: 24 };

  const ext = $derived.by(() => {
    if (!v.length) return null;
    let lo = Infinity;
    let hi = -Infinity;
    for (const x of v) {
      if (x < lo) lo = x;
      if (x > hi) hi = x;
    }
    return [lo, hi] as const;
  });
  const ariaLabel = $derived(
    ext ? `${label} graph, between ${Math.round(ext[0])} and ${Math.round(ext[1])} ${unit}` : `${label} graph, no data`,
  );

  function readColors() {
    colors.line = cssVar(color);
    colors.grid = cssVar("--slate-3");
    colors.text = cssVar("--muted");
    colors.high = cssVar("--red");
    colors.low = cssVar("--amber");
  }

  function resize() {
    if (!canvas) return;
    const rect = canvas.parentElement!.getBoundingClientRect();
    dpr = Math.min(window.devicePixelRatio || 1, 2);
    cssW = Math.max(100, Math.floor(rect.width));
    cssH = Math.max(80, Math.floor(rect.height));
    canvas.width = Math.round(cssW * dpr);
    canvas.height = Math.round(cssH * dpr);
    canvas.style.width = `${cssW}px`;
    canvas.style.height = `${cssH}px`;
  }

  function clock(ms: number): string {
    return new Date(ms).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
  }

  function scales() {
    const t0 = t[0] ?? 0;
    const t1 = Math.max(t0 + 60_000, t[t.length - 1] ?? 0);
    let lo = ext ? Math.floor((ext[0] - minSpan * 0.15) / 10) * 10 : 50;
    let hi = ext ? Math.ceil((ext[1] + minSpan * 0.15) / 10) * 10 : 150;
    if (hi - lo < minSpan) {
      const mid = (hi + lo) / 2;
      lo = Math.max(0, Math.floor((mid - minSpan / 2) / 10) * 10);
      hi = lo + minSpan;
    }
    const plotW = cssW - PAD.l - PAD.r;
    const plotH = cssH - PAD.t - PAD.b;
    return {
      t0,
      t1,
      lo,
      hi,
      plotW,
      plotH,
      X: (x: number) => PAD.l + ((x - t0) / (t1 - t0)) * plotW,
      Y: (y: number) => PAD.t + (1 - (y - lo) / (hi - lo)) * plotH,
      T: (px: number) => t0 + ((px - PAD.l) / plotW) * (t1 - t0),
    };
  }

  function draw() {
    if (!ctx) return;
    const sc = scales();
    const { X, Y, lo, hi, plotH, plotW } = sc;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, cssW, cssH);

    const step = hi - lo > 100 ? 40 : hi - lo > 60 ? 20 : 10;
    ctx.strokeStyle = withAlpha(colors.grid, 0.5);
    ctx.lineWidth = 1;
    ctx.fillStyle = colors.text;
    ctx.font = "11px 'JetBrains Mono', monospace";
    ctx.textAlign = "right";
    ctx.textBaseline = "middle";
    for (let y = Math.ceil(lo / step) * step; y <= hi; y += step) {
      const py = Math.round(Y(y)) + 0.5;
      ctx.beginPath();
      ctx.moveTo(PAD.l, py);
      ctx.lineTo(cssW - PAD.r, py);
      ctx.stroke();
      ctx.fillText(String(y), PAD.l - 10, py);
    }
    ctx.textAlign = "center";
    ctx.textBaseline = "alphabetic";
    const ticks = cssW > 520 ? 5 : 3;
    for (let i = 0; i < ticks; i++) {
      const tt = sc.t0 + ((sc.t1 - sc.t0) * i) / (ticks - 1);
      ctx.fillText(clock(tt), Math.min(cssW - PAD.r - 18, Math.max(PAD.l + 18, X(tt))), cssH - 6);
    }

    if (!t.length) {
      ctx.fillText("No data", PAD.l + plotW / 2, PAD.t + plotH / 2);
      return;
    }

    // Decimate to min/max per pixel column; break the line at gaps.
    const segments: number[][] = [];
    let pts: number[] = [];
    let bx = -1;
    let bMin = 0;
    let bMax = 0;
    let prevT = -1;
    const flush = () => {
      if (bx < 0) return;
      pts.push(bx, Y(bMin));
      if (bMax !== bMin) pts.push(bx + 0.5, Y(bMax));
    };
    for (let i = 0; i < t.length; i++) {
      if (prevT >= 0 && t[i] - prevT > gapMs) {
        flush();
        bx = -1;
        if (pts.length) segments.push(pts);
        pts = [];
      }
      prevT = t[i];
      const x = Math.round(X(t[i]));
      if (x !== bx) {
        flush();
        bx = x;
        bMin = bMax = v[i];
      } else {
        bMin = Math.min(bMin, v[i]);
        bMax = Math.max(bMax, v[i]);
      }
    }
    flush();
    if (pts.length) segments.push(pts);

    const grad = ctx.createLinearGradient(0, PAD.t, 0, PAD.t + plotH);
    grad.addColorStop(0, withAlpha(colors.line, 0.12));
    grad.addColorStop(1, withAlpha(colors.line, 0));
    for (const seg of segments) {
      if (seg.length < 4) continue;
      ctx.beginPath();
      ctx.moveTo(seg[0], seg[1]);
      for (let i = 2; i < seg.length; i += 2) ctx.lineTo(seg[i], seg[i + 1]);
      ctx.lineTo(seg[seg.length - 2], PAD.t + plotH);
      ctx.lineTo(seg[0], PAD.t + plotH);
      ctx.closePath();
      ctx.fillStyle = grad;
      ctx.fill();
      ctx.beginPath();
      ctx.moveTo(seg[0], seg[1]);
      for (let i = 2; i < seg.length; i += 2) ctx.lineTo(seg[i], seg[i + 1]);
      ctx.strokeStyle = colors.line;
      ctx.lineWidth = 1.5;
      ctx.lineJoin = "round";
      ctx.stroke();
    }

    // Alert markers: a dashed rule with a small flag at the top.
    for (const m of markers) {
      const x = Math.round(X(m.t)) + 0.5;
      const c = m.tone === "high" ? colors.high : colors.low;
      ctx.save();
      ctx.setLineDash([3, 3]);
      ctx.strokeStyle = withAlpha(c, 0.7);
      ctx.beginPath();
      ctx.moveTo(x, PAD.t);
      ctx.lineTo(x, PAD.t + plotH);
      ctx.stroke();
      ctx.restore();
      ctx.fillStyle = c;
      ctx.beginPath();
      if (m.tone === "high") {
        ctx.moveTo(x - 4.5, PAD.t);
        ctx.lineTo(x + 4.5, PAD.t);
        ctx.lineTo(x, PAD.t - 0.5 + 7);
      } else {
        ctx.moveTo(x - 4.5, PAD.t + 7);
        ctx.lineTo(x + 4.5, PAD.t + 7);
        ctx.lineTo(x, PAD.t);
      }
      ctx.closePath();
      ctx.fill();
    }

    // Hover crosshair.
    if (hoverX != null) {
      const tt = sc.T(hoverX);
      let i = nearest(tt);
      if (i >= 0 && Math.abs(t[i] - tt) > gapMs) i = -1;
      if (i >= 0) {
        const px = X(t[i]);
        const py = Y(v[i]);
        ctx.strokeStyle = withAlpha(colors.text, 0.5);
        ctx.beginPath();
        ctx.moveTo(Math.round(px) + 0.5, PAD.t);
        ctx.lineTo(Math.round(px) + 0.5, PAD.t + plotH);
        ctx.stroke();
        ctx.beginPath();
        ctx.arc(px, py, 3.5, 0, Math.PI * 2);
        ctx.fillStyle = colors.line;
        ctx.fill();
        const near = markers.find((m) => Math.abs(X(m.t) - px) < 6);
        readout = {
          x: Math.min(Math.max(px, PAD.l + 60), cssW - PAD.r - 60),
          text: `${clock(t[i])} · ${Math.round(v[i])} ${unit}${near ? ` · ${near.label}` : ""}`,
        };
      } else readout = null;
    } else readout = null;
  }

  function nearest(tt: number): number {
    let lo = 0;
    let hi = t.length - 1;
    if (hi < 0) return -1;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (t[mid] < tt) lo = mid + 1;
      else hi = mid;
    }
    if (lo > 0 && Math.abs(t[lo - 1] - tt) < Math.abs(t[lo] - tt)) lo--;
    return lo;
  }

  let raf = 0;
  function schedule() {
    if (!raf && ctx) raf = requestAnimationFrame(() => ((raf = 0), draw()));
  }

  $effect(() => {
    void t;
    void v;
    void markers;
    void hoverX;
    schedule();
  });

  function onMove(e: PointerEvent) {
    const r = canvas!.getBoundingClientRect();
    const x = e.clientX - r.left;
    hoverX = x >= PAD.l && x <= cssW - PAD.r ? x : null;
  }

  onMount(() => {
    ctx = canvas!.getContext("2d");
    readColors();
    resize();
    const ro = new ResizeObserver(() => {
      resize();
      schedule();
    });
    ro.observe(canvas!.parentElement!);
    const mo = new MutationObserver(() => {
      readColors();
      schedule();
    });
    mo.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme", "data-accent", "style"] });
    void document.fonts?.ready.then(schedule);
    schedule();
    return () => {
      ro.disconnect();
      mo.disconnect();
      if (raf) cancelAnimationFrame(raf);
    };
  });
</script>

<div class="graph" role="img" aria-label={ariaLabel}>
  <canvas bind:this={canvas} aria-hidden="true" onpointermove={onMove} onpointerleave={() => (hoverX = null)}></canvas>
  {#if readout}
    <span class="readout mono" style:left="{readout.x}px">{readout.text}</span>
  {/if}
</div>

<style>
  .graph {
    position: relative;
    width: 100%;
    height: 100%;
    min-height: 120px;
  }
  canvas {
    position: absolute;
    inset: 0;
    display: block;
    cursor: crosshair;
  }
  .readout {
    position: absolute;
    top: 0;
    translate: -50% 0;
    padding: 2px 8px;
    border-radius: var(--r-pill);
    background: var(--slate-2);
    box-shadow: inset 0 0 0 1px var(--line-2);
    color: var(--text);
    font-size: 11px;
    white-space: nowrap;
    pointer-events: none;
  }
</style>

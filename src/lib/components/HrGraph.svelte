<script lang="ts">
  import { onMount } from "svelte";
  import type { HistoryBuffer } from "../stores/history";
  import type { GraphRange } from "../types";
  import { cssVar, prefersReducedMotion, withAlpha } from "../theme";

  // Live heart rate graph on a canvas. Scrolls smoothly at display rate while data is live;
  // redraws only on change otherwise (and under reduced motion). No allocations per frame
  // beyond the canvas path.
  let {
    history,
    version,
    range,
    live,
    beat = 0,
    maxHr = 190,
    thresholds = [50, 60, 70, 80, 90],
  }: {
    history: HistoryBuffer;
    version: number;
    range: GraphRange;
    live: boolean;
    beat?: number;
    maxHr?: number;
    thresholds?: number[];
  } = $props();

  let canvas: HTMLCanvasElement | undefined = $state();
  let ctx: CanvasRenderingContext2D | null = null;
  let cssW = 0;
  let cssH = 0;
  let dpr = 1;
  let raf = 0;
  let dirty = true;
  let yLo = 50;
  let yHi = 150;
  let ariaLabel = $state("Heart rate graph");
  const reduced = prefersReducedMotion();
  const colors = { accent: "", grid: "", text: "", surface: "" };

  const RANGE_MS: Record<GraphRange, number> = { "60s": 60_000, "5m": 300_000, session: 0 };
  const PAD = { l: 34, r: 14, t: 10, b: 24 };

  function readColors() {
    colors.accent = cssVar("--pink");
    colors.grid = cssVar("--slate-3");
    colors.text = cssVar("--muted");
    colors.surface = cssVar("--slate-1");
    dirty = true;
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
    dirty = true;
  }

  $effect(() => {
    // Any of these changing requires a redraw.
    void version;
    void range;
    void live;
    void maxHr;
    void thresholds;
    dirty = true;
    ensureLoop();
  });

  $effect(() => {
    if (beat > 0) {
      dirty = true;
      ensureLoop();
    }
  });

  function fmtAgo(ms: number): string {
    const s = Math.round(ms / 1000);
    if (s <= 90) return `-${s}s`;
    const m = Math.round(s / 60);
    return `-${m}m`;
  }

  function draw(nowPerf: number) {
    if (!ctx || !canvas) return;
    const n = history.length;
    const now = Date.now();
    const last = history.lastTime();
    const first = history.firstTime();
    const t1 = live || last == null ? now : last;
    let t0 = range === "session" ? (first ?? t1 - 60_000) : t1 - RANGE_MS[range];
    if (t1 - t0 < 60_000) t0 = t1 - 60_000;
    const span = t1 - t0;

    const plotW = cssW - PAD.l - PAD.r;
    const plotH = cssH - PAD.t - PAD.b;
    const startIdx = Math.max(0, history.indexAtOrAfter(t0) - 1);

    // Target y range from visible data, eased for smooth rescaling.
    const ext = history.extent(startIdx, n - 1);
    let tLo = 50;
    let tHi = 150;
    if (ext) {
      tLo = Math.floor((ext[0] - 8) / 10) * 10;
      tHi = Math.ceil((ext[1] + 8) / 10) * 10;
      if (tHi - tLo < 40) {
        const mid = (tHi + tLo) / 2;
        tLo = Math.floor((mid - 20) / 10) * 10;
        tHi = tLo + 40;
      }
    }
    const k = reduced ? 1 : 0.18;
    yLo += (tLo - yLo) * k;
    yHi += (tHi - yHi) * k;
    const settling = Math.abs(tLo - yLo) > 0.2 || Math.abs(tHi - yHi) > 0.2;
    if (!settling) {
      yLo = tLo;
      yHi = tHi;
    }

    const X = (t: number) => PAD.l + ((t - t0) / span) * plotW;
    const Y = (v: number) => PAD.t + (1 - (v - yLo) / (yHi - yLo)) * plotH;

    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, cssW, cssH);

    // Grid + labels.
    const step = yHi - yLo > 100 ? 40 : yHi - yLo > 60 ? 20 : 10;
    ctx.strokeStyle = withAlpha(colors.grid, 0.5);
    ctx.lineWidth = 1;
    ctx.fillStyle = colors.text;
    ctx.font = "11px 'JetBrains Mono', monospace";
    ctx.textAlign = "right";
    ctx.textBaseline = "middle";
    for (let v = Math.ceil(yLo / step) * step; v <= yHi; v += step) {
      const y = Math.round(Y(v)) + 0.5;
      ctx.beginPath();
      ctx.moveTo(PAD.l, y);
      ctx.lineTo(cssW - PAD.r, y);
      ctx.stroke();
      ctx.fillText(String(v), PAD.l - 10, y);
    }
    ctx.textAlign = "center";
    ctx.textBaseline = "alphabetic";
    for (let i = 0; i <= 4; i++) {
      const t = t0 + (span * i) / 4;
      const label = i === 4 ? "now" : fmtAgo(t1 - t);
      ctx.fillText(label, Math.min(cssW - PAD.r - 12, Math.max(PAD.l + 12, X(t))), cssH - 6);
    }

    // Data path, decimated to at most ~1 point per pixel column with min/max.
    if (n > 0 && startIdx < n) {
      const pts: number[] = [];
      let bucketX = -1;
      let bMin = 0;
      let bMax = 0;
      let bLast = 0;
      let prevT = -1;
      const flush = () => {
        if (bucketX < 0) return;
        pts.push(bucketX, Y(bMin));
        if (bMax !== bMin) pts.push(bucketX + 0.5, Y(bMax));
        if (bLast !== bMax) pts.push(bucketX + 0.9, Y(bLast));
      };
      const segments: number[][] = [];
      // Break the line only at gaps longer than this source's usual pace. Slow senders (phone
      // apps relaying a watch) report every 5-25 s; mirrors the engine's learned stale timeout.
      let longest = 0;
      let gapCount = 0;
      for (let i = Math.max(startIdx + 1, n - 12); i < n; i++) {
        const g = history.t[i] - history.t[i - 1];
        if (g <= 60_000) {
          longest = Math.max(longest, g);
          gapCount++;
        }
      }
      const gapBreakMs = gapCount >= 3 ? Math.min(60_000, Math.max(5_000, longest * 1.5)) : 5_000;
      for (let i = startIdx; i < n; i++) {
        const t = history.t[i];
        const v = history.v[i];
        if (prevT >= 0 && t - prevT > gapBreakMs) {
          flush();
          bucketX = -1;
          if (pts.length) segments.push(pts.splice(0));
        }
        prevT = t;
        const x = Math.round(X(t));
        if (x !== bucketX) {
          flush();
          bucketX = x;
          bMin = bMax = bLast = v;
        } else {
          bMin = Math.min(bMin, v);
          bMax = Math.max(bMax, v);
          bLast = v;
        }
      }
      flush();
      // Extend the live head to "now" so the line meets the right edge smoothly.
      if (live && last != null && now - last < gapBreakMs && pts.length) {
        pts.push(X(now), pts[pts.length - 1]);
      }
      if (pts.length) segments.push(pts);

      const grad = ctx.createLinearGradient(0, PAD.t, 0, PAD.t + plotH);
      grad.addColorStop(0, withAlpha(colors.accent, 0.1));
      grad.addColorStop(1, withAlpha(colors.accent, 0));

      ctx.save();
      ctx.beginPath();
      ctx.rect(PAD.l, PAD.t - 4, plotW + 4, plotH + 8);
      ctx.clip();
      for (const seg of segments) {
        if (seg.length < 2) continue;
        const trace = () => {
          ctx!.moveTo(seg[0], seg[1]);
          const smooth = seg.length / 2 < plotW / 3;
          for (let i = 2; i < seg.length; i += 2) {
            if (smooth && i + 2 < seg.length) {
              const mx = (seg[i] + seg[i + 2]) / 2;
              const my = (seg[i + 1] + seg[i + 3]) / 2;
              ctx!.quadraticCurveTo(seg[i], seg[i + 1], mx, my);
            } else {
              ctx!.lineTo(seg[i], seg[i + 1]);
            }
          }
        };
        ctx.beginPath();
        trace();
        ctx.lineTo(seg[seg.length - 2], PAD.t + plotH);
        ctx.lineTo(seg[0], PAD.t + plotH);
        ctx.closePath();
        ctx.fillStyle = grad;
        ctx.fill();
        ctx.beginPath();
        trace();
        ctx.strokeStyle = colors.accent;
        ctx.lineWidth = 1.75;
        ctx.lineJoin = "round";
        ctx.lineCap = "round";
        ctx.stroke();
      }
      ctx.restore();

      // Live point: 6px dot with a slow pulse ring.
      const lastSeg = segments[segments.length - 1];
      if (live && lastSeg && lastSeg.length >= 2) {
        const hx = lastSeg[lastSeg.length - 2];
        const hy = lastSeg[lastSeg.length - 1];
        if (!reduced) {
          const p = (nowPerf % 1800) / 1800;
          ctx.beginPath();
          ctx.arc(hx, hy, 3 + p * 9, 0, Math.PI * 2);
          ctx.strokeStyle = withAlpha(colors.accent, 0.5 * (1 - p));
          ctx.lineWidth = 1.5;
          ctx.stroke();
        }
        ctx.beginPath();
        ctx.arc(hx, hy, 3, 0, Math.PI * 2);
        ctx.fillStyle = colors.accent;
        ctx.fill();
      }
      if (ext) {
        ariaLabel = `Heart rate graph, ${range === "session" ? "whole session" : range === "5m" ? "last 5 minutes" : "last 60 seconds"}, between ${Math.round(ext[0])} and ${Math.round(ext[1])} bpm`;
      }
    } else {
      ctx.fillStyle = colors.text;
      ctx.textAlign = "center";
      ctx.fillText("Waiting for heart rate data", PAD.l + plotW / 2, PAD.t + plotH / 2);
    }
    dirty = settling || (live && !reduced);
  }

  function frame(ts: number) {
    raf = 0;
    if (document.hidden) return;
    if (dirty) draw(ts);
    if (dirty) raf = requestAnimationFrame(frame);
  }

  function ensureLoop() {
    if (!raf && ctx && !document.hidden) raf = requestAnimationFrame(frame);
  }

  onMount(() => {
    ctx = canvas!.getContext("2d");
    readColors();
    resize();
    const ro = new ResizeObserver(() => {
      resize();
      ensureLoop();
    });
    ro.observe(canvas!.parentElement!);
    const mo = new MutationObserver(() => {
      readColors();
      ensureLoop();
    });
    mo.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme", "style"] });
    const onVis = () => {
      dirty = true;
      ensureLoop();
    };
    document.addEventListener("visibilitychange", onVis);
    void document.fonts?.ready.then(onVis);
    // Even when idle (not live), keep the time axis honest once a second.
    const slow = setInterval(() => {
      dirty = true;
      ensureLoop();
    }, 1000);
    ensureLoop();
    return () => {
      ro.disconnect();
      mo.disconnect();
      document.removeEventListener("visibilitychange", onVis);
      clearInterval(slow);
      if (raf) cancelAnimationFrame(raf);
    };
  });
</script>

<div class="graph" role="img" aria-label={ariaLabel}>
  <canvas bind:this={canvas} aria-hidden="true"></canvas>
</div>

<style>
  .graph {
    position: relative;
    width: 100%;
    height: 100%;
    min-height: 160px;
  }
  canvas {
    position: absolute;
    inset: 0;
    display: block;
  }
</style>

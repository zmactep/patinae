/**
 * ViewerCore — canvas lifecycle, WASM initialization, rAF loop, and event wiring.
 */

import type { WebViewer } from "../../../pkg/patinae_web.js";
import type {
  LabelInfo,
  OutputMessage,
  RenderMemoryProfileOption,
  ViewerPerformanceSnapshot,
  ViewerWasmPerformanceSnapshot,
} from "./types.js";
import { MOD_SHIFT, MOD_CTRL, MOD_ALT, MOD_META } from "./types.js";
import { LabelOverlay } from "./labels.js";

type WebViewerConstructorWithMemoryProfile = {
  create(
    canvas_id: string,
    picking_enabled: boolean,
    selection_overlay_enabled: boolean,
  ): Promise<WebViewer>;
  createWithMemoryProfile(
    canvas_id: string,
    picking_enabled: boolean,
    selection_overlay_enabled: boolean,
    memory_profile: string,
  ): Promise<WebViewer>;
};

type WebViewerWithMemoryWarnings = WebViewer & {
  takeWarnings?: () => { messages: OutputMessage[] };
};

/** Click-detection threshold: drag distance squared in CSS pixels. */
const CLICK_THRESHOLD_SQ = 25; // 5 CSS-px radius
/** `MouseEvent.button` indices we forward to the renderer. */
const MOUSE_BUTTONS = [0, 1, 2, 3, 4] as const; // left, middle, right, back, forward
const PERF_RING_CAPACITY = 240;

export class ViewerCore {
  private wasm: WebViewer | null = null;
  private canvas: HTMLCanvasElement;
  private animFrameId = 0;
  private resizeObserver: ResizeObserver | null = null;
  private labelOverlay: LabelOverlay;
  private _deferred = false;
  private _revealDuration = 150;
  private dpr = 1;
  /** `MouseEvent.buttons` bitmask the WASM side currently believes is pressed. */
  private buttonMask = 0;
  private clickStart: { x: number; y: number } | null = null;
  private fatalError: unknown | null = null;
  private fatalLabel: string | null = null;
  private queuedHover: { x: number; y: number } | null = null;
  private frameTimes: number[] = [];
  private lastFrameMs = 0;
  private frameCount = 0;
  private skipNextFrameTiming = false;

  /** Called with the raw WASM pick result (PickHitInfo | null) on each click. */
  onPick: ((hit: unknown | null) => void) | null = null;
  /** Called for renderer warnings produced outside direct command execution. */
  onOutput: ((message: OutputMessage) => void) | null = null;

  constructor(private container: HTMLElement) {
    // Ensure container is a positioning context for the label overlay
    const computedPos = getComputedStyle(container).position;
    if (computedPos === "static") {
      container.style.position = "relative";
    }

    this.canvas = document.createElement("canvas");
    this.canvas.id = "patinae-canvas-" + Math.random().toString(36).slice(2, 8);
    this.canvas.style.width = "100%";
    this.canvas.style.height = "100%";
    this.canvas.style.display = "block";
    this.canvas.tabIndex = 0; // focusable for keyboard events
    container.appendChild(this.canvas);

    this.labelOverlay = new LabelOverlay(container);
  }

  /**
   * Initialise the viewer.
   *
   * @param options.picking — whether to allocate hit-test picking readback
   *   resources (Rg32Uint half-res target + readbacks/reprojection).
   *   Read-only viewers should leave it `false` to save VRAM. Defaults to
   *   `false`. The choice is baked in at WASM construction; changing
   *   later requires re-creating the viewer.
   * @param options.selectionOverlay — whether to draw visible selection /
   *   hover overlays. Defaults to `options.picking`.
   * @param options.memoryProfile — optional renderer memory profile override:
   *   "performance", "balanced", "lite", "manual:<MiB>", or "auto".
   */
  async init(
    options: {
      picking?: boolean;
      selectionOverlay?: boolean;
      memoryProfile?: RenderMemoryProfileOption;
    } = {},
  ): Promise<WebViewer> {
    // Dynamically import the WASM module
    const wasmModule = await import("../../../pkg/patinae_web.js");
    await wasmModule.default(); // init wasm

    // Match canvas pixel size to layout size
    this.syncCanvasSize();

    const pickingEnabled = options.picking ?? false;
    const selectionOverlay = options.selectionOverlay ?? pickingEnabled;
    const memoryProfile = options.memoryProfile ?? "auto";
    const WebViewerCtor = wasmModule.WebViewer as WebViewerConstructorWithMemoryProfile;
    const viewer = memoryProfile === "auto"
      ? await WebViewerCtor.create(
        this.canvas.id,
        pickingEnabled,
        selectionOverlay,
      )
      : await WebViewerCtor.createWithMemoryProfile(
        this.canvas.id,
        pickingEnabled,
        selectionOverlay,
        memoryProfile,
      );
    this.wasm = viewer;
    this.fatalError = null;
    this.fatalLabel = null;
    this.syncCanvasSize();
    this.requireWasm("resize", (wasm) =>
      wasm.resize(this.canvas.width, this.canvas.height),
    );

    this.bindEvents();
    this.startLoop();

    return viewer;
  }

  get isFailed(): boolean {
    return this.fatalError !== null;
  }

  get lastError(): unknown | null {
    return this.fatalError;
  }

  /** Task lookup and validation failures belong to the request, not the renderer. */
  requestWasm<T>(label: string, fn: (wasm: WebViewer) => T): T {
    if (this.fatalError !== null) throw this.stoppedError(label);
    if (!this.wasm) throw new Error(`Patinae viewer is not initialized; cannot call ${label}`);
    return fn(this.wasm);
  }

  callWasm<T>(label: string, fn: (wasm: WebViewer) => T): T | undefined {
    if (this.fatalError !== null) return undefined;
    const wasm = this.wasm;
    if (!wasm) return undefined;

    try {
      return fn(wasm);
    } catch (error) {
      this.recordFatal(label, error);
      return undefined;
    }
  }

  requireWasm<T>(label: string, fn: (wasm: WebViewer) => T): T {
    if (this.fatalError !== null) {
      throw this.stoppedError(label);
    }
    const wasm = this.wasm;
    if (!wasm) {
      throw new Error(
        `Patinae viewer is not initialized; cannot call ${label}`,
      );
    }

    try {
      return fn(wasm);
    } catch (error) {
      this.recordFatal(label, error);
      throw error;
    }
  }

  queryWasm<T>(label: string, fallback: T, fn: (wasm: WebViewer) => T): T {
    if (!this.wasm && this.fatalError === null) return fallback;
    return this.requireWasm(label, fn);
  }

  get isDeferred(): boolean {
    return this._deferred;
  }

  setDeferred(defer: boolean, revealDuration = 150): void {
    this._deferred = defer;
    this._revealDuration = revealDuration;
    if (defer) {
      this.container.style.opacity = "0";
    }
  }

  reveal(): Promise<void> {
    if (!this._deferred) return Promise.resolve();
    this._deferred = false;

    return new Promise<void>((resolve) => {
      this.container.style.transition = `opacity ${this._revealDuration}ms ease-in`;
      // Force reflow so the transition triggers
      void this.container.offsetHeight;
      this.container.style.opacity = "1";

      let resolved = false;
      const done = () => {
        if (resolved) return;
        resolved = true;
        this.container.removeEventListener("transitionend", done);
        this.container.style.transition = "";
        resolve();
      };
      this.container.addEventListener("transitionend", done);
      // Fallback in case transitionend doesn't fire
      setTimeout(done, this._revealDuration + 50);
    });
  }

  destroy(): void {
    if (this.animFrameId) {
      cancelAnimationFrame(this.animFrameId);
      this.animFrameId = 0;
    }
    this.resizeObserver?.disconnect();
    this.labelOverlay.destroy();
    this.canvas.remove();
    const wasm = this.wasm;
    this.wasm = null;
    wasm?.free();
  }

  getPerformanceSnapshot(): ViewerPerformanceSnapshot {
    const avg =
      this.frameTimes.length > 0
        ? this.frameTimes.reduce((sum, ms) => sum + ms, 0) /
          this.frameTimes.length
        : 0;
    const wasm = (this.callWasm("get_performance_snapshot", (viewer) =>
      viewer.get_performance_snapshot(),
    ) ?? null) as ViewerWasmPerformanceSnapshot | null;
    return {
      frame_count: this.frameCount,
      avg_frame_ms: avg,
      median_frame_ms: percentile(this.frameTimes, 0.5),
      p95_frame_ms: percentile(this.frameTimes, 0.95),
      last_frame_ms: this.lastFrameMs,
      wasm,
    };
  }

  resetPerformanceStats(): void {
    this.frameTimes = [];
    this.lastFrameMs = 0;
    this.frameCount = 0;
    this.skipNextFrameTiming = true;
    this.callWasm("reset_performance_stats", (viewer) =>
      viewer.reset_performance_stats(),
    );
  }

  private recordFatal(label: string, error: unknown): void {
    if (this.fatalError !== null) return;

    this.fatalError = error;
    this.fatalLabel = label;
    this.clickStart = null;

    if (this.animFrameId) {
      cancelAnimationFrame(this.animFrameId);
      this.animFrameId = 0;
    }

    console.error(
      `[Patinae] WASM call failed in ${label}; viewer stopped.`,
      error,
    );
  }

  private stoppedError(label: string): Error {
    const firstLabel = this.fatalLabel ?? "unknown";
    const error = new Error(
      `Patinae viewer stopped after WASM call ${firstLabel} failed; cannot call ${label}`,
      { cause: this.fatalError },
    );
    return error;
  }

  // ---------------------------------------------------------------------------
  // Render loop
  // ---------------------------------------------------------------------------

  private startLoop(): void {
    let lastTime = performance.now();
    const loop = (now: number) => {
      this.animFrameId = requestAnimationFrame(loop);
      if (!this.wasm || this.isFailed) return;

      const dt = Math.min((now - lastTime) / 1000.0, 0.1);
      this.lastFrameMs = now - lastTime;
      if (this.skipNextFrameTiming) {
        this.skipNextFrameTiming = false;
      } else if (this.lastFrameMs > 0) {
        this.frameTimes.push(this.lastFrameMs);
        if (this.frameTimes.length > PERF_RING_CAPACITY) {
          this.frameTimes.shift();
        }
        this.frameCount++;
      }
      lastTime = now;

      this.callWasm("process_input", (wasm) => wasm.process_input());
      if (this.isFailed) return;
      this.callWasm("update_animations", (wasm) => wasm.update_animations(dt));
      if (this.isFailed) return;

      const hover = this.queuedHover;
      if (hover) {
        this.queuedHover = null;
        this.callWasm("process_hover", (wasm) =>
          wasm.process_hover(hover.x, hover.y),
        );
        if (this.isFailed) return;
      }

      // GPU-mode hover/click readbacks complete independently of visible
      // redraws. Poll every rAF so picks resolve even when the camera is idle.
      this.callWasm("poll_pending_picks", (wasm) => wasm.poll_pending_picks());
      if (this.isFailed) return;

      if (this.callWasm("needs_redraw", (wasm) => wasm.needs_redraw())) {
        this.callWasm("render_frame", (wasm) => wasm.render_frame());
        if (this.isFailed) return;
        this.drainRendererWarnings();
        if (this.isFailed) return;
        const labels = this.callWasm(
          "get_labels",
          (wasm) => wasm.get_labels() as LabelInfo[] | null,
        );
        if (this.isFailed) return;
        this.labelOverlay.update(labels ?? [], window.devicePixelRatio || 1);
      }

      // GPU-mode click picks complete asynchronously. Drain the latest
      // result here and forward it to onPick exactly like the CPU path.
      const pickedAsync = this.callWasm("take_completed_pick", (wasm) =>
        wasm.take_completed_pick(),
      );
      if (this.isFailed) return;
      if (pickedAsync !== undefined) {
        this.onPick?.(pickedAsync ?? null);
      }
    };
    this.animFrameId = requestAnimationFrame(loop);
  }

  private drainRendererWarnings(): void {
    const output = this.callWasm("takeWarnings", (wasm) => {
      const takeWarnings = (wasm as WebViewerWithMemoryWarnings).takeWarnings;
      return takeWarnings ? takeWarnings.call(wasm) : null;
    });
    if (!output) return;
    for (const message of output.messages ?? []) {
      this.onOutput?.(message);
    }
  }

  // ---------------------------------------------------------------------------
  // Event binding
  // ---------------------------------------------------------------------------

  /**
   * Release any button the renderer still believes is pressed but that the
   * event's `buttons` bitmask says is not.
   *
   * Pointer capture covers the common "released outside the canvas" case, but
   * capture can be denied or silently dropped (another element captured first,
   * OS-level focus loss, a native drag starting). Without this reconciliation
   * the renderer keeps the button latched and the camera stays stuck in a drag
   * on the next mouse move.
   */
  private reconcileButtons(e: PointerEvent): void {
    for (const button of MOUSE_BUTTONS) {
      const bit = buttonBit(button);
      if (!(this.buttonMask & bit) || e.buttons & bit) continue;
      this.buttonMask &= ~bit;
      this.callWasm("on_mouse_up", (wasm) =>
        wasm.on_mouse_up(e.offsetX, e.offsetY, button),
      );
      if (button === 0) this.clickStart = null;
    }
  }

  private bindEvents(): void {
    const c = this.canvas;
    this.dpr = window.devicePixelRatio || 1;

    c.addEventListener("pointerdown", (e) => {
      e.preventDefault();
      c.focus();
      // Capture the pointer so a drag that leaves the canvas keeps delivering
      // move/up events here instead of being swallowed by the document.
      try {
        c.setPointerCapture(e.pointerId);
      } catch {
        // Capture is best-effort; reconcileButtons() is the fallback.
      }
      this.reconcileButtons(e);
      this.buttonMask |= buttonBit(e.button);
      this.callWasm("on_mouse_down", (wasm) =>
        wasm.on_mouse_down(e.offsetX, e.offsetY, e.button, modBits(e)),
      );
      if (this.isFailed) return;
      // Track potential click start for left button only.
      if (e.button === 0) {
        this.clickStart = { x: e.offsetX, y: e.offsetY };
      }
    });

    c.addEventListener("pointermove", (e) => {
      this.reconcileButtons(e);
      this.callWasm("on_mouse_move", (wasm) =>
        wasm.on_mouse_move(e.offsetX, e.offsetY, modBits(e)),
      );
      if (this.isFailed) return;
      this.queuedHover = {
        x: e.offsetX * this.dpr,
        y: e.offsetY * this.dpr,
      };
    });

    // Re-entering the canvas is the first chance to notice a button that was
    // released while the cursor was elsewhere.
    c.addEventListener("pointerenter", (e) => this.reconcileButtons(e));

    c.addEventListener("pointerup", (e) => {
      if (c.hasPointerCapture(e.pointerId)) {
        c.releasePointerCapture(e.pointerId);
      }
      this.buttonMask &= ~buttonBit(e.button);
      this.callWasm("on_mouse_up", (wasm) =>
        wasm.on_mouse_up(e.offsetX, e.offsetY, e.button),
      );
      if (this.isFailed) return;
      // Detect click (left button, short drag).
      if (e.button === 0 && this.clickStart !== null) {
        const dx = e.offsetX - this.clickStart.x;
        const dy = e.offsetY - this.clickStart.y;
        if (dx * dx + dy * dy < CLICK_THRESHOLD_SQ) {
          this.callWasm("pick_at_screen", (wasm) =>
            wasm.pick_at_screen(e.offsetX * this.dpr, e.offsetY * this.dpr),
          );
          if (this.isFailed) return;
        }
        this.clickStart = null;
      }
    });

    // The browser took the pointer away (touch gesture, OS-level drag, ...):
    // no pointerup is coming, so release everything we still hold.
    c.addEventListener("pointercancel", (e) => {
      if (c.hasPointerCapture(e.pointerId)) {
        c.releasePointerCapture(e.pointerId);
      }
      for (const button of MOUSE_BUTTONS) {
        if (!(this.buttonMask & buttonBit(button))) continue;
        this.callWasm("on_mouse_up", (wasm) =>
          wasm.on_mouse_up(e.offsetX, e.offsetY, button),
        );
      }
      this.buttonMask = 0;
      this.clickStart = null;
      this.queuedHover = { x: -1, y: -1 };
    });

    c.addEventListener("mouseleave", () => {
      this.clickStart = null;
      this.queuedHover = { x: -1, y: -1 }; // guaranteed miss clears hover indicators
    });

    c.addEventListener(
      "wheel",
      (e) => {
        e.preventDefault();
        this.callWasm("on_wheel", (wasm) =>
          wasm.on_wheel(e.deltaY, modBits(e)),
        );
      },
      { passive: false },
    );

    c.addEventListener("contextmenu", (e) => e.preventDefault());

    // Resize
    this.resizeObserver = new ResizeObserver(() => {
      this.dpr = window.devicePixelRatio || 1;
      this.syncCanvasSize();
      this.callWasm("resize", (wasm) =>
        wasm.resize(this.canvas.width, this.canvas.height),
      );
    });
    this.resizeObserver.observe(this.container);
  }

  private syncCanvasSize(): void {
    const dpr = window.devicePixelRatio || 1;
    const rect = this.canvas.getBoundingClientRect();
    this.canvas.width = Math.round(rect.width * dpr);
    this.canvas.height = Math.round(rect.height * dpr);
  }
}

/** `MouseEvent.buttons` bit corresponding to a `MouseEvent.button` index. */
function buttonBit(button: number): number {
  switch (button) {
    case 0:
      return 1; // left
    case 1:
      return 4; // middle
    case 2:
      return 2; // right
    default:
      return 0; // back/forward and friends are not forwarded
  }
}

/** Convert MouseEvent/WheelEvent modifiers to bitmask. */
function modBits(e: MouseEvent | WheelEvent): number {
  let bits = 0;
  if (e.shiftKey) bits |= MOD_SHIFT;
  if (e.ctrlKey) bits |= MOD_CTRL;
  if (e.altKey) bits |= MOD_ALT;
  if (e.metaKey) bits |= MOD_META;
  return bits;
}

function percentile(values: number[], p: number): number {
  if (values.length === 0) return 0;
  const sorted = [...values].sort((a, b) => a - b);
  const idx = Math.round((sorted.length - 1) * Math.min(Math.max(p, 0), 1));
  return sorted[idx] ?? 0;
}

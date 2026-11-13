var ur = Object.defineProperty;
var hr = (r, e, t) => e in r ? ur(r, e, { enumerable: !0, configurable: !0, writable: !0, value: t }) : r[e] = t;
var k = (r, e, t) => hr(r, typeof e != "symbol" ? e + "" : e, t);
class pr {
  constructor(e) {
    k(this, "container");
    k(this, "pool", []);
    k(this, "activeCount", 0);
    k(this, "highlights", []);
    this.container = document.createElement("div"), this.container.style.position = "absolute", this.container.style.top = "0", this.container.style.left = "0", this.container.style.width = "100%", this.container.style.height = "100%", this.container.style.pointerEvents = "none", this.container.style.overflow = "hidden", e.appendChild(this.container);
  }
  update(e, t) {
    for (; this.pool.length < e.length; ) {
      const n = document.createElement("span");
      n.style.position = "absolute", n.style.whiteSpace = "nowrap", n.style.fontFamily = "-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif", n.style.textShadow = "0 0 3px rgba(0,0,0,0.8), 0 0 6px rgba(0,0,0,0.5)", n.style.display = "none", this.container.appendChild(n), this.pool.push(n);
    }
    for (let n = 0; n < e.length; n++) {
      const s = e[n], a = this.pool[n], c = s.x / t, u = s.y / t;
      a.style.left = `${c}px`, a.style.top = `${u}px`, a.textContent = s.text, a.style.fontSize = `${s.size}px`, a.style.transform = `translate(${-s.anchor_x * 100}%, ${-s.anchor_y * 100}%)`, a.style.color = fr(s.color), a.dataset.alignment = s.alignment, a.dataset.ownerId = String(s.owner_id), a.dataset.insertionOrdinal = String(s.insertion_ordinal), a.dataset.displayOrder = String(s.display_order), a.style.display = "";
    }
    for (let n = e.length; n < this.activeCount; n++)
      this.pool[n].style.display = "none";
    return this.activeCount = e.length, e.map((n, s) => {
      const a = this.pool[s].getBoundingClientRect();
      return {
        owner_id: n.owner_id,
        insertion_ordinal: n.insertion_ordinal,
        width: a.width,
        height: a.height
      };
    });
  }
  highlight(e) {
    for (let t = 0; t < this.activeCount; t++) {
      const n = this.pool[t], s = e[t] ?? 0;
      this.highlights[t] !== s && (this.highlights[t] = s, n.style.outline = s === 2 ? "2px solid #67e8f9" : s === 1 ? "1px solid #67e8f980" : "none", n.style.outlineOffset = "2px", n.style.borderRadius = "3px", n.style.background = s === 2 ? "#164e6380" : s === 1 ? "#164e6330" : "transparent");
    }
  }
  destroy() {
    this.container.remove();
  }
}
function fr(r) {
  const e = Math.round(at(r[0]) * 255), t = Math.round(at(r[1]) * 255), n = Math.round(at(r[2]) * 255), s = at(r[3]);
  return `rgba(${e}, ${t}, ${n}, ${s})`;
}
function at(r) {
  return Math.min(1, Math.max(0, r));
}
const dr = 25, mr = [0, 1, 2, 3, 4], gr = 240;
class br {
  constructor(e) {
    k(this, "wasm", null);
    k(this, "canvas");
    k(this, "animFrameId", 0);
    k(this, "resizeObserver", null);
    k(this, "labelOverlay");
    k(this, "_deferred", !1);
    k(this, "_revealDuration", 150);
    k(this, "dpr", 1);
    k(this, "eventAbort", new AbortController());
    k(this, "macos", /Mac|iPhone|iPad/.test(navigator.platform));
    /** `MouseEvent.buttons` bitmask the WASM side currently believes is pressed. */
    k(this, "buttonMask", 0);
    k(this, "clickStart", null);
    k(this, "fatalError", null);
    k(this, "fatalLabel", null);
    k(this, "queuedHover", null);
    k(this, "frameTimes", []);
    k(this, "lastFrameMs", 0);
    k(this, "frameCount", 0);
    k(this, "skipNextFrameTiming", !1);
    /** Called with the raw WASM pick result (PickHitInfo | null) on each click. */
    k(this, "onPick", null);
    /** Called for renderer warnings produced outside direct command execution. */
    k(this, "onOutput", null);
    this.container = e, getComputedStyle(e).position === "static" && (e.style.position = "relative"), this.canvas = document.createElement("canvas"), this.canvas.id = "patinae-canvas-" + Math.random().toString(36).slice(2, 8), this.canvas.style.width = "100%", this.canvas.style.height = "100%", this.canvas.style.display = "block", this.canvas.tabIndex = 0, e.appendChild(this.canvas), this.labelOverlay = new pr(e);
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
  async init(e = {}) {
    const t = await import("./patinae_web-DmhO40xJ.js");
    await t.default(), this.syncCanvasSize();
    const n = e.picking ?? !1, s = e.selectionOverlay ?? n, a = e.memoryProfile ?? "auto", c = t.WebViewer, u = a === "auto" ? await c.create(
      this.canvas.id,
      n,
      s
    ) : await c.createWithMemoryProfile(
      this.canvas.id,
      n,
      s,
      a
    );
    return this.wasm = u, this.fatalError = null, this.fatalLabel = null, this.syncCanvasSize(), this.requireWasm(
      "resize",
      (o) => o.resize(this.canvas.width, this.canvas.height)
    ), this.bindEvents(), this.startLoop(), u;
  }
  get isFailed() {
    return this.fatalError !== null;
  }
  get lastError() {
    return this.fatalError;
  }
  /** Task lookup and validation failures belong to the request, not the renderer. */
  requestWasm(e, t) {
    if (this.fatalError !== null) throw this.stoppedError(e);
    if (!this.wasm) throw new Error(`Patinae viewer is not initialized; cannot call ${e}`);
    return t(this.wasm);
  }
  callWasm(e, t) {
    if (this.fatalError !== null) return;
    const n = this.wasm;
    if (n)
      try {
        return t(n);
      } catch (s) {
        this.recordFatal(e, s);
        return;
      }
  }
  requireWasm(e, t) {
    if (this.fatalError !== null)
      throw this.stoppedError(e);
    const n = this.wasm;
    if (!n)
      throw new Error(
        `Patinae viewer is not initialized; cannot call ${e}`
      );
    try {
      return t(n);
    } catch (s) {
      throw this.recordFatal(e, s), s;
    }
  }
  queryWasm(e, t, n) {
    return !this.wasm && this.fatalError === null ? t : this.requireWasm(e, n);
  }
  get isDeferred() {
    return this._deferred;
  }
  setDeferred(e, t = 150) {
    this._deferred = e, this._revealDuration = t, e && (this.container.style.opacity = "0");
  }
  reveal() {
    return this._deferred ? (this._deferred = !1, new Promise((e) => {
      this.container.style.transition = `opacity ${this._revealDuration}ms ease-in`, this.container.offsetHeight, this.container.style.opacity = "1";
      let t = !1;
      const n = () => {
        t || (t = !0, this.container.removeEventListener("transitionend", n), this.container.style.transition = "", e());
      };
      this.container.addEventListener("transitionend", n), setTimeout(n, this._revealDuration + 50);
    })) : Promise.resolve();
  }
  destroy() {
    var t;
    this.animFrameId && (cancelAnimationFrame(this.animFrameId), this.animFrameId = 0), this.eventAbort.abort(), (t = this.resizeObserver) == null || t.disconnect(), this.labelOverlay.destroy(), this.canvas.remove();
    const e = this.wasm;
    this.wasm = null, e == null || e.free();
  }
  getPerformanceSnapshot() {
    const e = this.frameTimes.length > 0 ? this.frameTimes.reduce((n, s) => n + s, 0) / this.frameTimes.length : 0, t = this.callWasm(
      "get_performance_snapshot",
      (n) => n.get_performance_snapshot()
    ) ?? null;
    return {
      frame_count: this.frameCount,
      avg_frame_ms: e,
      median_frame_ms: qn(this.frameTimes, 0.5),
      p95_frame_ms: qn(this.frameTimes, 0.95),
      last_frame_ms: this.lastFrameMs,
      wasm: t
    };
  }
  resetPerformanceStats() {
    this.frameTimes = [], this.lastFrameMs = 0, this.frameCount = 0, this.skipNextFrameTiming = !0, this.callWasm(
      "reset_performance_stats",
      (e) => e.reset_performance_stats()
    );
  }
  recordFatal(e, t) {
    this.fatalError === null && (this.fatalError = t, this.fatalLabel = e, this.clickStart = null, this.animFrameId && (cancelAnimationFrame(this.animFrameId), this.animFrameId = 0), console.error(
      `[Patinae] WASM call failed in ${e}; viewer stopped.`,
      t
    ));
  }
  stoppedError(e) {
    const t = this.fatalLabel ?? "unknown";
    return new Error(
      `Patinae viewer stopped after WASM call ${t} failed; cannot call ${e}`,
      { cause: this.fatalError }
    );
  }
  // ---------------------------------------------------------------------------
  // Render loop
  // ---------------------------------------------------------------------------
  startLoop() {
    let e = performance.now();
    const t = (n) => {
      var u;
      if (this.animFrameId = requestAnimationFrame(t), !this.wasm || this.isFailed) return;
      this.dpr !== (window.devicePixelRatio || 1) && this.resizeViewport();
      const s = Math.min((n - e) / 1e3, 0.1);
      if (this.lastFrameMs = n - e, this.skipNextFrameTiming ? this.skipNextFrameTiming = !1 : this.lastFrameMs > 0 && (this.frameTimes.push(this.lastFrameMs), this.frameTimes.length > gr && this.frameTimes.shift(), this.frameCount++), e = n, this.callWasm("process_input", (o) => o.process_input()), this.isFailed || (this.callWasm("update_animations", (o) => o.update_animations(s)), this.isFailed)) return;
      const a = this.queuedHover;
      if (a && (this.queuedHover = null, this.callWasm(
        "process_hover",
        (o) => o.process_hover(a.x, a.y)
      ), this.isFailed) || (this.callWasm("poll_pending_picks", (o) => o.poll_pending_picks()), this.isFailed)) return;
      if (this.callWasm("needs_redraw", (o) => o.needs_redraw())) {
        if (this.callWasm("render_frame", (f) => f.render_frame()), this.isFailed || (this.drainRendererWarnings(), this.isFailed)) return;
        const o = this.callWasm(
          "get_labels",
          (f) => f.get_labels()
        );
        if (this.isFailed) return;
        const g = this.labelOverlay.update(o ?? [], this.dpr);
        this.callWasm("set_label_bounds", (f) => f.set_label_bounds(g));
      }
      this.updateManipulationAffordance();
      const c = this.callWasm(
        "take_completed_pick",
        (o) => o.take_completed_pick()
      );
      this.isFailed || c !== void 0 && ((u = this.onPick) == null || u.call(this, c ?? null));
    };
    this.animFrameId = requestAnimationFrame(t);
  }
  drainRendererWarnings() {
    var t;
    const e = this.callWasm("takeWarnings", (n) => {
      const s = n.takeWarnings;
      return s ? s.call(n) : null;
    });
    if (e)
      for (const n of e.messages ?? [])
        (t = this.onOutput) == null || t.call(this, n);
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
  reconcileButtons(e) {
    for (const t of mr) {
      const n = Ut(t);
      if (!(!(this.buttonMask & n) || e.buttons & n)) {
        if (t === 0) {
          this.cancelPointer();
          return;
        }
        this.buttonMask &= ~n, this.callWasm(
          "on_mouse_up",
          (s) => s.on_mouse_up(e.offsetX, e.offsetY, t)
        );
      }
    }
  }
  updateManipulationAffordance() {
    const e = this.callWasm(
      "label_highlights",
      (t) => t.label_highlights()
    );
    this.labelOverlay.highlight(e ?? []), this.canvas.style.cursor = this.callWasm(
      "manipulation_cursor",
      (t) => t.manipulation_cursor()
    ) ?? "default";
  }
  cancelPointer() {
    this.callWasm("cancel_pointer", (e) => e.cancel_pointer()), this.buttonMask = 0, this.clickStart = null, this.queuedHover = { x: -1, y: -1 }, this.updateManipulationAffordance();
  }
  bindEvents() {
    const e = this.canvas;
    this.dpr = window.devicePixelRatio || 1, this.callWasm(
      "configure_interaction",
      (s) => s.configure_interaction(this.macos, this.dpr)
    );
    const t = this.eventAbort.signal, n = (s) => {
      if (s.key === "Escape") {
        this.cancelPointer();
        return;
      }
      this.callWasm("on_modifiers", (a) => a.on_modifiers(je(s))), this.updateManipulationAffordance();
    };
    window.addEventListener("keydown", n, { signal: t }), window.addEventListener("keyup", n, { signal: t }), window.addEventListener("blur", () => this.cancelPointer(), { signal: t }), document.addEventListener(
      "visibilitychange",
      () => {
        document.hidden && this.cancelPointer();
      },
      { signal: t }
    ), e.addEventListener("pointerdown", (s) => {
      s.preventDefault(), e.focus();
      try {
        e.setPointerCapture(s.pointerId);
      } catch {
      }
      this.reconcileButtons(s), this.buttonMask |= Ut(s.button), this.callWasm(
        "on_mouse_down",
        (a) => a.on_mouse_down(s.offsetX, s.offsetY, s.button, je(s))
      ), !this.isFailed && s.button === 0 && (this.clickStart = { x: s.offsetX, y: s.offsetY });
    }), e.addEventListener("pointermove", (s) => {
      this.reconcileButtons(s), this.callWasm(
        "on_mouse_move",
        (a) => a.on_mouse_move(s.offsetX, s.offsetY, je(s))
      ), !this.isFailed && (this.queuedHover = {
        x: s.offsetX * this.dpr,
        y: s.offsetY * this.dpr
      });
    }), e.addEventListener("pointerenter", (s) => this.reconcileButtons(s)), e.addEventListener("pointerup", (s) => {
      this.buttonMask &= ~Ut(s.button), e.hasPointerCapture(s.pointerId) && e.releasePointerCapture(s.pointerId), this.callWasm("on_modifiers", (c) => c.on_modifiers(je(s)));
      const a = this.callWasm(
        "on_mouse_up",
        (c) => c.on_mouse_up(s.offsetX, s.offsetY, s.button)
      );
      if (!this.isFailed) {
        if (!a && s.button === 0 && this.clickStart !== null) {
          const c = s.offsetX - this.clickStart.x, u = s.offsetY - this.clickStart.y;
          if (c * c + u * u < dr && (this.callWasm(
            "pick_at_screen",
            (o) => o.pick_at_screen(s.offsetX * this.dpr, s.offsetY * this.dpr)
          ), this.isFailed))
            return;
        }
        s.button === 0 && (this.clickStart = null);
      }
    }), e.addEventListener("pointercancel", () => this.cancelPointer()), e.addEventListener("lostpointercapture", () => {
      this.buttonMask !== 0 && this.cancelPointer();
    }), e.addEventListener("mouseleave", () => {
      this.callWasm("leave_pointer", (s) => s.leave_pointer()), this.updateManipulationAffordance(), this.clickStart = null, this.queuedHover = { x: -1, y: -1 };
    }), e.addEventListener(
      "wheel",
      (s) => {
        s.preventDefault(), this.callWasm(
          "on_wheel",
          (a) => a.on_wheel(s.deltaY, je(s))
        );
      },
      { passive: !1 }
    ), e.addEventListener("contextmenu", (s) => s.preventDefault()), this.resizeObserver = new ResizeObserver(() => this.resizeViewport()), this.resizeObserver.observe(this.container);
  }
  resizeViewport() {
    this.cancelPointer(), this.dpr = window.devicePixelRatio || 1, this.callWasm(
      "configure_interaction",
      (e) => e.configure_interaction(this.macos, this.dpr)
    ), this.syncCanvasSize(), this.callWasm(
      "resize",
      (e) => e.resize(this.canvas.width, this.canvas.height)
    );
  }
  syncCanvasSize() {
    const e = window.devicePixelRatio || 1, t = this.canvas.getBoundingClientRect();
    this.canvas.width = Math.round(t.width * e), this.canvas.height = Math.round(t.height * e);
  }
}
function Ut(r) {
  switch (r) {
    case 0:
      return 1;
    // left
    case 1:
      return 4;
    // middle
    case 2:
      return 2;
    // right
    default:
      return 0;
  }
}
function je(r) {
  let e = 0;
  return r.shiftKey && (e |= 1), r.ctrlKey && (e |= 2), r.altKey && (e |= 4), r.metaKey && (e |= 8), e;
}
function qn(r, e) {
  if (r.length === 0) return 0;
  const t = [...r].sort((s, a) => s - a), n = Math.round((t.length - 1) * Math.min(Math.max(e, 0), 1));
  return t[n] ?? 0;
}
class kr {
  constructor() {
    k(this, "listeners", /* @__PURE__ */ new Map());
  }
  on(e, t) {
    this.listeners.has(e) || this.listeners.set(e, /* @__PURE__ */ new Set()), this.listeners.get(e).add(t);
  }
  off(e, t) {
    var n;
    (n = this.listeners.get(e)) == null || n.delete(t);
  }
  emit(e, t) {
    for (const n of this.listeners.get(e) ?? [])
      try {
        n(t);
      } catch (s) {
        console.error(`Event handler error (${e}):`, s);
      }
  }
}
function nn() {
  return { async: !1, breaks: !1, extensions: null, gfm: !0, hooks: null, pedantic: !1, renderer: null, silent: !1, tokenizer: null, walkTokens: null };
}
var Te = nn();
function gs(r) {
  Te = r;
}
var ye = { exec: () => null };
function Pe(r) {
  let e = [];
  return (t) => {
    let n = Math.max(0, Math.min(3, t - 1)), s = e[n];
    return s || (s = r(n), e[n] = s), s;
  };
}
function S(r, e = "") {
  let t = typeof r == "string" ? r : r.source, n = { replace: (s, a) => {
    let c = typeof a == "string" ? a : a.source;
    return c = c.replace(U.caret, "$1"), t = t.replace(s, c), n;
  }, getRegex: () => new RegExp(t, e) };
  return n;
}
var _r = ((r = "") => {
  try {
    return !!new RegExp("(?<=1)(?<!1)" + r);
  } catch {
    return !1;
  }
})(), U = { codeRemoveIndent: /^(?: {1,4}| {0,3}\t)/gm, outputLinkReplace: /\\([\[\]])/g, indentCodeCompensation: /^(\s+)(?:```)/, beginningSpace: /^\s+/, endingHash: /#$/, startingSpaceChar: /^ /, endingSpaceChar: / $/, nonSpaceChar: /[^ ]/, newLineCharGlobal: /\n/g, tabCharGlobal: /\t/g, multipleSpaceGlobal: /\s+/g, blankLine: /^[ \t]*$/, doubleBlankLine: /\n[ \t]*\n[ \t]*$/, blockquoteStart: /^ {0,3}>/, blockquoteSetextReplace: /\n {0,3}((?:=+|-+) *)(?=\n|$)/g, blockquoteSetextReplace2: /^ {0,3}>[ \t]?/gm, listReplaceNesting: /^ {1,4}(?=( {4})*[^ ])/g, listIsTask: /^\[[ xX]\] +\S/, listReplaceTask: /^\[[ xX]\] +/, listTaskCheckbox: /\[[ xX]\]/, anyLine: /\n.*\n/, hrefBrackets: /^<(.*)>$/, tableDelimiter: /[:|]/, tableAlignChars: /^\||\| *$/g, tableRowBlankLine: /\n[ \t]*$/, tableAlignRight: /^ *-+: *$/, tableAlignCenter: /^ *:-+: *$/, tableAlignLeft: /^ *:-+ *$/, startATag: /^<a /i, endATag: /^<\/a>/i, startPreScriptTag: /^<(pre|code|kbd|script)(\s|>)/i, endPreScriptTag: /^<\/(pre|code|kbd|script)(\s|>)/i, startAngleBracket: /^</, endAngleBracket: />$/, pedanticHrefTitle: /^([^'"]*[^\s])\s+(['"])(.*)\2/, unicodeAlphaNumeric: /[\p{L}\p{N}]/u, escapeTest: /[&<>"']/, escapeReplace: /[&<>"']/g, escapeTestNoEncode: /[<>"']|&(?!(#\d{1,7}|#[Xx][a-fA-F0-9]{1,6}|\w+);)/, escapeReplaceNoEncode: /[<>"']|&(?!(#\d{1,7}|#[Xx][a-fA-F0-9]{1,6}|\w+);)/g, caret: /(^|[^\[])\^/g, percentDecode: /%25/g, findPipe: /\|/g, splitPipe: / \|/, slashPipe: /\\\|/g, carriageReturn: /\r\n|\r/g, spaceLine: /^ +$/gm, notSpaceStart: /^\S*/, endingNewline: /\n$/, listItemRegex: (r) => new RegExp(`^( {0,3}${r})((?:[	 ][^\\n]*)?(?:\\n|$))`), nextBulletRegex: Pe((r) => new RegExp(`^ {0,${r}}(?:[*+-]|\\d{1,9}[.)])((?:[ 	][^\\n]*)?(?:\\n|$))`)), hrRegex: Pe((r) => new RegExp(`^ {0,${r}}((?:- *){3,}|(?:_ *){3,}|(?:\\* *){3,})(?:\\n+|$)`)), fencesBeginRegex: Pe((r) => new RegExp(`^ {0,${r}}(?:\`\`\`|~~~)`)), headingBeginRegex: Pe((r) => new RegExp(`^ {0,${r}}#`)), htmlBeginRegex: Pe((r) => new RegExp(`^ {0,${r}}<(?:[a-z].*>|!--)`, "i")), blockquoteBeginRegex: Pe((r) => new RegExp(`^ {0,${r}}>`)) }, wr = /^(?:[ \t]*(?:\n|$))+/, yr = /^((?: {4}| {0,3}\t)[^\n]+(?:\n(?:[ \t]*(?:\n|$))*)?)+/, xr = /^ {0,3}(`{3,}(?=[^`\n]*(?:\n|$))|~{3,})([^\n]*)(?:\n|$)(?:|([\s\S]*?)(?:\n|$))(?: {0,3}\1[~`]* *(?=\n|$)|$)/, Xe = /^ {0,3}((?:-[\t ]*){3,}|(?:_[ \t]*){3,}|(?:\*[ \t]*){3,})(?:\n+|$)/, Tr = /^ {0,3}(#{1,6})(?=\s|$)(.*)(?:\n+|$)/, sn = / {0,3}(?:[*+-]|\d{1,9}[.)])/, bs = /^(?!bull |blockCode|fences|blockquote|heading|html|table)((?:.|\n(?!\s*?\n|bull |blockCode|fences|blockquote|heading|html|table))+?)\n {0,3}(=+|-+) *(?:\n+|$)/, ks = S(bs).replace(/bull/g, sn).replace(/blockCode/g, /(?: {4}| {0,3}\t)/).replace(/fences/g, / {0,3}(?:`{3,}|~{3,})/).replace(/blockquote/g, / {0,3}>/).replace(/heading/g, / {0,3}#{1,6}(?:\s|$)/).replace(/html/g, / {0,3}<[^\n>]+>\n/).replace(/\|table/g, "").getRegex(), Sr = S(bs).replace(/bull/g, sn).replace(/blockCode/g, /(?: {4}| {0,3}\t)/).replace(/fences/g, / {0,3}(?:`{3,}|~{3,})/).replace(/blockquote/g, / {0,3}>/).replace(/heading/g, / {0,3}#{1,6}(?:\s|$)/).replace(/html/g, / {0,3}<[^\n>]+>\n/).replace(/table/g, / {0,3}\|?(?:[:\- ]*\|)+[\:\- ]*\n/).getRegex(), rn = /^([^\n]+(?:\n(?!hr|heading|lheading|blockquote|fences|list|html|table|[ \t]+\n)[^\n]+)*)/, Er = /^[^\n]+/, an = /(?!\s*\])(?:\\[\s\S]|[^\[\]\\])+/, vr = S(/^ {0,3}\[(label)\]: *(?:\n[ \t]*)?([^<\s][^\s]*|<.*?>)(?:(?: +(?:\n[ \t]*)?| *\n[ \t]*)(title))? *(?:\n+|$)/).replace("label", an).replace("title", /(?:"(?:\\"?|[^"\\])*"|'[^'\n]*(?:\n[^'\n]+)*\n?'|\([^()]*\))/).getRegex(), Ar = S(/^(bull)([ \t][^\n]*?)?(?:\n|$)/).replace(/bull/g, sn).getRegex(), ft = "address|article|aside|base|basefont|blockquote|body|caption|center|col|colgroup|dd|details|dialog|dir|div|dl|dt|fieldset|figcaption|figure|footer|form|frame|frameset|h[1-6]|head|header|hr|html|iframe|legend|li|link|main|menu|menuitem|meta|nav|noframes|ol|optgroup|option|p|param|search|section|summary|table|tbody|td|tfoot|th|thead|title|tr|track|ul", on = /<!--(?:-?>|[\s\S]*?(?:-->|$))/, Rr = S("^ {0,3}(?:<(script|pre|style|textarea)[\\s>][\\s\\S]*?(?:</\\1>[^\\n]*\\n*|$)|comment[^\\n]*(\\n+|$)|<\\?[\\s\\S]*?(?:\\?>[^\\n]*\\n*|$)|<![A-Z][\\s\\S]*?(?:>[^\\n]*\\n*|$)|<!\\[CDATA\\[[\\s\\S]*?(?:\\]\\]>[^\\n]*\\n*|$)|</?(tag)(?: +|\\n|/?>)[\\s\\S]*?(?:(?:\\n[ 	]*)+\\n|$)|<(?!script|pre|style|textarea)([a-z][\\w-]*)(?:attribute)*? */?>(?=[ \\t]*(?:\\n|$))[\\s\\S]*?(?:(?:\\n[ 	]*)+\\n|$)|</(?!script|pre|style|textarea)[a-z][\\w-]*\\s*>(?=[ \\t]*(?:\\n|$))[\\s\\S]*?(?:(?:\\n[ 	]*)+\\n|$))", "i").replace("comment", on).replace("tag", ft).replace("attribute", / +[a-zA-Z:_][\w.:-]*(?: *= *"[^"\n]*"| *= *'[^'\n]*'| *= *[^\s"'=<>`]+)?/).getRegex(), _s = (r) => S(rn).replace("hr", Xe).replace("heading", " {0,3}#{1,6}(?:\\s|$)").replace("|lheading", "").replace("|table", "").replace("blockquote", " {0,3}>").replace("fences", " {0,3}(?:`{3,}(?=[^`\\n]*\\n)|~~~)[^\\n]*\\n").replace("list", r).replace("html", "</?(?:tag)(?: +|\\n|/?>)|<(?:script|pre|style|textarea|!--)").replace("tag", ft).getRegex(), Lr = _s(/ {0,3}(?:[*+-]|1[.)])[ \t]+[^ \t\n]/), Or = _s(/ {0,3}(?:[*+-]|\d{1,9}[.)])(?:[ \t]|\n|$)/), Cr = S(/^( {0,3}> ?(paragraph|[^\n]*)(?:\n|$))+/).replace("paragraph", Or).getRegex(), ln = { blockquote: Cr, code: yr, def: vr, fences: xr, heading: Tr, hr: Xe, html: Rr, lheading: ks, list: Ar, newline: wr, paragraph: Lr, table: ye, text: Er }, Gn = S("^ *([^\\n ].*)\\n {0,3}((?:\\| *)?:?-+:? *(?:\\| *:?-+:? *)*(?:\\| *)?)(?:\\n((?:(?! *\\n|hr|heading|blockquote|code|fences|list|html).*(?:\\n|$))*)\\n*|$)").replace("hr", Xe).replace("heading", " {0,3}#{1,6}(?:\\s|$)").replace("blockquote", " {0,3}>").replace("code", "(?: {4}| {0,3}	)[^\\n]").replace("fences", " {0,3}(?:`{3,}(?=[^`\\n]*\\n)|~~~)[^\\n]*\\n").replace("list", " {0,3}(?:[*+-]|1[.)])[ \\t]").replace("html", "</?(?:tag)(?: +|\\n|/?>)|<(?:script|pre|style|textarea|!--)").replace("tag", ft).getRegex(), Pr = { ...ln, lheading: Sr, table: Gn, paragraph: S(rn).replace("hr", Xe).replace("heading", " {0,3}#{1,6}(?:\\s|$)").replace("|lheading", "").replace("table", Gn).replace("blockquote", " {0,3}>").replace("fences", " {0,3}(?:`{3,}(?=[^`\\n]*\\n)|~~~)[^\\n]*\\n").replace("list", " {0,3}(?:[*+-]|1[.)])[ \\t]+[^ \\t\\n]").replace("html", "</?(?:tag)(?: +|\\n|/?>)|<(?:script|pre|style|textarea|!--)").replace("tag", ft).getRegex() }, Ir = { ...ln, html: S(`^ *(?:comment *(?:\\n|\\s*$)|<(tag)[\\s\\S]+?</\\1> *(?:\\n{2,}|\\s*$)|<tag(?:"[^"]*"|'[^']*'|\\s[^'"/>\\s]*)*?/?> *(?:\\n{2,}|\\s*$))`).replace("comment", on).replace(/tag/g, "(?!(?:a|em|strong|small|s|cite|q|dfn|abbr|data|time|code|var|samp|kbd|sub|sup|i|b|u|mark|ruby|rt|rp|bdi|bdo|span|br|wbr|ins|del|img)\\b)\\w+(?!:|[^\\w\\s@]*@)\\b").getRegex(), def: /^ *\[([^\]]+)\]: *<?([^\s>]+)>?(?: +(["(][^\n]+[")]))? *(?:\n+|$)/, heading: /^(#{1,6})(.*)(?:\n+|$)/, fences: ye, lheading: /^(.+?)\n {0,3}(=+|-+) *(?:\n+|$)/, paragraph: S(rn).replace("hr", Xe).replace("heading", ` *#{1,6} *[^
]`).replace("lheading", ks).replace("|table", "").replace("blockquote", " {0,3}>").replace("|fences", "").replace("|list", "").replace("|html", "").replace("|tag", "").getRegex() }, Dr = /^\\([!"#$%&'()*+,\-./:;<=>?@\[\]\\^_`{|}~])/, Mr = /^(`+)([^`]|[^`][\s\S]*?[^`])\1(?!`)/, ws = /^( {2,}|\\)\n(?!\s*$)/, $r = /^(`+|[^`])(?:(?= {2,}\n)|[\s\S]*?(?:(?=[\\<!\[`*_]|\b_|$)|[^ ](?= {2,}\n)))/, Me = /[\p{P}\p{S}]/u, dt = /[\s\p{P}\p{S}]/u, cn = /[^\s\p{P}\p{S}]/u, Nr = S(/^((?![*_])punctSpace)/, "u").replace(/punctSpace/g, dt).getRegex(), ys = /(?!~)[\p{P}\p{S}]/u, zr = /(?!~)[\s\p{P}\p{S}]/u, Fr = /(?:[^\s\p{P}\p{S}]|~)/u, Ur = S(/link|precode-code|html/, "g").replace("link", /\[(?:[^\[\]`]|(?<a>`+)[^`]+\k<a>(?!`))*?\]\((?:\\[\s\S]|[^\\\(\)]|\((?:\\[\s\S]|[^\\\(\)])*\))*\)/).replace("precode-", _r ? "(?<!`)()" : "(^^|[^`])").replace("code", /(?<b>`+)[^`]+\k<b>(?!`)/).replace("html", /<(?! )[^<>]*?>/).getRegex(), xs = /^(?:\*+(?:((?!\*)punct)|([^\s*]))?)|^_+(?:((?!_)punct)|([^\s_]))?/, Wr = S(xs, "u").replace(/punct/g, Me).getRegex(), jr = S(xs, "u").replace(/punct/g, ys).getRegex(), Ts = "^[^_*]*?__[^_*]*?\\*[^_*]*?(?=__)|[^*]+(?=[^*])|(?!\\*)punct(\\*+)(?=[\\s]|$)|notPunctSpace(\\*+)(?!\\*)(?=punctSpace|$)|(?!\\*)punctSpace(\\*+)(?=notPunctSpace)|[\\s](\\*+)(?!\\*)(?=punct)|(?!\\*)punct(\\*+)(?!\\*)(?=punct)|notPunctSpace(\\*+)(?=notPunctSpace)", Br = S(Ts, "gu").replace(/notPunctSpace/g, cn).replace(/punctSpace/g, dt).replace(/punct/g, Me).getRegex(), Hr = S(Ts, "gu").replace(/notPunctSpace/g, Fr).replace(/punctSpace/g, zr).replace(/punct/g, ys).getRegex(), qr = S("^[^_*]*?\\*\\*[^_*]*?_[^_*]*?(?=\\*\\*)|[^_]+(?=[^_])|(?!_)punct(_+)(?=[\\s]|$)|notPunctSpace(_+)(?!_)(?=punctSpace|$)|(?!_)punctSpace(_+)(?=notPunctSpace)|[\\s](_+)(?!_)(?=punct)|(?!_)punct(_+)(?!_)(?=punct)", "gu").replace(/notPunctSpace/g, cn).replace(/punctSpace/g, dt).replace(/punct/g, Me).getRegex(), Gr = S(/^~~?(?:((?!~)punct)|[^\s~])/, "u").replace(/punct/g, Me).getRegex(), Yr = "^[^~]+(?=[^~])|(?!~)punct(~~?)(?=[\\s]|$)|notPunctSpace(~~?)(?!~)(?=punctSpace|$)|(?!~)punctSpace(~~?)(?=notPunctSpace)|[\\s](~~?)(?!~)(?=punct)|(?!~)punct(~~?)(?!~)(?=punct)|notPunctSpace(~~?)(?=notPunctSpace)", Vr = S(Yr, "gu").replace(/notPunctSpace/g, cn).replace(/punctSpace/g, dt).replace(/punct/g, Me).getRegex(), Xr = S(/\\(punct)/, "gu").replace(/punct/g, Me).getRegex(), Zr = S(/^<(scheme:[^\s\x00-\x1f<>]*|email)>/).replace("scheme", /[a-zA-Z][a-zA-Z0-9+.-]{1,31}/).replace("email", /[a-zA-Z0-9.!#$%&'*+/=?^_`{|}~-]+(@)[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?(?:\.[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?)+(?![-_])/).getRegex(), Kr = S(on).replace("(?:-->|$)", "-->").getRegex(), Qr = S("^comment|^</[a-zA-Z][\\w:-]*\\s*>|^<[a-zA-Z][\\w-]*(?:attribute)*?\\s*/?>|^<\\?[\\s\\S]*?\\?>|^<![a-zA-Z]+\\s[\\s\\S]*?>|^<!\\[CDATA\\[[\\s\\S]*?\\]\\]>").replace("comment", Kr).replace("attribute", /\s+[a-zA-Z:_][\w.:-]*(?:\s*=\s*"[^"]*"|\s*=\s*'[^']*'|\s*=\s*[^\s"'=<>`]+)?/).getRegex(), ut = /(?:\[(?:\\[\s\S]|[^\[\]\\])*\]|\\[\s\S]|`+(?!`)[^`]*?`+(?!`)|``+(?=\])|[^\[\]\\`])*?/, Jr = S(/^!?\[(label)\]\(\s*(href)(?:(?:[ \t]+(?:\n[ \t]*)?|\n[ \t]*)(title))?\s*\)/).replace("label", ut).replace("href", /<(?:\\.|[^\n<>\\])+>|[^ \t\n\x00-\x1f]+|(?=\))/).replace("title", /"(?:\\"?|[^"\\])*"|'(?:\\'?|[^'\\])*'|\((?:\\\)?|[^)\\])*\)/).getRegex(), Ss = S(/^!?\[(label)\]\[(ref)\]/).replace("label", ut).replace("ref", an).getRegex(), Es = S(/^!?\[(ref)\](?:\[\])?/).replace("ref", an).getRegex(), ei = S("reflink|nolink(?!\\()", "g").replace("reflink", Ss).replace("nolink", Es).getRegex(), Yn = /[hH][tT][tT][pP][sS]?|[fF][tT][pP]/, un = { _backpedal: ye, anyPunctuation: Xr, autolink: Zr, blockSkip: Ur, br: ws, code: Mr, del: ye, delLDelim: ye, delRDelim: ye, emStrongLDelim: Wr, emStrongRDelimAst: Br, emStrongRDelimUnd: qr, escape: Dr, link: Jr, nolink: Es, punctuation: Nr, reflink: Ss, reflinkSearch: ei, tag: Qr, text: $r, url: ye }, ti = { ...un, link: S(/^!?\[(label)\]\((.*?)\)/).replace("label", ut).getRegex(), reflink: S(/^!?\[(label)\]\s*\[([^\]]*)\]/).replace("label", ut).getRegex() }, Kt = { ...un, emStrongRDelimAst: Hr, emStrongLDelim: jr, delLDelim: Gr, delRDelim: Vr, url: S(/^((?:protocol):\/\/|www\.)(?:[a-zA-Z0-9\-]+\.?)+[^\s<]*|^email/).replace("protocol", Yn).replace("email", /[A-Za-z0-9._+-]+(@)[a-zA-Z0-9-_]+(?:\.[a-zA-Z0-9-_]*[a-zA-Z0-9])+(?![-_])/).getRegex(), _backpedal: /(?:[^?!.,:;*_'"~()&]+|\([^)]*\)|&(?![a-zA-Z0-9]+;$)|[?!.,:;*_'"~)]+(?!$))+/, del: /^(~~?)(?=[^\s~])((?:\\[\s\S]|[^\\])*?(?:\\[\s\S]|[^\s~\\]))\1(?=[^~]|$)/, text: S(/^(`+|~+|[^`~])(?:(?=[`~])|(?= {2,}\n)|(?=[a-zA-Z0-9.!#$%&'*+\/=?_`{\|}~-]+@)|[\s\S]*?(?:(?=[\\<!\[`*~_]|\b_|protocol:\/\/|www\.|$)|[^ ](?= {2,}\n)|[^a-zA-Z0-9.!#$%&'*+\/=?_`{\|}~-](?=[a-zA-Z0-9.!#$%&'*+\/=?_`{\|}~-]+@)))/).replace("protocol", Yn).getRegex() }, ni = { ...Kt, br: S(ws).replace("{2,}", "*").getRegex(), text: S(Kt.text).replace("\\b_", "\\b_| {2,}\\n").replace(/\{2,\}/g, "*").getRegex() }, ot = { normal: ln, gfm: Pr, pedantic: Ir }, Be = { normal: un, gfm: Kt, breaks: ni, pedantic: ti }, si = { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }, Vn = (r) => si[r];
function ie(r, e) {
  if (e) {
    if (U.escapeTest.test(r)) return r.replace(U.escapeReplace, Vn);
  } else if (U.escapeTestNoEncode.test(r)) return r.replace(U.escapeReplaceNoEncode, Vn);
  return r;
}
function Xn(r) {
  try {
    r = encodeURI(r).replace(U.percentDecode, "%");
  } catch {
    return null;
  }
  return r;
}
function Zn(r, e) {
  var a;
  let t = r.replace(U.findPipe, (c, u, o) => {
    let g = !1, f = u;
    for (; --f >= 0 && o[f] === "\\"; ) g = !g;
    return g ? "|" : " |";
  }), n = t.split(U.splitPipe), s = 0;
  if (n[0].trim() || n.shift(), n.length > 0 && !((a = n.at(-1)) != null && a.trim()) && n.pop(), e) if (n.length > e) n.splice(e);
  else for (; n.length < e; ) n.push("");
  for (; s < n.length; s++) n[s] = n[s].trim().replace(U.slashPipe, "|");
  return n;
}
function fe(r, e, t) {
  let n = r.length;
  if (n === 0) return "";
  let s = 0;
  for (; s < n && r.charAt(n - s - 1) === e; )
    s++;
  return r.slice(0, n - s);
}
function Kn(r) {
  let e = r.split(`
`), t = e.length - 1;
  for (; t >= 0 && U.blankLine.test(e[t]); ) t--;
  return e.length - t <= 2 ? r : e.slice(0, t + 1).join(`
`);
}
function ri(r, e) {
  if (r.indexOf(e[1]) === -1) return -1;
  let t = 0;
  for (let n = 0; n < r.length; n++) if (r[n] === "\\") n++;
  else if (r[n] === e[0]) t++;
  else if (r[n] === e[1] && (t--, t < 0)) return n;
  return t > 0 ? -2 : -1;
}
function ii(r, e = 0) {
  let t = e, n = "";
  for (let s of r) if (s === "	") {
    let a = 4 - t % 4;
    n += " ".repeat(a), t += a;
  } else n += s, t++;
  return n;
}
function Qn(r, e, t, n, s) {
  let a = e.href, c = e.title || null, u = r[1].replace(s.other.outputLinkReplace, "$1");
  n.state.inLink = !0;
  let o = { type: r[0].charAt(0) === "!" ? "image" : "link", raw: t, href: a, title: c, text: u, tokens: n.inlineTokens(u) };
  return n.state.inLink = !1, o;
}
function ai(r, e, t) {
  let n = r.match(t.other.indentCodeCompensation);
  if (n === null) return e;
  let s = n[1];
  return e.split(`
`).map((a) => {
    let c = a.match(t.other.beginningSpace);
    if (c === null) return a;
    let [u] = c;
    return u.length >= s.length ? a.slice(s.length) : a;
  }).join(`
`);
}
var ht = class {
  constructor(r) {
    k(this, "options");
    k(this, "rules");
    k(this, "lexer");
    this.options = r || Te;
  }
  space(r) {
    let e = this.rules.block.newline.exec(r);
    if (e && e[0].length > 0) return { type: "space", raw: e[0] };
  }
  code(r) {
    let e = this.rules.block.code.exec(r);
    if (e) {
      let t = this.options.pedantic ? e[0] : Kn(e[0]), n = t.replace(this.rules.other.codeRemoveIndent, "");
      return { type: "code", raw: t, codeBlockStyle: "indented", text: n };
    }
  }
  fences(r) {
    let e = this.rules.block.fences.exec(r);
    if (e) {
      let t = e[0], n = ai(t, e[3] || "", this.rules);
      return { type: "code", raw: t, lang: e[2] ? e[2].trim().replace(this.rules.inline.anyPunctuation, "$1") : e[2], text: n };
    }
  }
  heading(r) {
    let e = this.rules.block.heading.exec(r);
    if (e) {
      let t = e[2].trim();
      if (this.rules.other.endingHash.test(t)) {
        let n = fe(t, "#");
        (this.options.pedantic || !n || this.rules.other.endingSpaceChar.test(n)) && (t = n.trim());
      }
      return { type: "heading", raw: fe(e[0], `
`), depth: e[1].length, text: t, tokens: this.lexer.inline(t) };
    }
  }
  hr(r) {
    let e = this.rules.block.hr.exec(r);
    if (e) return { type: "hr", raw: fe(e[0], `
`) };
  }
  blockquote(r) {
    let e = this.rules.block.blockquote.exec(r);
    if (e) {
      let t = fe(e[0], `
`).split(`
`), n = "", s = "", a = [];
      for (; t.length > 0; ) {
        let c = !1, u = [], o;
        for (o = 0; o < t.length; o++) if (this.rules.other.blockquoteStart.test(t[o])) u.push(t[o]), c = !0;
        else if (!c) u.push(t[o]);
        else break;
        t = t.slice(o);
        let g = u.join(`
`), f = g.replace(this.rules.other.blockquoteSetextReplace, `
    $1`).replace(this.rules.other.blockquoteSetextReplace2, "");
        n = n ? `${n}
${g}` : g, s = s ? `${s}
${f}` : f;
        let b = this.lexer.state.top;
        if (this.lexer.state.top = !0, this.lexer.blockTokens(f, a, !0), this.lexer.state.top = b, t.length === 0) break;
        let d = a.at(-1);
        if ((d == null ? void 0 : d.type) === "code") break;
        if ((d == null ? void 0 : d.type) === "blockquote") {
          let T = d, w = T.raw + `
` + t.join(`
`), C = this.blockquote(w);
          a[a.length - 1] = C, n = n.substring(0, n.length - T.raw.length) + C.raw, s = s.substring(0, s.length - T.text.length) + C.text;
          break;
        } else if ((d == null ? void 0 : d.type) === "list") {
          let T = d, w = T.raw + `
` + t.join(`
`), C = this.list(w);
          a[a.length - 1] = C, n = n.substring(0, n.length - d.raw.length) + C.raw, s = s.substring(0, s.length - T.raw.length) + C.raw, t = w.substring(a.at(-1).raw.length).split(`
`);
          continue;
        }
      }
      return { type: "blockquote", raw: n, tokens: a, text: s };
    }
  }
  list(r) {
    let e = this.rules.block.list.exec(r);
    if (e) {
      let t = e[1].trim(), n = t.length > 1, s = { type: "list", raw: "", ordered: n, start: n ? +t.slice(0, -1) : "", loose: !1, items: [] };
      t = n ? `\\d{1,9}\\${t.slice(-1)}` : `\\${t}`, this.options.pedantic && (t = n ? t : "[*+-]");
      let a = this.rules.other.listItemRegex(t), c = !1;
      for (; r; ) {
        let o = !1, g = "", f = "";
        if (!(e = a.exec(r)) || this.rules.block.hr.test(r)) break;
        g = e[0], r = r.substring(g.length);
        let b = ii(e[2].split(`
`, 1)[0], e[1].length), d = r.split(`
`, 1)[0], T = !b.trim(), w = 0;
        if (this.options.pedantic ? (w = 2, f = b.trimStart()) : T ? w = e[1].length + 1 : (w = b.search(this.rules.other.nonSpaceChar), w = w > 4 ? 1 : w, f = b.slice(w), w += e[1].length), T && this.rules.other.blankLine.test(d) && (g += d + `
`, r = r.substring(d.length + 1), o = !0), !o) {
          let C = this.rules.other.nextBulletRegex(w), $ = this.rules.other.hrRegex(w), D = this.rules.other.fencesBeginRegex(w), Y = this.rules.other.headingBeginRegex(w), Z = this.rules.other.htmlBeginRegex(w), ae = this.rules.other.blockquoteBeginRegex(w);
          for (; r; ) {
            let ee = r.split(`
`, 1)[0], te;
            if (d = ee, this.options.pedantic ? (d = d.replace(this.rules.other.listReplaceNesting, "  "), te = d) : te = d.replace(this.rules.other.tabCharGlobal, "    "), D.test(d) || Y.test(d) || Z.test(d) || ae.test(d) || C.test(d) || $.test(d)) break;
            if (te.search(this.rules.other.nonSpaceChar) >= w || !d.trim()) f += `
` + te.slice(w);
            else {
              if (T || b.replace(this.rules.other.tabCharGlobal, "    ").search(this.rules.other.nonSpaceChar) >= 4 || D.test(b) || Y.test(b) || $.test(b)) break;
              f += `
` + d;
            }
            T = !d.trim(), g += ee + `
`, r = r.substring(ee.length + 1), b = te.slice(w);
          }
        }
        s.loose || (c ? s.loose = !0 : this.rules.other.doubleBlankLine.test(g) && (c = !0)), s.items.push({ type: "list_item", raw: g, task: !!this.options.gfm && this.rules.other.listIsTask.test(f), loose: !1, text: f, tokens: [] }), s.raw += g;
      }
      let u = s.items.at(-1);
      if (u) u.raw = u.raw.trimEnd(), u.text = u.text.trimEnd();
      else return;
      s.raw = s.raw.trimEnd();
      for (let o of s.items) {
        this.lexer.state.top = !1, o.tokens = this.lexer.blockTokens(o.text, []);
        let g = o.tokens[0];
        if (o.task && ((g == null ? void 0 : g.type) === "text" || (g == null ? void 0 : g.type) === "paragraph")) {
          o.text = o.text.replace(this.rules.other.listReplaceTask, ""), g.raw = g.raw.replace(this.rules.other.listReplaceTask, ""), g.text = g.text.replace(this.rules.other.listReplaceTask, "");
          for (let b = this.lexer.inlineQueue.length - 1; b >= 0; b--) if (this.rules.other.listIsTask.test(this.lexer.inlineQueue[b].src)) {
            this.lexer.inlineQueue[b].src = this.lexer.inlineQueue[b].src.replace(this.rules.other.listReplaceTask, "");
            break;
          }
          let f = this.rules.other.listTaskCheckbox.exec(o.raw);
          if (f) {
            let b = { type: "checkbox", raw: f[0] + " ", checked: f[0] !== "[ ]" };
            o.checked = b.checked, s.loose ? o.tokens[0] && ["paragraph", "text"].includes(o.tokens[0].type) && "tokens" in o.tokens[0] && o.tokens[0].tokens ? (o.tokens[0].raw = b.raw + o.tokens[0].raw, o.tokens[0].text = b.raw + o.tokens[0].text, o.tokens[0].tokens.unshift(b)) : o.tokens.unshift({ type: "paragraph", raw: b.raw, text: b.raw, tokens: [b] }) : o.tokens.unshift(b);
          }
        } else o.task && (o.task = !1);
        if (!s.loose) {
          let f = o.tokens.filter((d) => d.type === "space"), b = f.length > 0 && f.some((d) => this.rules.other.anyLine.test(d.raw));
          s.loose = b;
        }
      }
      if (s.loose) for (let o of s.items) {
        o.loose = !0;
        for (let g of o.tokens) g.type === "text" && (g.type = "paragraph");
      }
      return s;
    }
  }
  html(r) {
    let e = this.rules.block.html.exec(r);
    if (e) {
      let t = Kn(e[0]);
      return { type: "html", block: !0, raw: t, pre: e[1] === "pre" || e[1] === "script" || e[1] === "style", text: t };
    }
  }
  def(r) {
    let e = this.rules.block.def.exec(r);
    if (e) {
      let t = e[1].toLowerCase().replace(this.rules.other.multipleSpaceGlobal, " "), n = e[2] ? e[2].replace(this.rules.other.hrefBrackets, "$1").replace(this.rules.inline.anyPunctuation, "$1") : "", s = e[3] ? e[3].substring(1, e[3].length - 1).replace(this.rules.inline.anyPunctuation, "$1") : e[3];
      return { type: "def", tag: t, raw: fe(e[0], `
`), href: n, title: s };
    }
  }
  table(r) {
    var c;
    let e = this.rules.block.table.exec(r);
    if (!e || !this.rules.other.tableDelimiter.test(e[2])) return;
    let t = Zn(e[1]), n = e[2].replace(this.rules.other.tableAlignChars, "").split("|"), s = (c = e[3]) != null && c.trim() ? e[3].replace(this.rules.other.tableRowBlankLine, "").split(`
`) : [], a = { type: "table", raw: fe(e[0], `
`), header: [], align: [], rows: [] };
    if (t.length === n.length) {
      for (let u of n) this.rules.other.tableAlignRight.test(u) ? a.align.push("right") : this.rules.other.tableAlignCenter.test(u) ? a.align.push("center") : this.rules.other.tableAlignLeft.test(u) ? a.align.push("left") : a.align.push(null);
      for (let u = 0; u < t.length; u++) a.header.push({ text: t[u], tokens: this.lexer.inline(t[u]), header: !0, align: a.align[u] });
      for (let u of s) a.rows.push(Zn(u, a.header.length).map((o, g) => ({ text: o, tokens: this.lexer.inline(o), header: !1, align: a.align[g] })));
      return a;
    }
  }
  lheading(r) {
    let e = this.rules.block.lheading.exec(r);
    if (e) {
      let t = e[1].trim();
      return { type: "heading", raw: fe(e[0], `
`), depth: e[2].charAt(0) === "=" ? 1 : 2, text: t, tokens: this.lexer.inline(t) };
    }
  }
  paragraph(r) {
    let e = this.rules.block.paragraph.exec(r);
    if (e) {
      let t = e[1].charAt(e[1].length - 1) === `
` ? e[1].slice(0, -1) : e[1];
      return { type: "paragraph", raw: e[0], text: t, tokens: this.lexer.inline(t) };
    }
  }
  text(r) {
    let e = this.rules.block.text.exec(r);
    if (e) return { type: "text", raw: e[0], text: e[0], tokens: this.lexer.inline(e[0]) };
  }
  escape(r) {
    let e = this.rules.inline.escape.exec(r);
    if (e) return { type: "escape", raw: e[0], text: e[1] };
  }
  tag(r) {
    let e = this.rules.inline.tag.exec(r);
    if (e) return !this.lexer.state.inLink && this.rules.other.startATag.test(e[0]) ? this.lexer.state.inLink = !0 : this.lexer.state.inLink && this.rules.other.endATag.test(e[0]) && (this.lexer.state.inLink = !1), !this.lexer.state.inRawBlock && this.rules.other.startPreScriptTag.test(e[0]) ? this.lexer.state.inRawBlock = !0 : this.lexer.state.inRawBlock && this.rules.other.endPreScriptTag.test(e[0]) && (this.lexer.state.inRawBlock = !1), { type: "html", raw: e[0], inLink: this.lexer.state.inLink, inRawBlock: this.lexer.state.inRawBlock, block: !1, text: e[0] };
  }
  link(r) {
    let e = this.rules.inline.link.exec(r);
    if (e) {
      let t = e[2].trim();
      if (!this.options.pedantic && this.rules.other.startAngleBracket.test(t)) {
        if (!this.rules.other.endAngleBracket.test(t)) return;
        let a = fe(t.slice(0, -1), "\\");
        if ((t.length - a.length) % 2 === 0) return;
      } else {
        let a = ri(e[2], "()");
        if (a === -2) return;
        if (a > -1) {
          let c = (e[0].indexOf("!") === 0 ? 5 : 4) + e[1].length + a;
          e[2] = e[2].substring(0, a), e[0] = e[0].substring(0, c).trim(), e[3] = "";
        }
      }
      let n = e[2], s = "";
      if (this.options.pedantic) {
        let a = this.rules.other.pedanticHrefTitle.exec(n);
        a && (n = a[1], s = a[3]);
      } else s = e[3] ? e[3].slice(1, -1) : "";
      return n = n.trim(), this.rules.other.startAngleBracket.test(n) && (this.options.pedantic && !this.rules.other.endAngleBracket.test(t) ? n = n.slice(1) : n = n.slice(1, -1)), Qn(e, { href: n && n.replace(this.rules.inline.anyPunctuation, "$1"), title: s && s.replace(this.rules.inline.anyPunctuation, "$1") }, e[0], this.lexer, this.rules);
    }
  }
  reflink(r, e) {
    let t;
    if ((t = this.rules.inline.reflink.exec(r)) || (t = this.rules.inline.nolink.exec(r))) {
      let n = (t[2] || t[1]).replace(this.rules.other.multipleSpaceGlobal, " "), s = e[n.toLowerCase()];
      if (!s) {
        let a = t[0].charAt(0);
        return { type: "text", raw: a, text: a };
      }
      return Qn(t, s, t[0], this.lexer, this.rules);
    }
  }
  emStrong(r, e, t = "") {
    let n = this.rules.inline.emStrongLDelim.exec(r);
    if (!(!n || !n[1] && !n[2] && !n[3] && !n[4] || n[4] && t.match(this.rules.other.unicodeAlphaNumeric)) && (!(n[1] || n[3]) || !t || this.rules.inline.punctuation.exec(t))) {
      let s = [...n[0]].length - 1, a, c, u = s, o = 0, g = n[0][0] === "*" ? this.rules.inline.emStrongRDelimAst : this.rules.inline.emStrongRDelimUnd;
      for (g.lastIndex = 0, e = e.slice(-1 * r.length + s); (n = g.exec(e)) !== null; ) {
        if (a = n[1] || n[2] || n[3] || n[4] || n[5] || n[6], !a) continue;
        if (c = [...a].length, n[3] || n[4]) {
          u += c;
          continue;
        } else if ((n[5] || n[6]) && s % 3 && !((s + c) % 3)) {
          o += c;
          continue;
        }
        if (u -= c, u > 0) continue;
        c = Math.min(c, c + u + o);
        let f = [...n[0]][0].length, b = r.slice(0, s + n.index + f + c);
        if (Math.min(s, c) % 2) {
          let T = b.slice(1, -1);
          return { type: "em", raw: b, text: T, tokens: this.lexer.inlineTokens(T) };
        }
        let d = b.slice(2, -2);
        return { type: "strong", raw: b, text: d, tokens: this.lexer.inlineTokens(d) };
      }
    }
  }
  codespan(r) {
    let e = this.rules.inline.code.exec(r);
    if (e) {
      let t = e[2].replace(this.rules.other.newLineCharGlobal, " "), n = this.rules.other.nonSpaceChar.test(t), s = this.rules.other.startingSpaceChar.test(t) && this.rules.other.endingSpaceChar.test(t);
      return n && s && (t = t.substring(1, t.length - 1)), { type: "codespan", raw: e[0], text: t };
    }
  }
  br(r) {
    let e = this.rules.inline.br.exec(r);
    if (e) return { type: "br", raw: e[0] };
  }
  del(r, e, t = "") {
    let n = this.rules.inline.delLDelim.exec(r);
    if (n && (!n[1] || !t || this.rules.inline.punctuation.exec(t))) {
      let s = [...n[0]].length - 1, a, c, u = s, o = this.rules.inline.delRDelim;
      for (o.lastIndex = 0, e = e.slice(-1 * r.length + s); (n = o.exec(e)) !== null; ) {
        if (a = n[1] || n[2] || n[3] || n[4] || n[5] || n[6], !a || (c = [...a].length, c !== s)) continue;
        if (n[3] || n[4]) {
          u += c;
          continue;
        }
        if (u -= c, u > 0) continue;
        c = Math.min(c, c + u);
        let g = [...n[0]][0].length, f = r.slice(0, s + n.index + g + c), b = f.slice(s, -s);
        return { type: "del", raw: f, text: b, tokens: this.lexer.inlineTokens(b) };
      }
    }
  }
  autolink(r) {
    let e = this.rules.inline.autolink.exec(r);
    if (e) {
      let t, n;
      return e[2] === "@" ? (t = e[1], n = "mailto:" + t) : (t = e[1], n = t), { type: "link", raw: e[0], text: t, href: n, tokens: [{ type: "text", raw: t, text: t }] };
    }
  }
  url(r) {
    var t;
    let e;
    if (e = this.rules.inline.url.exec(r)) {
      let n, s;
      if (e[2] === "@") n = e[0], s = "mailto:" + n;
      else {
        let a;
        do
          a = e[0], e[0] = ((t = this.rules.inline._backpedal.exec(e[0])) == null ? void 0 : t[0]) ?? "";
        while (a !== e[0]);
        n = e[0], e[1] === "www." ? s = "http://" + e[0] : s = e[0];
      }
      return { type: "link", raw: e[0], text: n, href: s, tokens: [{ type: "text", raw: n, text: n }] };
    }
  }
  inlineText(r) {
    let e = this.rules.inline.text.exec(r);
    if (e) {
      let t = this.lexer.state.inRawBlock;
      return { type: "text", raw: e[0], text: e[0], escaped: t };
    }
  }
}, Q = class Qt {
  constructor(e) {
    k(this, "tokens");
    k(this, "options");
    k(this, "state");
    k(this, "inlineQueue");
    k(this, "tokenizer");
    this.tokens = [], this.tokens.links = /* @__PURE__ */ Object.create(null), this.options = e || Te, this.options.tokenizer = this.options.tokenizer || new ht(), this.tokenizer = this.options.tokenizer, this.tokenizer.options = this.options, this.tokenizer.lexer = this, this.inlineQueue = [], this.state = { inLink: !1, inRawBlock: !1, top: !0 };
    let t = { other: U, block: ot.normal, inline: Be.normal };
    this.options.pedantic ? (t.block = ot.pedantic, t.inline = Be.pedantic) : this.options.gfm && (t.block = ot.gfm, this.options.breaks ? t.inline = Be.breaks : t.inline = Be.gfm), this.tokenizer.rules = t;
  }
  static get rules() {
    return { block: ot, inline: Be };
  }
  static lex(e, t) {
    return new Qt(t).lex(e);
  }
  static lexInline(e, t) {
    return new Qt(t).inlineTokens(e);
  }
  lex(e) {
    e = e.replace(U.carriageReturn, `
`), this.blockTokens(e, this.tokens);
    for (let t = 0; t < this.inlineQueue.length; t++) {
      let n = this.inlineQueue[t];
      this.inlineTokens(n.src, n.tokens);
    }
    return this.inlineQueue = [], this.tokens;
  }
  blockTokens(e, t = [], n = !1) {
    var a, c, u;
    this.tokenizer.lexer = this, this.options.pedantic && (e = e.replace(U.tabCharGlobal, "    ").replace(U.spaceLine, ""));
    let s = 1 / 0;
    for (; e; ) {
      if (e.length < s) s = e.length;
      else {
        this.infiniteLoopError(e.charCodeAt(0));
        break;
      }
      let o;
      if ((c = (a = this.options.extensions) == null ? void 0 : a.block) != null && c.some((f) => (o = f.call({ lexer: this }, e, t)) ? (e = e.substring(o.raw.length), t.push(o), !0) : !1)) continue;
      if (o = this.tokenizer.space(e)) {
        e = e.substring(o.raw.length);
        let f = t.at(-1);
        o.raw.length === 1 && f !== void 0 ? f.raw += `
` : t.push(o);
        continue;
      }
      if (o = this.tokenizer.code(e)) {
        e = e.substring(o.raw.length);
        let f = t.at(-1);
        (f == null ? void 0 : f.type) === "paragraph" || (f == null ? void 0 : f.type) === "text" ? (f.raw += (f.raw.endsWith(`
`) ? "" : `
`) + o.raw, f.text += `
` + o.text, this.inlineQueue.at(-1).src = f.text) : t.push(o);
        continue;
      }
      if (o = this.tokenizer.fences(e)) {
        e = e.substring(o.raw.length), t.push(o);
        continue;
      }
      if (o = this.tokenizer.heading(e)) {
        e = e.substring(o.raw.length), t.push(o);
        continue;
      }
      if (o = this.tokenizer.hr(e)) {
        e = e.substring(o.raw.length), t.push(o);
        continue;
      }
      if (o = this.tokenizer.blockquote(e)) {
        e = e.substring(o.raw.length), t.push(o);
        continue;
      }
      if (o = this.tokenizer.list(e)) {
        e = e.substring(o.raw.length), t.push(o);
        continue;
      }
      if (o = this.tokenizer.html(e)) {
        e = e.substring(o.raw.length), t.push(o);
        continue;
      }
      if (o = this.tokenizer.def(e)) {
        e = e.substring(o.raw.length);
        let f = t.at(-1);
        (f == null ? void 0 : f.type) === "paragraph" || (f == null ? void 0 : f.type) === "text" ? (f.raw += (f.raw.endsWith(`
`) ? "" : `
`) + o.raw, f.text += `
` + o.raw, this.inlineQueue.at(-1).src = f.text) : this.tokens.links[o.tag] || (this.tokens.links[o.tag] = { href: o.href, title: o.title }, t.push(o));
        continue;
      }
      if (o = this.tokenizer.table(e)) {
        e = e.substring(o.raw.length), t.push(o);
        continue;
      }
      if (o = this.tokenizer.lheading(e)) {
        e = e.substring(o.raw.length), t.push(o);
        continue;
      }
      let g = e;
      if ((u = this.options.extensions) != null && u.startBlock) {
        let f = 1 / 0, b = e.slice(1), d;
        this.options.extensions.startBlock.forEach((T) => {
          d = T.call({ lexer: this }, b), typeof d == "number" && d >= 0 && (f = Math.min(f, d));
        }), f < 1 / 0 && f >= 0 && (g = e.substring(0, f + 1));
      }
      if (this.state.top && (o = this.tokenizer.paragraph(g))) {
        let f = t.at(-1);
        n && (f == null ? void 0 : f.type) === "paragraph" ? (f.raw += (f.raw.endsWith(`
`) ? "" : `
`) + o.raw, f.text += `
` + o.text, this.inlineQueue.pop(), this.inlineQueue.at(-1).src = f.text) : t.push(o), n = g.length !== e.length, e = e.substring(o.raw.length);
        continue;
      }
      if (o = this.tokenizer.text(e)) {
        e = e.substring(o.raw.length);
        let f = t.at(-1);
        (f == null ? void 0 : f.type) === "text" ? (f.raw += (f.raw.endsWith(`
`) ? "" : `
`) + o.raw, f.text += `
` + o.text, this.inlineQueue.pop(), this.inlineQueue.at(-1).src = f.text) : t.push(o);
        continue;
      }
      if (e) {
        this.infiniteLoopError(e.charCodeAt(0));
        break;
      }
    }
    return this.state.top = !0, t;
  }
  inline(e, t = []) {
    return this.inlineQueue.push({ src: e, tokens: t }), t;
  }
  inlineTokens(e, t = []) {
    var u, o, g, f, b;
    this.tokenizer.lexer = this;
    let n = e;
    if (this.tokens.links) {
      let d = Object.keys(this.tokens.links);
      d.length > 0 && (n = n.replace(this.tokenizer.rules.inline.reflinkSearch, (T) => d.includes(T.slice(T.lastIndexOf("[") + 1, -1)) ? "[" + "a".repeat(T.length - 2) + "]" : T));
    }
    n = n.replace(this.tokenizer.rules.inline.anyPunctuation, "++"), n = n.replace(this.tokenizer.rules.inline.blockSkip, (d, T, w) => {
      let C = w ? w.length : 0;
      return d.slice(0, C) + "[" + "a".repeat(d.length - C - 2) + "]";
    }), n = ((o = (u = this.options.hooks) == null ? void 0 : u.emStrongMask) == null ? void 0 : o.call({ lexer: this }, n)) ?? n;
    let s = !1, a = "", c = 1 / 0;
    for (; e; ) {
      if (e.length < c) c = e.length;
      else {
        this.infiniteLoopError(e.charCodeAt(0));
        break;
      }
      s || (a = ""), s = !1;
      let d;
      if ((f = (g = this.options.extensions) == null ? void 0 : g.inline) != null && f.some((w) => (d = w.call({ lexer: this }, e, t)) ? (e = e.substring(d.raw.length), t.push(d), !0) : !1)) continue;
      if (d = this.tokenizer.escape(e)) {
        e = e.substring(d.raw.length), t.push(d);
        continue;
      }
      if (d = this.tokenizer.tag(e)) {
        e = e.substring(d.raw.length), t.push(d);
        continue;
      }
      if (d = this.tokenizer.link(e)) {
        e = e.substring(d.raw.length), t.push(d);
        continue;
      }
      if (d = this.tokenizer.reflink(e, this.tokens.links)) {
        e = e.substring(d.raw.length);
        let w = t.at(-1);
        d.type === "text" && (w == null ? void 0 : w.type) === "text" ? (w.raw += d.raw, w.text += d.text) : t.push(d);
        continue;
      }
      if (d = this.tokenizer.emStrong(e, n, a)) {
        e = e.substring(d.raw.length), t.push(d);
        continue;
      }
      if (d = this.tokenizer.codespan(e)) {
        e = e.substring(d.raw.length), t.push(d);
        continue;
      }
      if (d = this.tokenizer.br(e)) {
        e = e.substring(d.raw.length), t.push(d);
        continue;
      }
      if (d = this.tokenizer.del(e, n, a)) {
        e = e.substring(d.raw.length), t.push(d);
        continue;
      }
      if (d = this.tokenizer.autolink(e)) {
        e = e.substring(d.raw.length), t.push(d);
        continue;
      }
      if (!this.state.inLink && (d = this.tokenizer.url(e))) {
        e = e.substring(d.raw.length), t.push(d);
        continue;
      }
      let T = e;
      if ((b = this.options.extensions) != null && b.startInline) {
        let w = 1 / 0, C = e.slice(1), $;
        this.options.extensions.startInline.forEach((D) => {
          $ = D.call({ lexer: this }, C), typeof $ == "number" && $ >= 0 && (w = Math.min(w, $));
        }), w < 1 / 0 && w >= 0 && (T = e.substring(0, w + 1));
      }
      if (d = this.tokenizer.inlineText(T)) {
        e = e.substring(d.raw.length), d.raw.slice(-1) !== "_" && (a = d.raw.slice(-1)), s = !0;
        let w = t.at(-1);
        (w == null ? void 0 : w.type) === "text" ? (w.raw += d.raw, w.text += d.text) : t.push(d);
        continue;
      }
      if (e) {
        this.infiniteLoopError(e.charCodeAt(0));
        break;
      }
    }
    return t;
  }
  infiniteLoopError(e) {
    let t = "Infinite loop on byte: " + e;
    if (this.options.silent) console.error(t);
    else throw new Error(t);
  }
}, pt = class {
  constructor(r) {
    k(this, "options");
    k(this, "parser");
    this.options = r || Te;
  }
  space(r) {
    return "";
  }
  code({ text: r, lang: e, escaped: t }) {
    var a;
    let n = (a = (e || "").match(U.notSpaceStart)) == null ? void 0 : a[0], s = r.replace(U.endingNewline, "") + `
`;
    return n ? '<pre><code class="language-' + ie(n) + '">' + (t ? s : ie(s, !0)) + `</code></pre>
` : "<pre><code>" + (t ? s : ie(s, !0)) + `</code></pre>
`;
  }
  blockquote({ tokens: r }) {
    return `<blockquote>
${this.parser.parse(r)}</blockquote>
`;
  }
  html({ text: r }) {
    return r;
  }
  def(r) {
    return "";
  }
  heading({ tokens: r, depth: e }) {
    return `<h${e}>${this.parser.parseInline(r)}</h${e}>
`;
  }
  hr(r) {
    return `<hr>
`;
  }
  list(r) {
    let e = r.ordered, t = r.start, n = "";
    for (let c = 0; c < r.items.length; c++) {
      let u = r.items[c];
      n += this.listitem(u);
    }
    let s = e ? "ol" : "ul", a = e && t !== 1 ? ' start="' + t + '"' : "";
    return "<" + s + a + `>
` + n + "</" + s + `>
`;
  }
  listitem(r) {
    return `<li>${this.parser.parse(r.tokens)}</li>
`;
  }
  checkbox({ checked: r }) {
    return "<input " + (r ? 'checked="" ' : "") + 'disabled="" type="checkbox"> ';
  }
  paragraph({ tokens: r }) {
    return `<p>${this.parser.parseInline(r)}</p>
`;
  }
  table(r) {
    let e = "", t = "";
    for (let s = 0; s < r.header.length; s++) t += this.tablecell(r.header[s]);
    e += this.tablerow({ text: t });
    let n = "";
    for (let s = 0; s < r.rows.length; s++) {
      let a = r.rows[s];
      t = "";
      for (let c = 0; c < a.length; c++) t += this.tablecell(a[c]);
      n += this.tablerow({ text: t });
    }
    return n && (n = `<tbody>${n}</tbody>`), `<table>
<thead>
` + e + `</thead>
` + n + `</table>
`;
  }
  tablerow({ text: r }) {
    return `<tr>
${r}</tr>
`;
  }
  tablecell(r) {
    let e = this.parser.parseInline(r.tokens), t = r.header ? "th" : "td";
    return (r.align ? `<${t} align="${r.align}">` : `<${t}>`) + e + `</${t}>
`;
  }
  strong({ tokens: r }) {
    return `<strong>${this.parser.parseInline(r)}</strong>`;
  }
  em({ tokens: r }) {
    return `<em>${this.parser.parseInline(r)}</em>`;
  }
  codespan({ text: r }) {
    return `<code>${ie(r, !0)}</code>`;
  }
  br(r) {
    return "<br>";
  }
  del({ tokens: r }) {
    return `<del>${this.parser.parseInline(r)}</del>`;
  }
  link({ href: r, title: e, tokens: t }) {
    let n = this.parser.parseInline(t), s = Xn(r);
    if (s === null) return n;
    r = s;
    let a = '<a href="' + r + '"';
    return e && (a += ' title="' + ie(e) + '"'), a += ">" + n + "</a>", a;
  }
  image({ href: r, title: e, text: t, tokens: n }) {
    n && (t = this.parser.parseInline(n, this.parser.textRenderer));
    let s = Xn(r);
    if (s === null) return ie(t);
    r = s;
    let a = `<img src="${r}" alt="${ie(t)}"`;
    return e && (a += ` title="${ie(e)}"`), a += ">", a;
  }
  text(r) {
    return "tokens" in r && r.tokens ? this.parser.parseInline(r.tokens) : "escaped" in r && r.escaped ? r.text : ie(r.text);
  }
}, hn = class {
  strong({ text: r }) {
    return r;
  }
  em({ text: r }) {
    return r;
  }
  codespan({ text: r }) {
    return r;
  }
  del({ text: r }) {
    return r;
  }
  html({ text: r }) {
    return r;
  }
  text({ text: r }) {
    return r;
  }
  link({ text: r }) {
    return "" + r;
  }
  image({ text: r }) {
    return "" + r;
  }
  br() {
    return "";
  }
  checkbox({ raw: r }) {
    return r;
  }
}, J = class Jt {
  constructor(e) {
    k(this, "options");
    k(this, "renderer");
    k(this, "textRenderer");
    this.options = e || Te, this.options.renderer = this.options.renderer || new pt(), this.renderer = this.options.renderer, this.renderer.options = this.options, this.renderer.parser = this, this.textRenderer = new hn();
  }
  static parse(e, t) {
    return new Jt(t).parse(e);
  }
  static parseInline(e, t) {
    return new Jt(t).parseInline(e);
  }
  parse(e) {
    var n, s;
    this.renderer.parser = this;
    let t = "";
    for (let a = 0; a < e.length; a++) {
      let c = e[a];
      if ((s = (n = this.options.extensions) == null ? void 0 : n.renderers) != null && s[c.type]) {
        let o = c, g = this.options.extensions.renderers[o.type].call({ parser: this }, o);
        if (g !== !1 || !["space", "hr", "heading", "code", "table", "blockquote", "list", "html", "def", "paragraph", "text"].includes(o.type)) {
          t += g || "";
          continue;
        }
      }
      let u = c;
      switch (u.type) {
        case "space": {
          t += this.renderer.space(u);
          break;
        }
        case "hr": {
          t += this.renderer.hr(u);
          break;
        }
        case "heading": {
          t += this.renderer.heading(u);
          break;
        }
        case "code": {
          t += this.renderer.code(u);
          break;
        }
        case "table": {
          t += this.renderer.table(u);
          break;
        }
        case "blockquote": {
          t += this.renderer.blockquote(u);
          break;
        }
        case "list": {
          t += this.renderer.list(u);
          break;
        }
        case "checkbox": {
          t += this.renderer.checkbox(u);
          break;
        }
        case "html": {
          t += this.renderer.html(u);
          break;
        }
        case "def": {
          t += this.renderer.def(u);
          break;
        }
        case "paragraph": {
          t += this.renderer.paragraph(u);
          break;
        }
        case "text": {
          t += this.renderer.text(u);
          break;
        }
        default: {
          let o = 'Token with "' + u.type + '" type was not found.';
          if (this.options.silent) return console.error(o), "";
          throw new Error(o);
        }
      }
    }
    return t;
  }
  parseInline(e, t = this.renderer) {
    var s, a;
    this.renderer.parser = this;
    let n = "";
    for (let c = 0; c < e.length; c++) {
      let u = e[c];
      if ((a = (s = this.options.extensions) == null ? void 0 : s.renderers) != null && a[u.type]) {
        let g = this.options.extensions.renderers[u.type].call({ parser: this }, u);
        if (g !== !1 || !["escape", "html", "link", "image", "strong", "em", "codespan", "br", "del", "text"].includes(u.type)) {
          n += g || "";
          continue;
        }
      }
      let o = u;
      switch (o.type) {
        case "escape": {
          n += t.text(o);
          break;
        }
        case "html": {
          n += t.html(o);
          break;
        }
        case "link": {
          n += t.link(o);
          break;
        }
        case "image": {
          n += t.image(o);
          break;
        }
        case "checkbox": {
          n += t.checkbox(o);
          break;
        }
        case "strong": {
          n += t.strong(o);
          break;
        }
        case "em": {
          n += t.em(o);
          break;
        }
        case "codespan": {
          n += t.codespan(o);
          break;
        }
        case "br": {
          n += t.br(o);
          break;
        }
        case "del": {
          n += t.del(o);
          break;
        }
        case "text": {
          n += t.text(o);
          break;
        }
        default: {
          let g = 'Token with "' + o.type + '" type was not found.';
          if (this.options.silent) return console.error(g), "";
          throw new Error(g);
        }
      }
    }
    return n;
  }
}, ct, Ve = (ct = class {
  constructor(r) {
    k(this, "options");
    k(this, "block");
    this.options = r || Te;
  }
  preprocess(r) {
    return r;
  }
  postprocess(r) {
    return r;
  }
  processAllTokens(r) {
    return r;
  }
  emStrongMask(r) {
    return r;
  }
  provideLexer(r = this.block) {
    return r ? Q.lex : Q.lexInline;
  }
  provideParser(r = this.block) {
    return r ? J.parse : J.parseInline;
  }
}, k(ct, "passThroughHooks", /* @__PURE__ */ new Set(["preprocess", "postprocess", "processAllTokens", "emStrongMask"])), k(ct, "passThroughHooksRespectAsync", /* @__PURE__ */ new Set(["preprocess", "postprocess", "processAllTokens"])), ct), oi = class {
  constructor(...r) {
    k(this, "defaults", nn());
    k(this, "options", this.setOptions);
    k(this, "parse", this.parseMarkdown(!0));
    k(this, "parseInline", this.parseMarkdown(!1));
    k(this, "Parser", J);
    k(this, "Renderer", pt);
    k(this, "TextRenderer", hn);
    k(this, "Lexer", Q);
    k(this, "Tokenizer", ht);
    k(this, "Hooks", Ve);
    this.use(...r);
  }
  walkTokens(r, e) {
    var n, s;
    let t = [];
    for (let a of r) switch (t = t.concat(e.call(this, a)), a.type) {
      case "table": {
        let c = a;
        for (let u of c.header) t = t.concat(this.walkTokens(u.tokens, e));
        for (let u of c.rows) for (let o of u) t = t.concat(this.walkTokens(o.tokens, e));
        break;
      }
      case "list": {
        let c = a;
        t = t.concat(this.walkTokens(c.items, e));
        break;
      }
      default: {
        let c = a;
        (s = (n = this.defaults.extensions) == null ? void 0 : n.childTokens) != null && s[c.type] ? this.defaults.extensions.childTokens[c.type].forEach((u) => {
          let o = c[u].flat(1 / 0);
          t = t.concat(this.walkTokens(o, e));
        }) : c.tokens && (t = t.concat(this.walkTokens(c.tokens, e)));
      }
    }
    return t;
  }
  use(...r) {
    let e = this.defaults.extensions || { renderers: {}, childTokens: {} };
    return r.forEach((t) => {
      let n = { ...t };
      if (n.async = this.defaults.async || n.async || !1, t.extensions && (t.extensions.forEach((s) => {
        if (!s.name) throw new Error("extension name required");
        if ("renderer" in s) {
          let a = e.renderers[s.name];
          a ? e.renderers[s.name] = function(...c) {
            let u = s.renderer.apply(this, c);
            return u === !1 && (u = a.apply(this, c)), u;
          } : e.renderers[s.name] = s.renderer;
        }
        if ("tokenizer" in s) {
          if (!s.level || s.level !== "block" && s.level !== "inline") throw new Error("extension level must be 'block' or 'inline'");
          let a = e[s.level];
          a ? a.unshift(s.tokenizer) : e[s.level] = [s.tokenizer], s.start && (s.level === "block" ? e.startBlock ? e.startBlock.push(s.start) : e.startBlock = [s.start] : s.level === "inline" && (e.startInline ? e.startInline.push(s.start) : e.startInline = [s.start]));
        }
        "childTokens" in s && s.childTokens && (e.childTokens[s.name] = s.childTokens);
      }), n.extensions = e), t.renderer) {
        let s = this.defaults.renderer || new pt(this.defaults);
        for (let a in t.renderer) {
          if (!(a in s)) throw new Error(`renderer '${a}' does not exist`);
          if (["options", "parser"].includes(a)) continue;
          let c = a, u = t.renderer[c], o = s[c];
          s[c] = (...g) => {
            let f = u.apply(s, g);
            return f === !1 && (f = o.apply(s, g)), f || "";
          };
        }
        n.renderer = s;
      }
      if (t.tokenizer) {
        let s = this.defaults.tokenizer || new ht(this.defaults);
        for (let a in t.tokenizer) {
          if (!(a in s)) throw new Error(`tokenizer '${a}' does not exist`);
          if (["options", "rules", "lexer"].includes(a)) continue;
          let c = a, u = t.tokenizer[c], o = s[c];
          s[c] = (...g) => {
            let f = u.apply(s, g);
            return f === !1 && (f = o.apply(s, g)), f;
          };
        }
        n.tokenizer = s;
      }
      if (t.hooks) {
        let s = this.defaults.hooks || new Ve();
        for (let a in t.hooks) {
          if (!(a in s)) throw new Error(`hook '${a}' does not exist`);
          if (["options", "block"].includes(a)) continue;
          let c = a, u = t.hooks[c], o = s[c];
          Ve.passThroughHooks.has(a) ? s[c] = (g) => {
            if (this.defaults.async && Ve.passThroughHooksRespectAsync.has(a)) return (async () => {
              let b = await u.call(s, g);
              return o.call(s, b);
            })();
            let f = u.call(s, g);
            return o.call(s, f);
          } : s[c] = (...g) => {
            if (this.defaults.async) return (async () => {
              let b = await u.apply(s, g);
              return b === !1 && (b = await o.apply(s, g)), b;
            })();
            let f = u.apply(s, g);
            return f === !1 && (f = o.apply(s, g)), f;
          };
        }
        n.hooks = s;
      }
      if (t.walkTokens) {
        let s = this.defaults.walkTokens, a = t.walkTokens;
        n.walkTokens = function(c) {
          let u = [];
          return u.push(a.call(this, c)), s && (u = u.concat(s.call(this, c))), u;
        };
      }
      this.defaults = { ...this.defaults, ...n };
    }), this;
  }
  setOptions(r) {
    return this.defaults = { ...this.defaults, ...r }, this;
  }
  lexer(r, e) {
    return Q.lex(r, e ?? this.defaults);
  }
  parser(r, e) {
    return J.parse(r, e ?? this.defaults);
  }
  parseMarkdown(r) {
    return (e, t) => {
      let n = { ...t }, s = { ...this.defaults, ...n }, a = this.onError(!!s.silent, !!s.async);
      if (this.defaults.async === !0 && n.async === !1) return a(new Error("marked(): The async option was set to true by an extension. Remove async: false from the parse options object to return a Promise."));
      if (typeof e > "u" || e === null) return a(new Error("marked(): input parameter is undefined or null"));
      if (typeof e != "string") return a(new Error("marked(): input parameter is of type " + Object.prototype.toString.call(e) + ", string expected"));
      if (s.hooks && (s.hooks.options = s, s.hooks.block = r), s.async) return (async () => {
        let c = s.hooks ? await s.hooks.preprocess(e) : e, u = await (s.hooks ? await s.hooks.provideLexer(r) : r ? Q.lex : Q.lexInline)(c, s), o = s.hooks ? await s.hooks.processAllTokens(u) : u;
        s.walkTokens && await Promise.all(this.walkTokens(o, s.walkTokens));
        let g = await (s.hooks ? await s.hooks.provideParser(r) : r ? J.parse : J.parseInline)(o, s);
        return s.hooks ? await s.hooks.postprocess(g) : g;
      })().catch(a);
      try {
        s.hooks && (e = s.hooks.preprocess(e));
        let c = (s.hooks ? s.hooks.provideLexer(r) : r ? Q.lex : Q.lexInline)(e, s);
        s.hooks && (c = s.hooks.processAllTokens(c)), s.walkTokens && this.walkTokens(c, s.walkTokens);
        let u = (s.hooks ? s.hooks.provideParser(r) : r ? J.parse : J.parseInline)(c, s);
        return s.hooks && (u = s.hooks.postprocess(u)), u;
      } catch (c) {
        return a(c);
      }
    };
  }
  onError(r, e) {
    return (t) => {
      if (t.message += `
Please report this to https://github.com/markedjs/marked.`, r) {
        let n = "<p>An error occurred:</p><pre>" + ie(t.message + "", !0) + "</pre>";
        return e ? Promise.resolve(n) : n;
      }
      if (e) return Promise.reject(t);
      throw t;
    };
  }
}, xe = new oi();
function v(r, e) {
  return xe.parse(r, e);
}
v.options = v.setOptions = function(r) {
  return xe.setOptions(r), v.defaults = xe.defaults, gs(v.defaults), v;
};
v.getDefaults = nn;
v.defaults = Te;
v.use = function(...r) {
  return xe.use(...r), v.defaults = xe.defaults, gs(v.defaults), v;
};
v.walkTokens = function(r, e) {
  return xe.walkTokens(r, e);
};
v.parseInline = xe.parseInline;
v.Parser = J;
v.parser = J.parse;
v.Renderer = pt;
v.TextRenderer = hn;
v.Lexer = Q;
v.lexer = Q.lex;
v.Tokenizer = ht;
v.Hooks = Ve;
v.parse = v;
v.options;
v.setOptions;
v.use;
v.walkTokens;
v.parseInline;
J.parse;
Q.lex;
/*! @license DOMPurify 3.4.17 | (c) Cure53 and other contributors | Released under the Apache license 2.0 and Mozilla Public License 2.0 | github.com/cure53/DOMPurify/blob/3.4.17/LICENSE */
function Jn(r, e) {
  (e == null || e > r.length) && (e = r.length);
  for (var t = 0, n = Array(e); t < e; t++) n[t] = r[t];
  return n;
}
function li(r) {
  if (Array.isArray(r)) return r;
}
function ci(r, e) {
  var t = r == null ? null : typeof Symbol < "u" && r[Symbol.iterator] || r["@@iterator"];
  if (t != null) {
    var n, s, a, c, u = [], o = !0, g = !1;
    try {
      if (a = (t = t.call(r)).next, e !== 0) for (; !(o = (n = a.call(t)).done) && (u.push(n.value), u.length !== e); o = !0) ;
    } catch (f) {
      g = !0, s = f;
    } finally {
      try {
        if (!o && t.return != null && (c = t.return(), Object(c) !== c)) return;
      } finally {
        if (g) throw s;
      }
    }
    return u;
  }
}
function ui() {
  throw new TypeError(`Invalid attempt to destructure non-iterable instance.
In order to be iterable, non-array objects must have a [Symbol.iterator]() method.`);
}
/*! regenerator-runtime -- Copyright (c) 2014-present, Facebook, Inc. -- license (MIT): https://github.com/babel/babel/blob/main/packages/babel-helpers/LICENSE */
function hi(r, e) {
  return li(r) || ci(r, e) || pi(r, e) || ui();
}
function pi(r, e) {
  if (r) {
    if (typeof r == "string") return Jn(r, e);
    var t = {}.toString.call(r).slice(8, -1);
    return t === "Object" && r.constructor && (t = r.constructor.name), t === "Map" || t === "Set" ? Array.from(r) : t === "Arguments" || /^(?:Ui|I)nt(?:8|16|32)(?:Clamped)?Array$/.test(t) ? Jn(r, e) : void 0;
  }
}
const vs = Object.entries, es = Object.setPrototypeOf, fi = Object.isFrozen, As = Object.getPrototypeOf, Rs = Object.getOwnPropertyDescriptor;
let N = Object.freeze, F = Object.seal, Ie = Object.create, Ls = typeof Reflect < "u" && Reflect, en = Ls.apply, tn = Ls.construct;
N || (N = function(e) {
  return e;
});
F || (F = function(e) {
  return e;
});
en || (en = function(e, t) {
  for (var n = arguments.length, s = new Array(n > 2 ? n - 2 : 0), a = 2; a < n; a++) s[a - 2] = arguments[a];
  return e.apply(t, s);
});
tn || (tn = function(e) {
  for (var t = arguments.length, n = new Array(t > 1 ? t - 1 : 0), s = 1; s < t; s++) n[s - 1] = arguments[s];
  return new e(...n);
});
const le = O(Array.prototype.forEach), di = O(Array.prototype.lastIndexOf), ts = O(Array.prototype.pop), He = O(Array.prototype.push), mi = O(Array.prototype.slice), gi = O(Array.prototype.splice), De = Array.isArray, Wt = Os(Set), jt = O(Set.prototype.add), qe = O(Set.prototype.has), we = O(String.prototype.toLowerCase), Bt = O(String.prototype.toString), ns = O(String.prototype.match), Ge = O(String.prototype.replace), ss = O(String.prototype.indexOf), bi = O(String.prototype.trim), ki = O(Number.prototype.toString), _i = O(Boolean.prototype.toString), rs = typeof BigInt > "u" ? null : O(BigInt.prototype.toString), is = typeof Symbol > "u" ? null : O(Symbol.prototype.toString), q = O(Object.prototype.hasOwnProperty), Ye = O(Object.prototype.toString), z = O(RegExp.prototype.test), ce = Os(TypeError);
function O(r) {
  return function(e) {
    e instanceof RegExp && (e.lastIndex = 0);
    for (var t = arguments.length, n = new Array(t > 1 ? t - 1 : 0), s = 1; s < t; s++) n[s - 1] = arguments[s];
    return en(r, e, n);
  };
}
function Os(r) {
  return function() {
    for (var e = arguments.length, t = new Array(e), n = 0; n < e; n++) t[n] = arguments[n];
    return tn(r, t);
  };
}
function E(r, e) {
  let t = arguments.length > 2 && arguments[2] !== void 0 ? arguments[2] : we;
  if (es && es(r, null), !De(e)) return r;
  let n = e.length;
  for (; n--; ) {
    let s = e[n];
    if (typeof s == "string") {
      const a = t(s);
      a !== s && (fi(e) || (e[n] = a), s = a);
    }
    r[s] = !0;
  }
  return r;
}
function wi(r) {
  for (let e = 0; e < r.length; e++) q(r, e) || (r[e] = null);
  return r;
}
function G(r) {
  const e = Ie(null);
  for (const n of vs(r)) {
    var t = hi(n, 2);
    const s = t[0], a = t[1];
    q(r, s) && (De(a) ? e[s] = wi(a) : a && typeof a == "object" && a.constructor === Object ? e[s] = G(a) : e[s] = a);
  }
  return e;
}
function yi(r) {
  switch (typeof r) {
    case "string":
      return r;
    case "number":
      return ki(r);
    case "boolean":
      return _i(r);
    case "bigint":
      return rs ? rs(r) : "0";
    case "symbol":
      return is ? is(r) : "Symbol()";
    case "undefined":
      return Ye(r);
    case "function":
    case "object": {
      if (r === null) return Ye(r);
      const e = r, t = X(e, "toString");
      if (typeof t == "function") {
        const n = t(e);
        return typeof n == "string" ? n : Ye(n);
      }
      return Ye(r);
    }
    default:
      return Ye(r);
  }
}
function X(r, e) {
  for (; r !== null; ) {
    const n = Rs(r, e);
    if (n) {
      if (n.get) return O(n.get);
      if (typeof n.value == "function") return O(n.value);
    }
    r = As(r);
  }
  function t() {
    return null;
  }
  return t;
}
function xi(r, e) {
  for (; r !== null; ) {
    const t = Rs(r, e);
    if (t) return t.get ? O(t.get) : null;
    r = As(r);
  }
  return null;
}
function Ti(r) {
  try {
    return z(r, ""), !0;
  } catch {
    return !1;
  }
}
const as = N([
  "a",
  "abbr",
  "acronym",
  "address",
  "area",
  "article",
  "aside",
  "audio",
  "b",
  "bdi",
  "bdo",
  "big",
  "blink",
  "blockquote",
  "body",
  "br",
  "button",
  "canvas",
  "caption",
  "center",
  "cite",
  "code",
  "col",
  "colgroup",
  "content",
  "data",
  "datalist",
  "dd",
  "decorator",
  "del",
  "details",
  "dfn",
  "dialog",
  "dir",
  "div",
  "dl",
  "dt",
  "element",
  "em",
  "fieldset",
  "figcaption",
  "figure",
  "font",
  "footer",
  "form",
  "h1",
  "h2",
  "h3",
  "h4",
  "h5",
  "h6",
  "head",
  "header",
  "hgroup",
  "hr",
  "html",
  "i",
  "img",
  "input",
  "ins",
  "kbd",
  "label",
  "legend",
  "li",
  "main",
  "map",
  "mark",
  "marquee",
  "menu",
  "menuitem",
  "meter",
  "nav",
  "nobr",
  "ol",
  "optgroup",
  "option",
  "output",
  "p",
  "picture",
  "pre",
  "progress",
  "q",
  "rp",
  "rt",
  "ruby",
  "s",
  "samp",
  "search",
  "section",
  "select",
  "shadow",
  "slot",
  "small",
  "source",
  "spacer",
  "span",
  "strike",
  "strong",
  "style",
  "sub",
  "summary",
  "sup",
  "table",
  "tbody",
  "td",
  "template",
  "textarea",
  "tfoot",
  "th",
  "thead",
  "time",
  "tr",
  "track",
  "tt",
  "u",
  "ul",
  "var",
  "video",
  "wbr"
]), Ht = N([
  "svg",
  "a",
  "altglyph",
  "altglyphdef",
  "altglyphitem",
  "animatecolor",
  "animatemotion",
  "animatetransform",
  "circle",
  "clippath",
  "defs",
  "desc",
  "ellipse",
  "enterkeyhint",
  "exportparts",
  "filter",
  "font",
  "g",
  "glyph",
  "glyphref",
  "hkern",
  "image",
  "inputmode",
  "line",
  "lineargradient",
  "marker",
  "mask",
  "metadata",
  "mpath",
  "part",
  "path",
  "pattern",
  "polygon",
  "polyline",
  "radialgradient",
  "rect",
  "stop",
  "style",
  "switch",
  "symbol",
  "text",
  "textpath",
  "title",
  "tref",
  "tspan",
  "view",
  "vkern"
]), qt = N([
  "feBlend",
  "feColorMatrix",
  "feComponentTransfer",
  "feComposite",
  "feConvolveMatrix",
  "feDiffuseLighting",
  "feDisplacementMap",
  "feDistantLight",
  "feDropShadow",
  "feFlood",
  "feFuncA",
  "feFuncB",
  "feFuncG",
  "feFuncR",
  "feGaussianBlur",
  "feImage",
  "feMerge",
  "feMergeNode",
  "feMorphology",
  "feOffset",
  "fePointLight",
  "feSpecularLighting",
  "feSpotLight",
  "feTile",
  "feTurbulence"
]), Si = N([
  "animate",
  "color-profile",
  "cursor",
  "discard",
  "font-face",
  "font-face-format",
  "font-face-name",
  "font-face-src",
  "font-face-uri",
  "foreignobject",
  "hatch",
  "hatchpath",
  "mesh",
  "meshgradient",
  "meshpatch",
  "meshrow",
  "missing-glyph",
  "script",
  "set",
  "solidcolor",
  "unknown",
  "use"
]), Gt = N([
  "math",
  "menclose",
  "merror",
  "mfenced",
  "mfrac",
  "mglyph",
  "mi",
  "mlabeledtr",
  "mmultiscripts",
  "mn",
  "mo",
  "mover",
  "mpadded",
  "mphantom",
  "mroot",
  "mrow",
  "ms",
  "mspace",
  "msqrt",
  "mstyle",
  "msub",
  "msup",
  "msubsup",
  "mtable",
  "mtd",
  "mtext",
  "mtr",
  "munder",
  "munderover",
  "mprescripts"
]), Ei = N([
  "maction",
  "maligngroup",
  "malignmark",
  "mlongdiv",
  "mscarries",
  "mscarry",
  "msgroup",
  "mstack",
  "msline",
  "msrow",
  "semantics",
  "annotation",
  "annotation-xml",
  "mprescripts",
  "none"
]), os = N(["#text"]), ls = N([
  "accept",
  "action",
  "align",
  "alt",
  "autocapitalize",
  "autocomplete",
  "autopictureinpicture",
  "autoplay",
  "background",
  "bgcolor",
  "border",
  "capture",
  "cellpadding",
  "cellspacing",
  "checked",
  "cite",
  "class",
  "clear",
  "color",
  "cols",
  "colspan",
  "command",
  "commandfor",
  "controls",
  "controlslist",
  "coords",
  "crossorigin",
  "datetime",
  "decoding",
  "default",
  "dir",
  "disabled",
  "disablepictureinpicture",
  "disableremoteplayback",
  "download",
  "draggable",
  "enctype",
  "enterkeyhint",
  "exportparts",
  "face",
  "for",
  "headers",
  "height",
  "hidden",
  "high",
  "href",
  "hreflang",
  "id",
  "inert",
  "inputmode",
  "integrity",
  "ismap",
  "kind",
  "label",
  "lang",
  "list",
  "loading",
  "loop",
  "low",
  "max",
  "maxlength",
  "media",
  "method",
  "min",
  "minlength",
  "multiple",
  "muted",
  "name",
  "nonce",
  "noshade",
  "novalidate",
  "nowrap",
  "open",
  "optimum",
  "part",
  "pattern",
  "placeholder",
  "playsinline",
  "popover",
  "popovertarget",
  "popovertargetaction",
  "poster",
  "preload",
  "pubdate",
  "radiogroup",
  "readonly",
  "rel",
  "required",
  "rev",
  "reversed",
  "role",
  "rows",
  "rowspan",
  "spellcheck",
  "scope",
  "selected",
  "shape",
  "size",
  "sizes",
  "slot",
  "span",
  "srclang",
  "start",
  "src",
  "srcset",
  "step",
  "style",
  "summary",
  "tabindex",
  "title",
  "translate",
  "type",
  "usemap",
  "valign",
  "value",
  "width",
  "wrap",
  "xmlns"
]), Yt = N([
  "accent-height",
  "accumulate",
  "additive",
  "alignment-baseline",
  "amplitude",
  "ascent",
  "attributename",
  "attributetype",
  "azimuth",
  "basefrequency",
  "baseline-shift",
  "begin",
  "bias",
  "by",
  "class",
  "clip",
  "clippathunits",
  "clip-path",
  "clip-rule",
  "color",
  "color-interpolation",
  "color-interpolation-filters",
  "color-profile",
  "color-rendering",
  "cx",
  "cy",
  "d",
  "dx",
  "dy",
  "diffuseconstant",
  "direction",
  "display",
  "divisor",
  "dominant-baseline",
  "dur",
  "edgemode",
  "elevation",
  "end",
  "exponent",
  "fill",
  "fill-opacity",
  "fill-rule",
  "filter",
  "filterunits",
  "flood-color",
  "flood-opacity",
  "font-family",
  "font-size",
  "font-size-adjust",
  "font-stretch",
  "font-style",
  "font-variant",
  "font-weight",
  "fx",
  "fy",
  "g1",
  "g2",
  "glyph-name",
  "glyphref",
  "gradientunits",
  "gradienttransform",
  "height",
  "href",
  "id",
  "image-rendering",
  "in",
  "in2",
  "intercept",
  "k",
  "k1",
  "k2",
  "k3",
  "k4",
  "kerning",
  "keypoints",
  "keysplines",
  "keytimes",
  "lang",
  "lengthadjust",
  "letter-spacing",
  "kernelmatrix",
  "kernelunitlength",
  "lighting-color",
  "local",
  "marker-end",
  "marker-mid",
  "marker-start",
  "markerheight",
  "markerunits",
  "markerwidth",
  "maskcontentunits",
  "maskunits",
  "max",
  "mask",
  "mask-type",
  "media",
  "method",
  "mode",
  "min",
  "name",
  "numoctaves",
  "offset",
  "operator",
  "opacity",
  "order",
  "orient",
  "orientation",
  "origin",
  "overflow",
  "paint-order",
  "path",
  "pathlength",
  "patterncontentunits",
  "patterntransform",
  "patternunits",
  "pointer-events",
  "points",
  "preservealpha",
  "preserveaspectratio",
  "primitiveunits",
  "r",
  "rx",
  "ry",
  "radius",
  "refx",
  "refy",
  "repeatcount",
  "repeatdur",
  "restart",
  "result",
  "rotate",
  "scale",
  "seed",
  "shape-rendering",
  "slope",
  "specularconstant",
  "specularexponent",
  "spreadmethod",
  "startoffset",
  "stddeviation",
  "stitchtiles",
  "stop-color",
  "stop-opacity",
  "stroke-dasharray",
  "stroke-dashoffset",
  "stroke-linecap",
  "stroke-linejoin",
  "stroke-miterlimit",
  "stroke-opacity",
  "stroke",
  "stroke-width",
  "style",
  "surfacescale",
  "systemlanguage",
  "tabindex",
  "tablevalues",
  "targetx",
  "targety",
  "transform",
  "transform-origin",
  "text-anchor",
  "text-decoration",
  "text-orientation",
  "text-rendering",
  "textlength",
  "type",
  "u1",
  "u2",
  "unicode",
  "values",
  "vector-effect",
  "viewbox",
  "visibility",
  "version",
  "vert-adv-y",
  "vert-origin-x",
  "vert-origin-y",
  "width",
  "word-spacing",
  "wrap",
  "writing-mode",
  "xchannelselector",
  "ychannelselector",
  "x",
  "x1",
  "x2",
  "xmlns",
  "y",
  "y1",
  "y2",
  "z",
  "zoomandpan"
]), cs = N([
  "accent",
  "accentunder",
  "align",
  "bevelled",
  "close",
  "columnalign",
  "columnlines",
  "columnspacing",
  "columnspan",
  "denomalign",
  "depth",
  "dir",
  "display",
  "displaystyle",
  "encoding",
  "fence",
  "frame",
  "height",
  "href",
  "id",
  "largeop",
  "length",
  "linethickness",
  "lquote",
  "lspace",
  "mathbackground",
  "mathcolor",
  "mathsize",
  "mathvariant",
  "maxsize",
  "minsize",
  "movablelimits",
  "notation",
  "numalign",
  "open",
  "rowalign",
  "rowlines",
  "rowspacing",
  "rowspan",
  "rspace",
  "rquote",
  "scriptlevel",
  "scriptminsize",
  "scriptsizemultiplier",
  "selection",
  "separator",
  "separators",
  "stretchy",
  "subscriptshift",
  "supscriptshift",
  "symmetric",
  "voffset",
  "width",
  "xmlns"
]), lt = N([
  "xlink:href",
  "xml:id",
  "xlink:title",
  "xml:space",
  "xmlns:xlink"
]), vi = F(/{{[\w\W]*|^[\w\W]*}}/g), Ai = F(/<%[\w\W]*|^[\w\W]*%>/g), Ri = F(/\${[\w\W]*/g), Li = F(/^data-[\-\w.\u00B7-\uFFFF]+$/), Oi = F(/^aria-[\-\w]+$/), us = F(/^(?:(?:(?:f|ht)tps?|mailto|tel|callto|sms|cid|xmpp|matrix):|[^a-z]|[a-z+.\-]+(?:[^a-z+.\-:]|$))/i), Ci = F(/^(?:\w+script|data):/i), Pi = F(/[\u0000-\u0020\u00A0\u1680\u180E\u2000-\u2029\u205F\u3000]/g), Ii = F(/^html$/i), Di = F(/^[a-z][.\w]*(-[.\w]+)+$/i), Vt = F(/<[/\w!]/g), hs = F(/<[/\w]/g), Mi = F(/<\/no(script|embed|frames)/i), ps = F(/\/>/i), I = {
  element: 1,
  attribute: 2,
  text: 3,
  cdataSection: 4,
  entityReference: 5,
  entityNode: 6,
  processingInstruction: 7,
  comment: 8,
  document: 9,
  documentType: 10,
  documentFragment: 11,
  notation: 12
}, $i = N(E({}, [
  "style",
  "script",
  "xmp",
  "iframe",
  "noembed",
  "noframes",
  "plaintext",
  "noscript"
])), Ni = function() {
  return typeof window > "u" ? null : window;
}, zi = function(e, t) {
  if (typeof e != "object" || typeof e.createPolicy != "function") return null;
  let n = null;
  const s = "data-tt-policy-suffix";
  t && t.hasAttribute(s) && (n = t.getAttribute(s));
  const a = "dompurify" + (n ? "#" + n : "");
  try {
    return e.createPolicy(a, {
      createHTML(c) {
        return c;
      },
      createScriptURL(c) {
        return c;
      }
    });
  } catch {
    return console.warn("TrustedTypes policy " + a + " could not be created."), null;
  }
}, fs = function() {
  return {
    afterSanitizeAttributes: [],
    afterSanitizeElements: [],
    afterSanitizeShadowDOM: [],
    beforeSanitizeAttributes: [],
    beforeSanitizeElements: [],
    beforeSanitizeShadowDOM: [],
    uponSanitizeAttribute: [],
    uponSanitizeElement: [],
    uponSanitizeShadowNode: []
  };
}, de = function(e, t, n, s) {
  return q(e, t) && De(e[t]) ? E(s.base ? G(s.base) : {}, e[t], s.transform) : n;
}, Xt = function(e, t, n) {
  const s = q(e, t) ? e[t] : void 0;
  return s && typeof s == "object" ? G(s) : n();
};
function Cs() {
  let r = arguments.length > 0 && arguments[0] !== void 0 ? arguments[0] : Ni();
  const e = (m) => Cs(m);
  if (e.version = "3.4.17", e.removed = [], !r || !r.document || r.document.nodeType !== I.document || !r.Element)
    return e.isSupported = !1, e;
  let t = r.document;
  const n = t, s = n.currentScript;
  r.DocumentFragment;
  const a = r.HTMLTemplateElement, c = r.Node, u = r.Element, o = r.NodeFilter;
  r.NamedNodeMap === void 0 && (r.NamedNodeMap || r.MozNamedAttrMap), r.HTMLFormElement;
  const g = r.DOMParser, f = r.trustedTypes, b = u.prototype, d = X(b, "cloneNode"), T = X(b, "remove"), w = X(b, "removeAttributeNode"), C = X(b, "nextSibling"), $ = X(b, "childNodes"), D = X(b, "parentNode"), Y = X(b, "shadowRoot"), Z = X(b, "attributes"), ae = c && c.prototype ? X(c.prototype, "nodeType") : null, ee = c && c.prototype ? X(c.prototype, "nodeName") : null, te = c && c.prototype ? X(c.prototype, "ownerDocument") : null, V = function(i) {
    return ae ? ae(i) : i.nodeType;
  }, $e = function(i) {
    return ee ? ee(i) : i.nodeName;
  }, pn = xi(b, "namespaceURI"), mt = function(i) {
    return pn ? pn(i) : i.namespaceURI;
  };
  if (typeof a == "function") {
    const m = t.createElement("template");
    m.content && m.content.ownerDocument && (t = m.content.ownerDocument);
  }
  let j, me = "", gt, fn = !1, Ne = 0;
  const dn = function() {
    if (Ne > 0) throw ce('A configured TRUSTED_TYPES_POLICY callback (createHTML or createScriptURL) must not call DOMPurify.sanitize, as that causes infinite recursion. Do not pass a policy whose callbacks wrap DOMPurify as TRUSTED_TYPES_POLICY; see the "DOMPurify and Trusted Types" section of the README.');
  }, Se = function(i) {
    dn(), Ne++;
    try {
      return j.createHTML(i);
    } finally {
      Ne--;
    }
  }, Ps = function(i) {
    dn(), Ne++;
    try {
      return j.createScriptURL(i);
    } finally {
      Ne--;
    }
  }, Is = function() {
    return fn || (gt = zi(f, s), fn = !0), gt;
  }, Ze = t, bt = Ze.implementation, mn = Ze.createNodeIterator, Ds = Ze.createDocumentFragment, Ms = Ze.getElementsByTagName, $s = n.importNode;
  let A = fs();
  e.isSupported = typeof vs == "function" && typeof D == "function" && bt && bt.createHTMLDocument !== void 0;
  const Ns = vi, zs = Ai, Fs = Ri, Us = Li, Ws = Oi, js = Ci, gn = Pi, Bs = Di;
  let bn = us, R = null;
  const kt = E({}, [
    ...as,
    ...Ht,
    ...qt,
    ...Gt,
    ...os
  ]);
  let L = null;
  const _t = E({}, [
    ...ls,
    ...Yt,
    ...cs,
    ...lt
  ]);
  let ne = Object.seal(Ie(null, {
    tagNameCheck: {
      writable: !0,
      configurable: !1,
      enumerable: !0,
      value: null
    },
    attributeNameCheck: {
      writable: !0,
      configurable: !1,
      enumerable: !0,
      value: null
    },
    allowCustomizedBuiltInElements: {
      writable: !0,
      configurable: !1,
      enumerable: !0,
      value: !1
    }
  })), ze = null, kn = null;
  const ue = Object.seal(Ie(null, {
    tagCheck: {
      writable: !0,
      configurable: !1,
      enumerable: !0,
      value: null
    },
    attributeCheck: {
      writable: !0,
      configurable: !1,
      enumerable: !0,
      value: null
    }
  }));
  let _n = !0, wt = !0, wn = !1, yt = !0, oe = !1, ge = !0, be = !1, xt = !1, Ke = null, Qe = null, Tt = !1, Ee = !1, Je = !1, et = !1, yn = !0, xn = !1;
  const Tn = "user-content-";
  let St = !0, Et = !1, ke = null, ve = {}, _e = null;
  const Sn = E({}, [
    "annotation-xml",
    "audio",
    "colgroup",
    "desc",
    "foreignobject",
    "head",
    "iframe",
    "math",
    "mi",
    "mn",
    "mo",
    "ms",
    "mtext",
    "noembed",
    "noframes",
    "noscript",
    "plaintext",
    "script",
    "selectedcontent",
    "style",
    "svg",
    "template",
    "thead",
    "title",
    "video",
    "xmp"
  ]);
  let En = null;
  const vn = E({}, [
    "audio",
    "video",
    "img",
    "source",
    "image",
    "track"
  ]);
  let An = null;
  const Rn = E({}, [
    "alt",
    "class",
    "for",
    "id",
    "label",
    "name",
    "pattern",
    "placeholder",
    "role",
    "summary",
    "title",
    "value",
    "style",
    "xmlns"
  ]), tt = "http://www.w3.org/1998/Math/MathML", nt = "http://www.w3.org/2000/svg", se = "http://www.w3.org/1999/xhtml";
  let Ae = se, vt = !1, At = null;
  const Hs = E({}, [
    tt,
    nt,
    se
  ], Bt), Ln = N([
    "mi",
    "mo",
    "mn",
    "ms",
    "mtext"
  ]);
  let Rt = E({}, Ln);
  const On = N(["annotation-xml"]);
  let Lt = E({}, On);
  const qs = E({}, [
    "title",
    "style",
    "font",
    "a",
    "script"
  ]);
  let Fe = null;
  const Gs = ["application/xhtml+xml", "text/html"], Ys = "text/html";
  let P = null, Re = null;
  const Vs = t.createElement("form"), Cn = function(i) {
    return i instanceof RegExp || i instanceof Function;
  }, Ot = function() {
    let i = arguments.length > 0 && arguments[0] !== void 0 ? arguments[0] : {};
    if (Re && Re === i) return;
    (!i || typeof i != "object") && (i = {}), i = G(i), Fe = Gs.indexOf(i.PARSER_MEDIA_TYPE) === -1 ? Ys : i.PARSER_MEDIA_TYPE, P = Fe === "application/xhtml+xml" ? Bt : we, R = de(i, "ALLOWED_TAGS", kt, { transform: P }), L = de(i, "ALLOWED_ATTR", _t, { transform: P }), At = de(i, "ALLOWED_NAMESPACES", Hs, { transform: Bt }), An = de(i, "ADD_URI_SAFE_ATTR", Rn, {
      transform: P,
      base: Rn
    }), En = de(i, "ADD_DATA_URI_TAGS", vn, {
      transform: P,
      base: vn
    }), _e = de(i, "FORBID_CONTENTS", Sn, { transform: P }), ze = de(i, "FORBID_TAGS", G({}), { transform: P }), kn = de(i, "FORBID_ATTR", G({}), { transform: P }), ve = q(i, "USE_PROFILES") ? i.USE_PROFILES && typeof i.USE_PROFILES == "object" ? G(i.USE_PROFILES) : i.USE_PROFILES : !1, _n = i.ALLOW_ARIA_ATTR !== !1, wt = i.ALLOW_DATA_ATTR !== !1, wn = i.ALLOW_UNKNOWN_PROTOCOLS || !1, yt = i.ALLOW_SELF_CLOSE_IN_ATTR !== !1, oe = i.SAFE_FOR_TEMPLATES || !1, ge = i.SAFE_FOR_XML !== !1, be = i.WHOLE_DOCUMENT || !1, Ee = i.RETURN_DOM || !1, Je = i.RETURN_DOM_FRAGMENT || !1, et = i.RETURN_TRUSTED_TYPE || !1, Tt = i.FORCE_BODY || !1, yn = i.SANITIZE_DOM !== !1, xn = i.SANITIZE_NAMED_PROPS || !1, St = i.KEEP_CONTENT !== !1, Et = i.IN_PLACE || !1, bn = Ti(i.ALLOWED_URI_REGEXP) ? i.ALLOWED_URI_REGEXP : us, Ae = typeof i.NAMESPACE == "string" ? i.NAMESPACE : se, Rt = Xt(i, "MATHML_TEXT_INTEGRATION_POINTS", () => E({}, Ln)), Lt = Xt(i, "HTML_INTEGRATION_POINTS", () => E({}, On));
    const l = Xt(i, "CUSTOM_ELEMENT_HANDLING", () => Ie(null));
    if (ne = Ie(null), q(l, "tagNameCheck") && Cn(l.tagNameCheck) && (ne.tagNameCheck = l.tagNameCheck), q(l, "attributeNameCheck") && Cn(l.attributeNameCheck) && (ne.attributeNameCheck = l.attributeNameCheck), q(l, "allowCustomizedBuiltInElements") && typeof l.allowCustomizedBuiltInElements == "boolean" && (ne.allowCustomizedBuiltInElements = l.allowCustomizedBuiltInElements), F(ne), oe && (wt = !1), Je && (Ee = !0), ve && (R = E({}, os), L = Ie(null), ve.html === !0 && (E(R, as), E(L, ls)), ve.svg === !0 && (E(R, Ht), E(L, Yt), E(L, lt)), ve.svgFilters === !0 && (E(R, qt), E(L, Yt), E(L, lt)), ve.mathMl === !0 && (E(R, Gt), E(L, cs), E(L, lt))), ue.tagCheck = null, ue.attributeCheck = null, q(i, "ADD_TAGS") && (typeof i.ADD_TAGS == "function" ? ue.tagCheck = i.ADD_TAGS : De(i.ADD_TAGS) && (R === kt && (R = G(R)), E(R, i.ADD_TAGS, P))), q(i, "ADD_ATTR") && (typeof i.ADD_ATTR == "function" ? ue.attributeCheck = i.ADD_ATTR : De(i.ADD_ATTR) && (L === _t && (L = G(L)), E(L, i.ADD_ATTR, P))), q(i, "ADD_FORBID_CONTENTS") && De(i.ADD_FORBID_CONTENTS) && (_e === Sn && (_e = G(_e)), E(_e, i.ADD_FORBID_CONTENTS, P)), St && (R["#text"] = !0), be && E(R, [
      "html",
      "head",
      "body"
    ]), R.table && (E(R, ["tbody"]), delete ze.tbody), i.TRUSTED_TYPES_POLICY) {
      if (typeof i.TRUSTED_TYPES_POLICY.createHTML != "function") throw ce('TRUSTED_TYPES_POLICY configuration option must provide a "createHTML" hook.');
      if (typeof i.TRUSTED_TYPES_POLICY.createScriptURL != "function") throw ce('TRUSTED_TYPES_POLICY configuration option must provide a "createScriptURL" hook.');
      const h = j;
      j = i.TRUSTED_TYPES_POLICY;
      try {
        me = Se("");
      } catch (p) {
        throw j = h, p;
      }
    } else i.TRUSTED_TYPES_POLICY === null ? (j = void 0, me = "") : (j === void 0 && (j = Is()), j && typeof me == "string" && (me = Se("")));
    N && N(i), Re = i;
  }, Pn = E({}, [
    ...Ht,
    ...qt,
    ...Si
  ]), In = E({}, [...Gt, ...Ei]), Xs = function(i, l, h) {
    return l.namespaceURI === se ? i === "svg" : l.namespaceURI === tt ? i === "svg" && (h === "annotation-xml" || Rt[h]) : !!Pn[i];
  }, Zs = function(i, l, h) {
    return l.namespaceURI === se ? i === "math" : l.namespaceURI === nt ? i === "math" && Lt[h] : !!In[i];
  }, Ks = function(i, l, h) {
    return l.namespaceURI === nt && !Lt[h] || l.namespaceURI === tt && !Rt[h] ? !1 : !In[i] && (qs[i] || !Pn[i]);
  }, Qs = function(i) {
    const l = mt(i), h = we($e(i)), p = D(i);
    let _, y;
    return p && V(p) === I.element ? (_ = { namespaceURI: mt(p) }, y = we($e(p))) : (_ = { namespaceURI: Ae }, y = "template"), At[l] ? l === nt ? Xs(h, _, y) : l === tt ? Zs(h, _, y) : l === se ? Ks(h, _, y) : !!(Fe === "application/xhtml+xml" && At[l]) : !1;
  }, he = function(i) {
    He(e.removed, { element: i });
    try {
      D(i).removeChild(i);
    } catch {
      if (T(i), !D(i)) throw ce("a node selected for removal could not be detached from its tree and cannot be safely returned; refusing to sanitize in place");
    }
  }, Dn = function(i, l, h) {
    try {
      w(i, l);
    } catch {
      try {
        i.removeAttribute(h);
      } catch {
      }
    }
  }, Ct = function(i) {
    It(i);
    const l = $(i);
    l && le(mi(l), (p) => {
      try {
        T(p);
      } catch {
      }
    });
    const h = V(i) === I.element ? Z(i) : null;
    if (h) for (let p = h.length - 1; p >= 0; --p) {
      const _ = h[p], y = _ && _.name;
      typeof y == "string" && Dn(i, _, y);
    }
  }, Le = function(i, l, h) {
    if (!h) try {
      h = l.getAttributeNode(i);
    } catch {
      h = null;
    }
    He(e.removed, {
      attribute: h || null,
      from: l
    });
    try {
      h ? w(l, h) : l.removeAttribute(i);
    } catch {
      try {
        l.removeAttribute(i);
      } catch {
      }
    }
    if (i === "is")
      if (Ee || Je) try {
        he(l);
      } catch {
      }
      else try {
        l.setAttribute(i, "");
      } catch {
      }
  }, Mn = function(i) {
    const l = Z(i);
    if (l)
      for (let h = l.length - 1; h >= 0; --h) {
        const p = l[h], _ = p && p.name;
        typeof _ != "string" || L[P(_)] || Dn(i, p, _);
      }
  }, Pt = function(i, l) {
    let h = arguments.length > 2 && arguments[2] !== void 0 ? arguments[2] : !1;
    const p = [i];
    for (; p.length > 0; ) {
      const _ = p.pop();
      if (l(_) === !1) continue;
      if (h && V(_) === I.element) {
        const x = Y(_);
        pe(x) && p.push(x);
      }
      const y = $(_);
      if (y) for (let x = y.length - 1; x >= 0; --x) p.push(y[x]);
    }
  }, It = function(i) {
    Pt(i, (l) => {
      V(l) === I.element && Mn(l);
    });
  }, Dt = function(i) {
    const l = [];
    return Pt(i, (h) => {
      l.push(h);
    }, !0), l;
  }, Js = function(i, l) {
    const h = Wt();
    le(Dt(i), (p) => {
      V(p) === I.element && jt(h, p);
    }), le(l, (p) => {
      V(p) === I.element && !qe(h, p) && Mn(p);
    });
  }, $n = function(i, l) {
    let h = !1;
    return le(e.removed, (p) => {
      p.element && (h = h || p.element === i, It(p.element));
    }), Js(i, l), h;
  }, er = 8, tr = function(i, l) {
    const h = ke;
    let p = null;
    const _ = () => (p || (p = Wt(), le(l, (y) => {
      jt(p, y);
    })), p);
    for (let y = 0; ; y++) {
      const x = Dt(i), M = [i];
      let re = !1;
      if (le(x, (W) => {
        V(W) === I.documentFragment ? He(M, W) : !re && !qe(h, W) && (re = qe(_(), W));
      }), !re) return;
      if (y >= er) throw ce("the tree kept changing during in-place sanitization; refusing to return it");
      le(M, (W, B) => {
        nr(W, B > 0, i, _());
      });
    }
  }, nr = function(i, l, h, p) {
    const _ = Mt(i);
    let y;
    for (; y = _.nextNode(); )
      !qe(p, y) || qe(ke, y) || (l && K(A.uponSanitizeShadowNode, y, null), Nt(y, h), zt(y, h), pe(y.content) && We(y.content));
  }, sr = function(i) {
    ke && jt(ke, i);
  }, Nn = function(i, l) {
    return ge ? i === "patchsrc" ? !0 : i === "for" && l !== "label" && l !== "output" : !1;
  }, rr = function(i) {
    ge && Pt(i, (l) => {
      const h = V(l);
      if (h === I.processingInstruction || h === I.comment && z(hs, l.data)) {
        try {
          T(l);
        } catch {
        }
        return !1;
      }
      if (h === I.element) {
        const p = l, _ = P($e(l));
        try {
          p.hasAttribute && p.hasAttribute("patchsrc") && p.removeAttribute("patchsrc"), p.hasAttribute && p.hasAttribute("for") && Nn("for", _) && p.removeAttribute("for");
        } catch {
        }
      }
      return !0;
    });
  }, zn = function(i) {
    let l = null, h = null;
    if (Tt) i = "<remove></remove>" + i;
    else {
      const y = ns(i, /^[\r\n\t ]+/);
      h = y && y[0];
    }
    Fe === "application/xhtml+xml" && Ae === se && (i = '<html xmlns="http://www.w3.org/1999/xhtml"><head></head><body>' + i + "</body></html>");
    const p = j ? Se(i) : i;
    if (Ae === se) try {
      l = new g().parseFromString(p, Fe);
    } catch {
    }
    if (!l || !l.documentElement) {
      l = bt.createDocument(Ae, "template", null);
      try {
        l.documentElement.innerHTML = vt ? me : p;
      } catch {
      }
    }
    const _ = l.body || l.documentElement;
    return i && h && _.insertBefore(t.createTextNode(h), _.childNodes[0] || null), Ae === se ? Ms.call(l, be ? "html" : "body")[0] : be ? l.documentElement : _;
  }, Mt = function(i) {
    const l = te ? te(i) : i.ownerDocument;
    return mn.call(l || i, i, o.SHOW_ELEMENT | o.SHOW_COMMENT | o.SHOW_TEXT | o.SHOW_PROCESSING_INSTRUCTION | o.SHOW_CDATA_SECTION, null);
  }, st = function(i) {
    return i = Ge(i, Ns, " "), i = Ge(i, zs, " "), i = Ge(i, Fs, " "), i;
  }, $t = function(i) {
    var l;
    Fn(i);
    const h = (l = i.querySelectorAll) === null || l === void 0 ? void 0 : l.call(i, "template");
    h && le(h, (p) => {
      pe(p.content) && $t(p.content);
    });
  }, Fn = function(i) {
    i.normalize();
    const l = te ? te(i) : i.ownerDocument, h = mn.call(l || i, i, o.SHOW_TEXT | o.SHOW_COMMENT | o.SHOW_CDATA_SECTION | o.SHOW_PROCESSING_INSTRUCTION, null);
    let p = h.nextNode();
    for (; p; )
      p.data = st(p.data), p = h.nextNode();
  }, rt = function(i) {
    const l = ee ? ee(i) : null;
    return typeof l != "string" || P(l) !== "form" ? !1 : typeof i.nodeName != "string" || typeof i.textContent != "string" || typeof i.removeChild != "function" || i.attributes !== Z(i) || typeof i.removeAttribute != "function" || typeof i.removeAttributeNode != "function" || typeof i.getAttributeNode != "function" || typeof i.setAttribute != "function" || typeof i.namespaceURI != "string" || typeof i.insertBefore != "function" || typeof i.hasChildNodes != "function" || i.nodeType !== ae(i) || i.childNodes !== $(i);
  }, pe = function(i) {
    if (!ae || typeof i != "object" || i === null) return !1;
    try {
      return ae(i) === I.documentFragment;
    } catch {
      return !1;
    }
  }, Ue = function(i) {
    if (!ae || typeof i != "object" || i === null) return !1;
    try {
      return typeof ae(i) == "number";
    } catch {
      return !1;
    }
  };
  function K(m, i, l) {
    m.length !== 0 && le(m, (h) => {
      h.call(e, i, l, Re);
    });
  }
  const ir = function(i, l) {
    return !!(ge && i.hasChildNodes() && !Ue(i.firstElementChild) && z(Vt, i.textContent) && z(Vt, i.innerHTML) || ge && $i[l] && V(i) === I.element && mt(i) === se && (Ue(i.firstElementChild) || typeof i.textContent == "string" && z(Vt, i.textContent)) || i.nodeType === I.processingInstruction || ge && i.nodeType === I.comment && z(hs, i.data));
  }, it = function(i, l) {
    if (i instanceof RegExp) return z(i, l);
    if (i instanceof Function) {
      for (var h = arguments.length, p = new Array(h > 2 ? h - 2 : 0), _ = 2; _ < h; _++) p[_ - 2] = arguments[_];
      return !!i(l, ...p);
    }
    return !1;
  }, ar = function(i, l, h) {
    if (!ze[l] && jn(l) && it(ne.tagNameCheck, l)) return !1;
    if (St && !_e[l] && !_e[we(l)]) {
      const p = D(i), _ = $(i);
      if (_ && p) {
        const y = _.length;
        for (let x = y - 1; x >= 0; --x) {
          const M = i === h ? d(_[x], !0) : _[x];
          p.insertBefore(M, C(i));
        }
      }
    }
    return he(i), !0;
  }, Un = function(i, l, h, p) {
    return i.length === 0 ? l : l === h || l === p ? G(l) : l;
  }, Oe = function(i, l) {
    return i === l || D(i) !== null ? !1 : (Et && It(i), !0);
  }, Nt = function(i, l) {
    if (sr(i), K(A.beforeSanitizeElements, i, null), Oe(i, l)) return !0;
    if (rt(i))
      return he(i), !0;
    const h = P($e(i));
    if (R = Un(A.uponSanitizeElement, R, kt, Ke), K(A.uponSanitizeElement, i, {
      tagName: h,
      allowedTags: R
    }), Oe(i, l)) return !0;
    if (ir(i, h))
      return he(i), !0;
    if (ze[h] || !(ue.tagCheck instanceof Function && ue.tagCheck(h)) && !R[h]) {
      const p = ar(i, h, l);
      return p === !1 && (K(A.afterSanitizeElements, i, null), Oe(i, l)) ? !0 : p;
    }
    if (V(i) === I.element && !Qs(i) || (h === "noscript" || h === "noembed" || h === "noframes") && z(Mi, i.innerHTML))
      return he(i), !0;
    if (oe && i.nodeType === I.text) {
      const p = st(i.textContent);
      i.textContent !== p && (He(e.removed, { element: i.cloneNode() }), i.textContent = p);
    }
    return K(A.afterSanitizeElements, i, null), Oe(i, l);
  }, Wn = function(i, l, h) {
    if (kn[l] || Nn(l, i) || yn && (l === "id" || l === "name") && (h in t || h in Vs)) return !1;
    const p = L[l] || ue.attributeCheck instanceof Function && ue.attributeCheck(l, i);
    return wt && z(Us, l) || _n && z(Ws, l) ? !0 : p ? An[l] || z(bn, Ge(h, gn, "")) || (l === "src" || l === "xlink:href" || l === "href") && i !== "script" && ss(h, "data:") === 0 && En[i] || wn && !z(js, Ge(h, gn, "")) ? !0 : !h : jn(i) && it(ne.tagNameCheck, i) && it(ne.attributeNameCheck, l, i) || l === "is" && ne.allowCustomizedBuiltInElements && it(ne.tagNameCheck, h);
  }, or = E({}, [
    "annotation-xml",
    "color-profile",
    "font-face",
    "font-face-format",
    "font-face-name",
    "font-face-src",
    "font-face-uri",
    "missing-glyph"
  ]), jn = function(i) {
    return !or[we(i)] && z(Bs, i);
  }, lr = function(i, l, h, p) {
    if (j && typeof f == "object" && typeof f.getAttributeType == "function" && !h) switch (f.getAttributeType(i, l)) {
      case "TrustedHTML":
        return Se(p);
      case "TrustedScriptURL":
        return Ps(p);
    }
    return p;
  }, cr = function(i, l, h, p) {
    try {
      return h ? i.setAttributeNS(h, l, p) : i.setAttribute(l, p), rt(i) ? (he(i), !1) : !0;
    } catch {
      return Le(l, i), !1;
    }
  }, Bn = function(i, l) {
    return ge && z(/((--!?|])>)|<\/(style|script|title|xmp|textarea|noscript|iframe|noembed|noframes)/i, l) ? !0 : we(i) === "attributename" && ns(l, "href") !== null;
  }, zt = function(i, l) {
    if (K(A.beforeSanitizeAttributes, i, null), Oe(i, l)) return;
    const h = i.attributes;
    if (!h || rt(i)) return;
    L = Un(A.uponSanitizeAttribute, L, _t, Qe);
    const p = {
      attrName: "",
      attrValue: "",
      keepAttr: !0,
      allowedAttributes: L,
      forceKeepAttr: void 0
    };
    let _ = h.length;
    const y = P(i.nodeName);
    for (; _--; ) {
      const x = h[_], M = x.name, re = x.namespaceURI, W = x.value, B = P(M), Ce = W;
      let H = M === "value" ? Ce : bi(Ce), Hn = !1;
      if (p.attrName = B, p.attrValue = H, p.keepAttr = !0, p.forceKeepAttr = void 0, K(A.uponSanitizeAttribute, i, p), H = p.attrValue, xn && (B === "id" || B === "name") && ss(H, Tn) !== 0 && (Le(M, i, x), H = Tn + H, Hn = !0), Bn(M, H)) {
        Le(M, i, x);
        continue;
      }
      if (!p.forceKeepAttr) {
        if (!p.keepAttr) {
          Le(M, i, x);
          continue;
        }
        if (!yt && z(ps, H)) {
          Le(M, i, x);
          continue;
        }
        if (oe && (H = st(H)), !Wn(y, B, H)) {
          Le(M, i, x);
          continue;
        }
        H = lr(y, B, re, H), H !== Ce && cr(i, M, re, H) && Hn && ts(e.removed);
      }
    }
    K(A.afterSanitizeAttributes, i, null), Oe(i, l);
  }, We = function(i) {
    let l = null;
    const h = Mt(i);
    for (K(A.beforeSanitizeShadowDOM, i, null); l = h.nextNode(); )
      if (K(A.uponSanitizeShadowNode, l, null), Nt(l, i), zt(l, i), pe(l.content) && We(l.content), V(l) === I.element) {
        const p = Y(l);
        pe(p) && (Ft(p), We(p));
      }
    oe && Fn(i), K(A.afterSanitizeShadowDOM, i, null);
  }, Ft = function(i) {
    const l = [{
      node: i,
      shadow: null
    }];
    for (; l.length > 0; ) {
      const h = l.pop();
      if (h.shadow) {
        We(h.shadow);
        continue;
      }
      const p = h.node, _ = V(p) === I.element, y = $(p);
      if (y) for (let x = y.length - 1; x >= 0; --x) l.push({
        node: y[x],
        shadow: null
      });
      if (_) {
        const x = ee ? ee(p) : null;
        if (typeof x == "string" && P(x) === "template") {
          const M = p.content;
          pe(M) && l.push({
            node: M,
            shadow: null
          });
        }
      }
      if (_) {
        const x = Y(p);
        pe(x) && l.push({
          node: null,
          shadow: x
        }, {
          node: x,
          shadow: null
        });
      }
    }
  };
  return e.sanitize = function(m) {
    let i = arguments.length > 1 && arguments[1] !== void 0 ? arguments[1] : {}, l = null, h = null, p = null, _ = null;
    if (vt = !m, vt && (m = "<!-->"), typeof m != "string" && !Ue(m) && (m = yi(m), typeof m != "string"))
      throw ce("dirty is not a string, aborting");
    if (!e.isSupported) return m;
    xt ? (R = Ke, L = Qe) : Ot(i), (A.uponSanitizeElement.length > 0 || A.uponSanitizeAttribute.length > 0) && (R = G(R)), A.uponSanitizeAttribute.length > 0 && (L = G(L)), e.removed = [];
    const y = Et && typeof m != "string" && Ue(m);
    let x = [];
    const M = ke;
    if (y) {
      rr(m);
      const B = $e(m);
      if (typeof B == "string") {
        const Ce = P(B);
        if (!R[Ce] || ze[Ce])
          throw Ct(m), ce("root node is forbidden and cannot be sanitized in-place");
      }
      if (rt(m))
        throw Ct(m), ce("root node is clobbered and cannot be sanitized in-place");
      x = Dt(m), ke = Wt();
    } else if (Ue(m))
      l = zn("<!---->"), h = l.ownerDocument.importNode(m, !0), h.nodeType === I.element && h.nodeName === "BODY" || h.nodeName === "HTML" ? l = h : l.appendChild(h), Ft(l);
    else {
      if (!Ee && !oe && !be && m.indexOf("<") === -1) return j && et ? Se(m) : m;
      if (l = zn(m), !l) return Ee ? null : et ? me : "";
    }
    l && Tt && he(l.firstChild);
    const re = y ? m : l;
    try {
      y && Ft(m);
      const B = Mt(re);
      for (; p = B.nextNode(); )
        Nt(p, re), zt(p, re), pe(p.content) && We(p.content);
      y && tr(m, x);
    } catch (B) {
      throw y && (Ct(m), $n(m, x)), B;
    } finally {
      ke = M;
    }
    if (y) {
      if ($n(m, x)) throw ce("a node selected for removal could not be safely returned; refusing to sanitize in place");
      return oe && $t(m), m;
    }
    if (Ee) {
      if (oe && $t(l), Je)
        for (_ = Ds.call(l.ownerDocument); l.firstChild; ) _.appendChild(l.firstChild);
      else _ = l;
      return (L.shadowroot || L.shadowrootmode) && (_ = $s.call(n, _, !0)), _;
    }
    let W = be ? l.outerHTML : l.innerHTML;
    return be && R["!doctype"] && l.ownerDocument && l.ownerDocument.doctype && l.ownerDocument.doctype.name && z(Ii, l.ownerDocument.doctype.name) && (W = "<!DOCTYPE " + l.ownerDocument.doctype.name + `>
` + W), oe && (W = st(W)), j && et ? Se(W) : W;
  }, e.setConfig = function() {
    let m = arguments.length > 0 && arguments[0] !== void 0 ? arguments[0] : {};
    Ot(m), xt = !0, Ke = R, Qe = L;
  }, e.clearConfig = function() {
    Re = null, xt = !1, Ke = null, Qe = null, j = gt, me = "";
  }, e.isValidAttribute = function(m, i, l) {
    Re || Ot({});
    const h = P(m), p = P(i);
    return typeof l == "string" && (Bn(i, l) || !yt && z(ps, l)) ? !1 : Wn(h, p, l);
  }, e.addHook = function(m, i) {
    typeof i == "function" && q(A, m) && He(A[m], i);
  }, e.removeHook = function(m, i) {
    if (q(A, m)) {
      if (i !== void 0) {
        const l = di(A[m], i);
        return l === -1 ? void 0 : gi(A[m], l, 1)[0];
      }
      return ts(A[m]);
    }
  }, e.removeHooks = function(m) {
    q(A, m) && (A[m] = []);
  }, e.removeAllHooks = function() {
    A = fs();
  }, e;
}
var Fi = Cs();
class ds {
  constructor(e, t) {
    k(this, "container");
    k(this, "viewer");
    k(this, "output");
    k(this, "input");
    k(this, "history", []);
    k(this, "historyIdx", -1);
    this.container = e, this.viewer = t, e.innerHTML = `
      <div class="repl-header">Command Line</div>
      <div class="repl-output"></div>
      <div class="repl-input-row">
        <span class="repl-prompt">Patinae&gt;</span>
        <input class="repl-input" type="text" placeholder="Type a command..." spellcheck="false" autocomplete="off" />
      </div>
    `, this.output = e.querySelector(".repl-output"), this.input = e.querySelector(".repl-input"), this.input.addEventListener("keydown", (n) => this.onKey(n));
  }
  async onKey(e) {
    if (e.key === "Enter") {
      const t = this.input.value.trim();
      if (!t) return;
      this.history.push(t), this.historyIdx = this.history.length, this.input.value = "", this.appendLine(`Patinae> ${t}`, "cmd");
      const n = await this.viewer.execute(t);
      for (const s of n.messages)
        this.appendOutputMessage({ level: s.kind.toLowerCase(), text: s.text, format: s.format });
    } else e.key === "ArrowUp" ? (e.preventDefault(), this.historyIdx > 0 && (this.historyIdx--, this.input.value = this.history[this.historyIdx])) : e.key === "ArrowDown" && (e.preventDefault(), this.historyIdx < this.history.length - 1 ? (this.historyIdx++, this.input.value = this.history[this.historyIdx]) : (this.historyIdx = this.history.length, this.input.value = ""));
  }
  appendOutputMessage(e) {
    if (e.level === "clear") {
      this.clearOutput();
      return;
    }
    this.appendLine(e.text, e.level, e.format);
  }
  appendLine(e, t, n = "text") {
    const s = document.createElement("div");
    if (s.className = `repl-line repl-${t}`, n === "markdown") {
      s.classList.add("repl-markdown"), s.innerHTML = Fi.sanitize(v.parse(e, { async: !1 }), {
        USE_PROFILES: { html: !0 },
        FORBID_TAGS: ["img", "video", "audio", "iframe", "style"],
        FORBID_ATTR: ["style"]
      });
      for (const a of s.querySelectorAll("a"))
        a.target = "_blank", a.rel = "noopener noreferrer";
    } else
      s.textContent = e;
    this.output.appendChild(s), this.output.scrollTop = this.output.scrollHeight;
  }
  clearOutput() {
    this.output.replaceChildren();
  }
  update() {
  }
  destroy() {
    this.container.innerHTML = "";
  }
}
const Ui = ["lines", "sticks", "cartoon", "spheres", "surface"];
class Wi {
  constructor(e, t) {
    k(this, "container");
    k(this, "viewer");
    k(this, "list");
    k(this, "selectedObject", null);
    this.container = e, this.viewer = t, e.innerHTML = `
      <div class="panel-header">Objects</div>
      <div class="object-list"></div>
    `, this.list = e.querySelector(".object-list");
  }
  update() {
    const e = this.viewer.getObjectInfos();
    this.list.innerHTML = "";
    for (const n of e) {
      const s = n.name, a = document.createElement("div");
      a.className = `object-row${n.parent_group ? " grouped" : ""}${this.selectedObject === s ? " selected" : ""}`, a.dataset.objectName = s, a.setAttribute("role", "button"), a.setAttribute("aria-selected", String(this.selectedObject === s)), n.focus_disabled_reason && a.setAttribute("aria-description", n.focus_disabled_reason), a.tabIndex = 0, a.addEventListener("click", (b) => {
        b.target.closest("button, select") || this.selectObject(s);
      }), a.addEventListener("keydown", (b) => {
        (b.key === "Enter" || b.key === " ") && (b.preventDefault(), this.selectObject(s));
      }), a.addEventListener("dblclick", () => {
        n.can_focus && this.viewer.execute(`zoom ${s}`);
      });
      const c = document.createElement("button");
      c.className = `obj-vis ${n.enabled ? "enabled" : "disabled"}`, c.textContent = n.enabled ? "V" : "-", c.title = n.enabled ? "Hide" : "Show", c.addEventListener("click", () => {
        this.viewer.execute(n.enabled ? `disable ${s}` : `enable ${s}`);
      });
      const u = document.createElement("span");
      u.className = `obj-kind obj-kind-${Bi(n)}`, n.multicolor && u.classList.add("multicolor"), u.textContent = Hi(n), u.title = Zt(n), u.setAttribute("aria-label", Zt(n)), u.style.color = qi(n.color);
      const o = document.createElement("span");
      o.className = "obj-name", o.textContent = s, o.title = [
        `${s} — ${Zt(n)}`,
        n.parent_group ? `Group: ${n.parent_group}` : null,
        n.focus_disabled_reason
      ].filter(Boolean).join(" · ");
      const g = document.createElement("span");
      if (g.className = "obj-metadata", n.object_type === "molecule" ? (g.textContent = n.storage_mode === "instanced" ? `${n.displayed_atom_count} (${n.instance_count} copies)` : `${n.atom_count}`, g.title = n.storage_mode === "instanced" ? `${n.atom_count} stored atoms · ${n.displayed_atom_count} displayed atoms · shared colors and representations` : `${n.atom_count} atoms`) : n.object_type === "measurement" ? (g.textContent = `${n.entity_count}`, g.title = `${n.entity_count} measurement ${ms(n.entity_count, "entity", "entities")}`) : n.object_type === "label" ? (g.textContent = `${n.entity_count}`, g.title = `${n.entity_count} ${ms(n.entity_count, "label", "labels")}`) : g.textContent = "map", a.appendChild(c), a.appendChild(u), a.appendChild(o), a.appendChild(g), n.has_unresolved_entities) {
        const b = document.createElement("span");
        b.className = "obj-warning", b.textContent = "!", b.title = "One or more annotation entities are unresolved", b.setAttribute("aria-label", b.title), a.appendChild(b);
      }
      if (n.has_representations) {
        const b = document.createElement("span");
        b.className = "obj-reps";
        for (const d of Ui) {
          const T = document.createElement("button");
          T.className = "rep-btn", T.textContent = d.charAt(0).toUpperCase(), T.title = d, T.addEventListener("click", () => {
            this.viewer.execute(`show ${d}, ${s}`);
          }), b.appendChild(T);
        }
        a.appendChild(b);
      }
      const f = ji(n, (b) => {
        this.runObjectAction(n, b);
      });
      f && a.appendChild(f), this.list.appendChild(a);
    }
    const t = this.viewer.getSelectionList();
    if (t.length > 0) {
      const n = document.createElement("hr");
      n.className = "object-list-separator", this.list.appendChild(n);
      for (const s of t) {
        const a = document.createElement("div");
        a.className = "object-row selection-row";
        const c = document.createElement("button");
        c.className = `obj-vis selection-vis ${s.visible ? "enabled" : "disabled"}`, c.textContent = s.visible ? "V" : "-", c.title = s.visible ? "Hide indicators" : "Show indicators", c.addEventListener("click", () => {
          this.viewer.execute(`toggle ${s.name}`);
        });
        const u = document.createElement("span");
        u.className = "obj-name selection-name", u.textContent = `(${s.name})`, u.title = s.expression;
        const o = document.createElement("button");
        o.className = "rep-btn selection-delete", o.textContent = "X", o.title = "Delete selection", o.addEventListener("click", () => {
          this.viewer.execute(`deselect ${s.name}`);
        }), a.appendChild(c), a.appendChild(u), a.appendChild(o), this.list.appendChild(a);
      }
    }
  }
  destroy() {
    this.container.innerHTML = "";
  }
  selectObject(e) {
    this.selectedObject = e;
    for (const t of this.list.querySelectorAll(".object-row[data-object-name]")) {
      const n = t.dataset.objectName === e;
      t.classList.toggle("selected", n), t.setAttribute("aria-selected", String(n));
    }
  }
  runObjectAction(e, t) {
    var n, s, a;
    switch (t) {
      case "focus":
        this.viewer.execute(`zoom ${e.name}`);
        break;
      case "color": {
        const c = (n = window.prompt(`Color for ${e.name}:`, "cyan")) == null ? void 0 : n.trim();
        c && this.viewer.execute(`color ${c}, ${e.name}`);
        break;
      }
      case "rename": {
        const c = (s = window.prompt(`Rename ${e.name}:`, e.name)) == null ? void 0 : s.trim();
        c && c !== e.name && (this.viewer.execute(`set_name ${e.name}, ${c}`), this.selectedObject === e.name && (this.selectedObject = c));
        break;
      }
      case "group": {
        const c = (a = window.prompt(`Add ${e.name} to group:`)) == null ? void 0 : a.trim();
        c && this.viewer.execute(`group ${c}, ${e.name}, action=add`);
        break;
      }
      case "delete":
        window.confirm(`Delete ${e.name}?`) && (this.viewer.execute(`delete ${e.name}`), this.selectedObject === e.name && (this.selectedObject = null));
        break;
    }
  }
}
function ji(r, e) {
  const t = [];
  if (r.can_focus ? t.push(["focus", "Focus", !1]) : r.focus_disabled_reason && t.push(["focus", `Focus — ${r.focus_disabled_reason}`, !0]), r.can_color && t.push(["color", "Color…", !1]), r.can_rename && t.push(["rename", "Rename…", !1]), r.can_group && t.push(["group", "Add to group…", !1]), r.can_delete && t.push(["delete", "Delete…", !1]), t.length === 0) return null;
  const n = document.createElement("select");
  n.className = "obj-actions", n.title = `Actions for ${r.name}`, r.focus_disabled_reason && (n.title += ` · ${r.focus_disabled_reason}`), n.setAttribute("aria-label", n.title);
  const s = document.createElement("option");
  s.value = "", s.textContent = "•••", n.appendChild(s);
  for (const [a, c, u] of t) {
    const o = document.createElement("option");
    o.value = a, o.textContent = c, o.disabled = u, n.appendChild(o);
  }
  return n.addEventListener("change", () => {
    const a = n.value;
    n.value = "", a && e(a);
  }), n;
}
function Bi(r) {
  return r.object_type === "measurement" ? r.measurement_kind ?? "measurement" : r.object_type;
}
function Zt(r) {
  switch (r.object_type) {
    case "molecule":
      return "Molecule";
    case "map":
      return "Map";
    case "measurement":
      switch (r.measurement_kind) {
        case "distance":
          return "Distance measurement";
        case "angle":
          return "Angle measurement";
        case "dihedral":
          return "Dihedral measurement";
        default:
          return "Measurement";
      }
    case "label":
      return "Label collection";
  }
}
function Hi(r) {
  switch (r.object_type) {
    case "molecule":
      return "M";
    case "map":
      return "▦";
    case "measurement":
      switch (r.measurement_kind) {
        case "distance":
          return "↔";
        case "angle":
          return "∠";
        case "dihedral":
          return "⟳";
        default:
          return "⟷";
      }
    case "label":
      return "T";
  }
}
function qi(r) {
  const e = r.map((t) => Math.round(Math.min(1, Math.max(0, t)) * 255));
  return `rgb(${e[0]}, ${e[1]}, ${e[2]})`;
}
function ms(r, e, t) {
  return r === 1 ? e : t;
}
class Gi {
  constructor(e, t) {
    k(this, "container");
    k(this, "viewer");
    k(this, "content");
    this.container = e, this.viewer = t, e.innerHTML = `
      <div class="panel-header">Sequence</div>
      <div class="sequence-content"></div>
    `, this.content = e.querySelector(".sequence-content");
  }
  update() {
    const e = this.viewer.getSequenceData();
    this.content.innerHTML = "";
    for (const t of e) {
      const n = document.createElement("div");
      n.className = "seq-chain";
      const s = document.createElement("div");
      s.className = "seq-chain-header", s.textContent = `${t.object_name} / ${t.chain_id || "?"}`, n.appendChild(s);
      const a = document.createElement("div");
      a.className = "seq-residues";
      for (const c of t.residues) {
        const u = document.createElement("span");
        if (u.className = "seq-residue", u.textContent = c.one_letter, u.title = `${c.resn} ${c.resv}`, u.addEventListener("click", () => {
          const o = `${t.object_name} and chain ${t.chain_id} and resi ${c.resv}`;
          this.viewer.execute(`select sele, ${o}`);
        }), a.appendChild(u), c.resv % 10 === 0) {
          const o = document.createElement("span");
          o.className = "seq-spacer", o.textContent = " ", a.appendChild(o);
        }
      }
      n.appendChild(a), this.content.appendChild(n);
    }
  }
  destroy() {
    this.container.innerHTML = "";
  }
}
class Yi {
  constructor(e, t) {
    k(this, "container");
    k(this, "viewer");
    k(this, "frameLabel", null);
    k(this, "slider", null);
    k(this, "playBtn", null);
    this.container = e, this.viewer = t, e.innerHTML = `
      <div class="panel-header">Movie</div>
      <div class="movie-controls">
        <div class="movie-buttons">
          <button class="movie-btn" data-action="beginning" title="Beginning">|&lt;</button>
          <button class="movie-btn" data-action="backward" title="Step back">&lt;</button>
          <button class="movie-btn movie-play" data-action="play" title="Play/Pause">&#9654;</button>
          <button class="movie-btn" data-action="forward" title="Step forward">&gt;</button>
          <button class="movie-btn" data-action="end" title="End">&gt;|</button>
        </div>
        <div class="movie-timeline">
          <input type="range" class="movie-slider" min="1" max="1" value="1" />
          <span class="movie-frame-label">1 / 1</span>
        </div>
      </div>
    `, this.frameLabel = e.querySelector(".movie-frame-label"), this.slider = e.querySelector(".movie-slider"), this.playBtn = e.querySelector(".movie-play"), e.querySelectorAll(".movie-btn").forEach((n) => {
      n.addEventListener("click", () => {
        switch (n.dataset.action) {
          case "play":
            this.viewer.execute("mplay");
            break;
          case "beginning":
            this.viewer.execute("frame 1");
            break;
          case "end": {
            const a = this.viewer.getMovieState();
            this.viewer.execute(`frame ${a.frame_count}`);
            break;
          }
          case "forward":
            this.viewer.execute("forward");
            break;
          case "backward":
            this.viewer.execute("backward");
            break;
        }
      });
    }), this.slider.addEventListener("input", () => {
      const n = parseInt(this.slider.value, 10);
      this.viewer.execute(`frame ${n}`);
    });
  }
  update() {
    const e = this.viewer.getMovieState();
    this.slider && (this.slider.max = String(Math.max(1, e.frame_count)), this.slider.value = String(e.current_frame + 1)), this.frameLabel && (this.frameLabel.textContent = `${e.current_frame + 1} / ${Math.max(1, e.frame_count)}`), this.playBtn && (this.playBtn.textContent = e.is_playing ? "⏸" : "▶");
  }
  destroy() {
    this.container.innerHTML = "";
  }
}
class Vi {
  constructor(e, t = {}) {
    k(this, "core");
    k(this, "events", new kr());
    k(this, "panels", /* @__PURE__ */ new Map());
    k(this, "options");
    /** This facade reads the viewer's Rust registry; it stores no task state. */
    k(this, "tasks", {
      get: async (e) => this.core.requestWasm("get_task", (t) => t.get_task(e)),
      list: async (e = {}) => this.core.requestWasm("list_tasks", (t) => t.list_tasks(e)),
      cancel: async (e) => this.core.requestWasm("cancel_task", (t) => t.cancel_task(e)),
      wait: async (e, t) => await this.core.requestWasm("wait_task", (n) => n.wait_task(e, t))
    });
    this.options = t, this.core = new br(e), t.defer && this.core.setDeferred(!0, t.revealDuration ?? 150);
  }
  async init() {
    const e = await this.core.init({
      picking: this.options.picking ?? !1,
      selectionOverlay: this.options.selectionOverlay,
      memoryProfile: this.options.memoryProfile
    });
    this.options.picking && this.core.requireWasm("set_picking_enabled", (n) => n.set_picking_enabled(!0)), this.core.onOutput = (n) => this.emitRendererOutput(n), e.set_task_listener((n) => {
      for (const s of n) this.events.emit("tasks.changed", s);
      this.refreshPanels();
    }), this.core.onPick = (n) => {
      const s = n ?? {
        object_name: null,
        atom_index: null,
        chain: null,
        residue: null,
        expression: null
      };
      this.events.emit("atom-picked", s), this.refreshPanels();
    };
    const t = [];
    if (this.options.layout)
      t.push(...this.options.layout);
    else if (this.options.panels)
      for (const n of this.options.panels)
        t.push({ name: n, slot: "right" });
    for (const n of t) {
      let s;
      if (this.options.slots && (s = this.options.slots[n.slot]), s || (s = document.getElementById("sidebar")), !s) continue;
      const a = document.createElement("div");
      a.className = `patinae-panel patinae-panel-${n.name}`, n.collapsed && a.classList.add("collapsed"), s.appendChild(a);
      let c;
      switch (n.name) {
        case "repl":
          c = new ds(a, this);
          break;
        case "objects":
          c = new Wi(a, this);
          break;
        case "sequence":
          c = new Gi(a, this);
          break;
        case "movie":
          c = new Yi(a, this);
          break;
      }
      this.panels.set(n.name, c);
    }
    this.events.emit("ready", {});
  }
  // ---------------------------------------------------------------------------
  // Deferred display
  // ---------------------------------------------------------------------------
  get isDeferred() {
    return this.core.isDeferred;
  }
  async show() {
    await this.core.reveal();
  }
  // ---------------------------------------------------------------------------
  // Picking
  // ---------------------------------------------------------------------------
  /**
   * Enable or disable cursor-based atom picking at runtime.
   *
   * When enabled, left-click picks atoms, updates the `sele` selection, and
   * fires `atom-picked` events. Can also be set at construction time via
   * `ViewerOptions.picking`.
   */
  setPicking(e) {
    this.core.callWasm("set_picking_enabled", (t) => t.set_picking_enabled(e));
  }
  /** Enable or disable the visible selection / hover overlay at runtime. */
  setSelectionOverlay(e) {
    this.core.callWasm(
      "set_selection_overlay_enabled",
      (t) => t.set_selection_overlay_enabled(e)
    );
  }
  // ---------------------------------------------------------------------------
  // Commands
  // ---------------------------------------------------------------------------
  /** Execute once and retain IDs of accepted work, including on partial failure. */
  async execute(e) {
    const t = this.core.requireWasm("execute", (n) => n.execute(e));
    return this.emitCommandMessages(t.messages.map((n) => ({
      level: n.kind.toLowerCase(),
      text: n.text,
      format: n.format
    }))), this.refreshPanels(), t;
  }
  /** Byte parsing is synchronous and therefore does not create a task. */
  loadData(e, t, n) {
    const s = this.core.requireWasm("load_data", (a) => a.load_data(e, t, n));
    return this.refreshPanels(), s;
  }
  /** URL loading uses the common parser and returns accepted task identities. */
  async loadUrl(e, t) {
    let n = `load ${JSON.stringify(new URL(e, location.href).href)}`;
    return t != null && t.name && (n += `, object=${JSON.stringify(t.name)}`), t != null && t.format && (n += `, format=${JSON.stringify(t.format)}`), this.execute(n);
  }
  // ---------------------------------------------------------------------------
  // Queries
  // ---------------------------------------------------------------------------
  getObjectNames() {
    return this.core.queryWasm(
      "get_object_names",
      [],
      (e) => e.get_object_names()
    );
  }
  getObjectInfo(e) {
    return this.core.queryWasm(
      "get_object_info",
      null,
      (t) => t.get_object_info(e)
    );
  }
  getObjectInfos() {
    return this.core.queryWasm(
      "get_object_infos",
      [],
      (e) => e.get_object_infos()
    );
  }
  getSequenceData() {
    return this.core.queryWasm(
      "get_sequence_data",
      [],
      (e) => e.get_sequence_data()
    );
  }
  getMovieState() {
    return this.core.queryWasm(
      "get_movie_state",
      { frame_count: 0, current_frame: 0, is_playing: !1, rock_enabled: !1 },
      (e) => e.get_movie_state()
    );
  }
  getSelectionList() {
    return this.core.queryWasm(
      "get_selection_list",
      [],
      (e) => e.get_selection_list()
    );
  }
  getPerformanceSnapshot() {
    return this.core.getPerformanceSnapshot();
  }
  resetPerformanceStats() {
    this.core.resetPerformanceStats();
  }
  countAtoms(e = "all") {
    return this.core.requireWasm("count_atoms", (t) => t.count_atoms(e));
  }
  // ---------------------------------------------------------------------------
  // Events
  // ---------------------------------------------------------------------------
  on(e, t) {
    this.events.on(e, t);
  }
  off(e, t) {
    this.events.off(e, t);
  }
  // ---------------------------------------------------------------------------
  // Panel management
  // ---------------------------------------------------------------------------
  refreshPanels() {
    for (const e of this.panels.values())
      e.update();
    this.events.emit("objects-changed", { names: this.getObjectNames() });
  }
  emitCommandMessages(e) {
    for (const t of e)
      this.events.emit("command-output", t);
  }
  emitRendererOutput(e) {
    this.events.emit("command-output", e);
    const t = this.panels.get("repl");
    t instanceof ds && t.appendOutputMessage(e);
  }
  destroy() {
    for (const e of this.panels.values())
      e.destroy();
    this.panels.clear(), this.core.destroy();
  }
}
function Zi(r = "patinae-viewer") {
  customElements.get(r) || customElements.define(
    r,
    class extends HTMLElement {
      constructor() {
        super(...arguments);
        k(this, "viewer", null);
      }
      async connectedCallback() {
        var w;
        const t = this.attachShadow({ mode: "open" }), n = document.createElement("div");
        n.style.width = "100%", n.style.height = "100%", n.style.position = "relative", t.appendChild(n);
        const s = this.getAttribute("panels"), a = s ? s.split(",").map((C) => C.trim()) : [], c = this.getAttribute("src"), u = this.getAttribute("command"), o = this.getAttribute("defer"), g = o !== null ? o !== "false" : !!(c || u), f = this.getAttribute("selection-overlay"), b = f === null ? void 0 : f !== "false", d = (w = this.getAttribute("memory-profile")) == null ? void 0 : w.trim(), T = d === "performance" || d === "balanced" || d === "lite" || /^manual:[1-9]\d*$/.test(d ?? "") || d === "auto" ? d : void 0;
        if (this.viewer = new Vi(n, {
          panels: a,
          defer: g,
          selectionOverlay: b,
          memoryProfile: T
        }), await this.viewer.init(), c) {
          const C = c.split(/\s+/).filter(Boolean);
          for (const $ of C) {
            const D = await this.viewer.loadUrl($);
            if (await Promise.all(D.task_ids.map(async (Y) => {
              const Z = await this.viewer.tasks.wait(Y);
              if (Z.state !== "succeeded") throw new Error(`Task ${Y} ${Z.state}`);
            })), "Err" in D.result) throw new Error(D.result.Err);
          }
        }
        if (u)
          for (const C of u.split(";")) {
            const $ = C.trim();
            if ($) {
              const D = await this.viewer.execute($);
              if (await Promise.all(D.task_ids.map(async (Y) => {
                const Z = await this.viewer.tasks.wait(Y);
                if (Z.state !== "succeeded") throw new Error(`Task ${Y} ${Z.state}`);
              })), "Err" in D.result) throw new Error(D.result.Err);
            }
          }
        g && await this.viewer.show();
      }
      disconnectedCallback() {
        var t;
        (t = this.viewer) == null || t.destroy(), this.viewer = null;
      }
    }
  );
}
export {
  Vi as PatinaeViewer,
  Zi as registerElement
};

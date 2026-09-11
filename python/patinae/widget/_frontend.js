/**
 * anywidget ESM frontend for Patinae Jupyter widget.
 *
 * Loads WASM glue JS via blob URL and WASM binary from base64 traitlet.
 * This avoids file-serving issues across different Jupyter environments.
 */

function modBits(e) {
  let bits = 0;
  if (e.shiftKey) bits |= 1;
  if (e.ctrlKey) bits |= 2;
  if (e.altKey) bits |= 4;
  if (e.metaKey) bits |= 8;
  return bits;
}

const CLICK_THRESHOLD_SQ = 25;

/** `MouseEvent.button` indices we forward to the renderer: left, middle, right. */
const MOUSE_BUTTONS = [0, 1, 2];

/** `MouseEvent.buttons` bit corresponding to a `MouseEvent.button` index. */
function buttonBit(button) {
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

function decodeBase64(b64) {
  const bin = atob(b64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return bytes.buffer;
}

export default {
  async render({ model, el }) {
    // ── Container + Canvas ─────────────────────────────────────────
    const container = document.createElement("div");
    container.style.width = model.get("_width");
    container.style.height = model.get("_height");
    container.style.position = "relative";
    container.style.overflow = "hidden";
    container.style.background = "#000";
    el.appendChild(container);

    const canvas = document.createElement("canvas");
    canvas.id = "patinae-" + Math.random().toString(36).slice(2, 8);
    canvas.style.width = "100%";
    canvas.style.height = "100%";
    canvas.style.display = "block";
    canvas.tabIndex = 0;
    container.appendChild(canvas);

    const status = document.createElement("div");
    status.style.cssText =
      "position:absolute;top:50%;left:50%;transform:translate(-50%,-50%);" +
      "color:#888;font:14px sans-serif;text-align:center;";
    status.textContent = "Loading Patinae...";
    container.appendChild(status);

    // ── Load WASM ──────────────────────────────────────────────────
    let wasm = null;
    const viewId = crypto.randomUUID();
    let resolveInitialized;
    let rejectInitialized;
    const initialized = new Promise((resolve, reject) => {
      resolveInitialized = resolve;
      rejectInitialized = reject;
    });
    // Initialization can fail before any request arrives.
    initialized.catch(() => {});

    // Every operation has a response; execution and observation share Rust state.
    const onRequest = async (req, buffers = []) => {
      if (req.protocol !== 1 || req.id == null || req.view_id !== viewId) return;
      let result = null;
      let error = null;
      try {
        await initialized;
        if (!wasm) throw new Error("Widget frontend disconnected");
        const p = req.params || {};
        switch (req.method) {
          case "command_files": result = wasm.command_files(p.command, p.script_path); break;
          case "execute": {
            const files = Object.create(null);
            for (let i = 0; i < (p.files || []).length; i++) {
              const buffer = buffers[i];
              if (!buffer) throw new Error("Missing local file buffer");
              files[p.files[i]] = ArrayBuffer.isView(buffer)
                ? new Uint8Array(buffer.buffer, buffer.byteOffset, buffer.byteLength)
                : new Uint8Array(buffer);
            }
            result = await wasm.execute_with_files(p.command, files);
            break;
          }
          case "get_task": result = wasm.get_task(p.id); break;
          case "list_tasks": result = wasm.list_tasks(p); break;
          case "cancel_task": result = wasm.cancel_task(p.id); break;
          case "wait_task": result = await wasm.wait_task(p.id, p.timeout_ms); break;
          case "count_atoms": result = wasm.count_atoms(p.selection || "all"); break;
          case "get_names": result = wasm.get_object_names(); break;
          case "get_label": result = wasm.get_label_object(String(p.name || "")); break;
          case "get_movie_state": result = wasm.get_movie_state(); break;
          case "update_animations":
            wasm.update_animations(Number(p.dt || 0));
            result = wasm.needs_redraw();
            break;
          default: throw new Error("Unknown viewer method: " + req.method);
        }
      } catch (e) {
        error = e && typeof e === "object" && e.code ? e : { code: "request_failed", message: String(e) };
      }
      model.send({ protocol: 1, view_id: viewId, id: req.id, result, error });
    };
    model.on("msg:custom", onRequest);
    model.send({ protocol: 1, view_id: viewId, event: "ready" });


    try {
      // Import glue JS via blob URL
      const glueJs = model.get("_glue_js");
      if (!glueJs) throw new Error("WASM glue JS not received");

      const blob = new Blob([glueJs], { type: "application/javascript" });
      const blobUrl = URL.createObjectURL(blob);
      let glue;
      try {
        glue = await import(/* webpackIgnore: true */ blobUrl);
      } finally {
        URL.revokeObjectURL(blobUrl);
      }

      // Decode WASM binary from base64
      status.textContent = "Initializing WebGPU...";
      const wasmB64 = model.get("_wasm_b64");
      if (!wasmB64) throw new Error("WASM binary not received");

      const wasmBuf = decodeBase64(wasmB64);
      const wasmModule = await WebAssembly.compile(wasmBuf);
      glue.initSync(wasmModule);

      // Sync canvas pixel size before creating viewer
      const dpr = window.devicePixelRatio || 1;
      const rect = canvas.getBoundingClientRect();
      canvas.width = Math.round(rect.width * dpr);
      canvas.height = Math.round(rect.height * dpr);

      wasm = await glue.WebViewer.create(canvas.id);
      wasm.set_picking_enabled(model.get("_picking"));
      status.remove();
      resolveInitialized();
    } catch (err) {
      rejectInitialized(err);
      status.innerHTML =
        "<strong>Patinae widget failed to initialize.</strong><br><br>" +
        "Requires WebGPU (Chrome 113+, Edge 113+).<br><br>" +
        "<code>" + String(err) + "</code>";
      status.style.color = "#c00";
      return () => { model.off("msg:custom", onRequest); model.send({ protocol: 1, view_id: viewId, event: "disconnected" }); };
    }

    // ── Render loop ────────────────────────────────────────────────
    let animId = 0;
    let lastTime = performance.now();
    const loop = (now) => {
      if (!wasm) return;
      const dt = Math.min((now - lastTime) / 1000.0, 0.1);
      lastTime = now;
      try {
        wasm.process_input();
        wasm.update_animations(dt);
        if (wasm.needs_redraw()) {
          wasm.render_frame();
        }
      } catch (error) {
        console.error("Patinae rendering failed", error);
        status.textContent = "Patinae rendering failed: " + String(error);
        container.appendChild(status);
        model.send({ protocol: 1, view_id: viewId, event: "disconnected" });
        return;
      }
      animId = requestAnimationFrame(loop);
    };
    animId = requestAnimationFrame(loop);

    // ── Mouse events ───────────────────────────────────────────────
    let clickStart = null;
    let currentDpr = window.devicePixelRatio || 1;
    // `MouseEvent.buttons` bitmask the WASM side currently believes is pressed.
    let buttonMask = 0;

    // Release any button the renderer still holds but that `e.buttons` says is
    // no longer pressed. Pointer capture covers the common "released outside
    // the canvas" case, but capture can be denied or dropped (another element
    // captured first, OS-level focus loss); without this the renderer keeps the
    // button latched and the camera stays stuck in a drag.
    const reconcileButtons = (e) => {
      for (const button of MOUSE_BUTTONS) {
        const bit = buttonBit(button);
        if (!(buttonMask & bit) || e.buttons & bit) continue;
        buttonMask &= ~bit;
        wasm.on_mouse_up(e.offsetX, e.offsetY, button);
        if (button === 0) clickStart = null;
      }
    };

    canvas.addEventListener("pointerdown", (e) => {
      e.preventDefault();
      canvas.focus();
      // Capture the pointer so a drag that leaves the canvas keeps delivering
      // move/up events here instead of being swallowed by the document.
      try {
        canvas.setPointerCapture(e.pointerId);
      } catch {
        // Capture is best-effort; reconcileButtons() is the fallback.
      }
      reconcileButtons(e);
      buttonMask |= buttonBit(e.button);
      wasm.on_mouse_down(e.offsetX, e.offsetY, e.button, modBits(e));
      if (e.button === 0) clickStart = { x: e.offsetX, y: e.offsetY };
    });

    canvas.addEventListener("pointermove", (e) => {
      reconcileButtons(e);
      wasm.on_mouse_move(e.offsetX, e.offsetY, modBits(e));
      wasm.process_hover(e.offsetX * currentDpr, e.offsetY * currentDpr);
    });

    // Re-entering the canvas is the first chance to notice a button that was
    // released while the cursor was elsewhere.
    canvas.addEventListener("pointerenter", (e) => reconcileButtons(e));

    canvas.addEventListener("pointerup", (e) => {
      if (canvas.hasPointerCapture(e.pointerId)) {
        canvas.releasePointerCapture(e.pointerId);
      }
      buttonMask &= ~buttonBit(e.button);
      wasm.on_mouse_up(e.offsetX, e.offsetY, e.button);
      if (e.button === 0 && clickStart) {
        const dx = e.offsetX - clickStart.x;
        const dy = e.offsetY - clickStart.y;
        if (dx * dx + dy * dy < CLICK_THRESHOLD_SQ) {
          wasm.pick_at_screen(e.offsetX * currentDpr, e.offsetY * currentDpr);
        }
        clickStart = null;
      }
    });

    // The browser took the pointer away (touch gesture, OS-level drag, ...):
    // no pointerup is coming, so release everything we still hold.
    canvas.addEventListener("pointercancel", (e) => {
      if (canvas.hasPointerCapture(e.pointerId)) {
        canvas.releasePointerCapture(e.pointerId);
      }
      for (const button of MOUSE_BUTTONS) {
        if (buttonMask & buttonBit(button)) {
          wasm.on_mouse_up(e.offsetX, e.offsetY, button);
        }
      }
      buttonMask = 0;
      clickStart = null;
      wasm.process_hover(-1, -1);
    });

    canvas.addEventListener("mouseleave", () => {
      clickStart = null;
      wasm.process_hover(-1, -1);
    });

    canvas.addEventListener(
      "wheel",
      (e) => {
        e.preventDefault();
        wasm.on_wheel(e.deltaY, modBits(e));
      },
      { passive: false },
    );

    canvas.addEventListener("contextmenu", (e) => e.preventDefault());

    // ── Resize ─────────────────────────────────────────────────────
    const syncSize = () => {
      currentDpr = window.devicePixelRatio || 1;
      const rect = canvas.getBoundingClientRect();
      canvas.width = Math.round(rect.width * currentDpr);
      canvas.height = Math.round(rect.height * currentDpr);
      if (wasm) wasm.resize(canvas.width, canvas.height);
    };

    const ro = new ResizeObserver(syncSize);
    ro.observe(container);

    // ── Picking toggle ─────────────────────────────────────────────
    model.on("change:_picking", () => {
      if (wasm) wasm.set_picking_enabled(model.get("_picking"));
    });

    // ── Cleanup ────────────────────────────────────────────────────
    return () => {
      model.off("msg:custom", onRequest);
      model.send({ protocol: 1, view_id: viewId, event: "disconnected" });
      cancelAnimationFrame(animId);
      ro.disconnect();
      wasm = null;
      container.remove();
    };
  },
};

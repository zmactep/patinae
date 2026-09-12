/**
 * @patinae/viewer — embeddable WebGPU molecular visualization.
 *
 * Usage:
 *   import { PatinaeViewer } from "@patinae/viewer";
 *   const viewer = new PatinaeViewer(document.getElementById("container")!);
 *   await viewer.init();
 *   const reply = await viewer.execute("fetch 1CRN");
 *   await Promise.all(reply.task_ids.map(id => viewer.tasks.wait(id)));
 */

import type { RenderMemoryProfileOption } from "./core/types.js";

export { PatinaeViewer } from "./core/api.js";
export type {
  CommandOutput,
  TaskState,
  TaskEffects,
  TaskProgress,
  TaskDiagnostic,
  TaskOutcome,
  TaskSnapshot,
  TaskSummary,
  TaskListFilters,
  TaskListPage,
  TaskCancelReply,
  TaskChanged,
  OutputFormat,
  OutputMessage,
  ObjectInfo,
  SequenceChain,
  SequenceResidue,
  MovieState,
  ViewerOptions,
  ViewerPerformanceSnapshot,
  ViewerWasmPerformanceSnapshot,
  RenderMemoryProfileOption,
  PanelName,
  PanelSlot,
  PanelPlacement,
} from "./core/types.js";
export type { ViewerEventType, ViewerEventMap } from "./core/events.js";

// Custom element registration
import { PatinaeViewer } from "./core/api.js";

/**
 * Register `<patinae-viewer>` as a custom HTML element.
 *
 * Attributes:
 *   src               — URL(s) to load on mount (whitespace-separated for multiple)
 *   panels            — comma-separated panel names (repl, objects, sequence, movie)
 *   command           — command to run after loading
 *   selection-overlay — set to "false" to suppress selection/hover visuals
 *   memory-profile    — force "performance", "balanced", "lite", or "manual:<MiB>" at startup
 */
export function registerElement(tagName = "patinae-viewer"): void {
  if (customElements.get(tagName)) return;

  customElements.define(
    tagName,
    class extends HTMLElement {
      private viewer: PatinaeViewer | null = null;

      async connectedCallback() {
        const shadow = this.attachShadow({ mode: "open" });

        const wrapper = document.createElement("div");
        wrapper.style.width = "100%";
        wrapper.style.height = "100%";
        wrapper.style.position = "relative";
        shadow.appendChild(wrapper);

        const panelAttr = this.getAttribute("panels");
        const panels = panelAttr
          ? (panelAttr.split(",").map((s) => s.trim()) as Array<"repl" | "objects" | "sequence" | "movie">)
          : [];

        const src = this.getAttribute("src");
        const cmd = this.getAttribute("command");
        const deferAttr = this.getAttribute("defer");
        const shouldDefer = deferAttr !== null
          ? deferAttr !== "false"
          : !!(src || cmd);
        const selectionOverlayAttr = this.getAttribute("selection-overlay");
        const selectionOverlay = selectionOverlayAttr === null
          ? undefined
          : selectionOverlayAttr !== "false";
        const memoryProfileAttr = this.getAttribute("memory-profile")?.trim();
        const memoryProfile: RenderMemoryProfileOption | undefined = memoryProfileAttr === "performance"
          || memoryProfileAttr === "balanced"
          || memoryProfileAttr === "lite"
          || /^manual:[1-9]\d*$/.test(memoryProfileAttr ?? "")
          || memoryProfileAttr === "auto"
          ? memoryProfileAttr as RenderMemoryProfileOption
          : undefined;

        this.viewer = new PatinaeViewer(wrapper, {
          panels,
          defer: shouldDefer,
          selectionOverlay,
          memoryProfile,
        });
        await this.viewer.init();

        if (src) {
          const urls = src.split(/\s+/).filter(Boolean);
          for (const url of urls) {
            const reply = await this.viewer.loadUrl(url);
            await Promise.all(reply.task_ids.map(async id => {
              const task = await this.viewer!.tasks.wait(id);
              if (task.state !== "succeeded") throw new Error(`Task ${id} ${task.state}`);
            }));
            if ("Err" in reply.result) throw new Error(reply.result.Err);
          }
        }

        if (cmd) {
          // Split on semicolons to support multiple commands;
          // Wait for accepted tasks before revealing the initialized scene.
          for (const c of cmd.split(";")) {
            const trimmed = c.trim();
            if (trimmed) {
              const reply = await this.viewer.execute(trimmed);
              await Promise.all(reply.task_ids.map(async id => {
                const task = await this.viewer!.tasks.wait(id);
                if (task.state !== "succeeded") throw new Error(`Task ${id} ${task.state}`);
              }));
              if ("Err" in reply.result) throw new Error(reply.result.Err);
            }
          }
        }

        if (shouldDefer) {
          await this.viewer.show();
        }
      }

      disconnectedCallback() {
        this.viewer?.destroy();
        this.viewer = null;
      }
    }
  );
}

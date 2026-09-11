/**
 * Public API facade — wraps ViewerCore + panels into a clean interface.
 */

import { ViewerCore } from "./viewer.js";
import { ViewerEvents } from "./events.js";
import type {
  CommandOutput,
  TaskSnapshot,
  TaskListFilters,
  TaskListPage,
  TaskCancelReply,
  TaskChanged,
  ObjectInfo,
  PickHitInfo,
  SelectionInfo,
  SequenceChain,
  MovieState,
  OutputMessage,
  ViewerOptions,
  PanelName,
  PanelPlacement,
  ViewerPerformanceSnapshot,
} from "./types.js";
import type { ViewerEventType, ViewerEventMap } from "./events.js";
import { ReplPanel } from "../panels/repl.js";
import { ObjectListPanel } from "../panels/object-list.js";
import { SequencePanel } from "../panels/sequence.js";
import { MoviePanel } from "../panels/movie.js";

type PanelInstance = ReplPanel | ObjectListPanel | SequencePanel | MoviePanel;

export class PatinaeViewer {
  private core: ViewerCore;
  private events = new ViewerEvents();
  private panels = new Map<PanelName, PanelInstance>();
  private options: ViewerOptions;

  constructor(container: HTMLElement, options: ViewerOptions = {}) {
    this.options = options;
    this.core = new ViewerCore(container);
    if (options.defer) {
      this.core.setDeferred(true, options.revealDuration ?? 150);
    }
  }

  async init(): Promise<void> {
    // `options.picking` flows into WASM construction so the renderer
    // allocates (or skips) hit-test readback resources. Selection overlay
    // is a separate visual toggle and defaults to the picking choice.
    const wasm = await this.core.init({
      picking: this.options.picking ?? false,
      selectionOverlay: this.options.selectionOverlay,
      memoryProfile: this.options.memoryProfile,
    });

    // Arm the CPU-side hit-test flag so click/hover events trigger picks.
    if (this.options.picking) {
      this.core.requireWasm("set_picking_enabled", (wasm) => wasm.set_picking_enabled(true));
    }

    this.core.onOutput = (message) => this.emitRendererOutput(message);
    wasm.set_task_listener((changes: TaskChanged[]) => {
      for (const change of changes) this.events.emit("tasks.changed", change);
      this.refreshPanels();
    });

    // Forward pick results as typed events.
    this.core.onPick = (hit) => {
      const h = (hit as PickHitInfo | null) ?? {
        object_name: null,
        atom_index: null,
        chain: null,
        residue: null,
        expression: null,
      };
      this.events.emit("atom-picked", h);
      this.refreshPanels();
    };

    // Build the list of panels to mount
    const placements: PanelPlacement[] = [];

    if (this.options.layout) {
      placements.push(...this.options.layout);
    } else if (this.options.panels) {
      // Legacy mode — all panels go into a sidebar
      for (const name of this.options.panels) {
        placements.push({ name, slot: "right" });
      }
    }

    for (const placement of placements) {
      // Find the target container for this panel
      let target: HTMLElement | null | undefined;
      if (this.options.slots) {
        target = this.options.slots[placement.slot];
      }
      if (!target) {
        target = document.getElementById("sidebar");
      }
      if (!target) continue;

      const panelEl = document.createElement("div");
      panelEl.className = `patinae-panel patinae-panel-${placement.name}`;
      if (placement.collapsed) {
        panelEl.classList.add("collapsed");
      }
      target.appendChild(panelEl);

      let panel: PanelInstance;
      switch (placement.name) {
        case "repl":
          panel = new ReplPanel(panelEl, this);
          break;
        case "objects":
          panel = new ObjectListPanel(panelEl, this);
          break;
        case "sequence":
          panel = new SequencePanel(panelEl, this);
          break;
        case "movie":
          panel = new MoviePanel(panelEl, this);
          break;
      }
      this.panels.set(placement.name, panel);
    }

    this.events.emit("ready", {});
  }

  // ---------------------------------------------------------------------------
  // Deferred display
  // ---------------------------------------------------------------------------

  get isDeferred(): boolean {
    return this.core.isDeferred;
  }

  async show(): Promise<void> {
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
  setPicking(enabled: boolean): void {
    this.core.callWasm("set_picking_enabled", (wasm) => wasm.set_picking_enabled(enabled));
  }

  /** Enable or disable the visible selection / hover overlay at runtime. */
  setSelectionOverlay(enabled: boolean): void {
    this.core.callWasm("set_selection_overlay_enabled", (wasm) =>
      wasm.set_selection_overlay_enabled(enabled),
    );
  }

  // ---------------------------------------------------------------------------
  // Commands
  // ---------------------------------------------------------------------------

  /** Execute once and retain IDs of accepted work, including on partial failure. */
  async execute(command: string): Promise<CommandOutput> {
    const result = this.core.requireWasm("execute", (wasm) => wasm.execute(command) as CommandOutput);
    this.emitCommandMessages(result.messages.map(message => ({
      level: message.kind.toLowerCase() as OutputMessage["level"], text: message.text,
    })));
    this.refreshPanels();
    return result;
  }

  /** This facade reads the viewer's Rust registry; it stores no task state. */
  readonly tasks = {
    get: async (id: string): Promise<TaskSnapshot> =>
      this.core.requestWasm("get_task", wasm => wasm.get_task(id) as TaskSnapshot),
    list: async (filters: TaskListFilters = {}): Promise<TaskListPage> =>
      this.core.requestWasm("list_tasks", wasm => wasm.list_tasks(filters) as TaskListPage),
    cancel: async (id: string): Promise<TaskCancelReply> =>
      this.core.requestWasm("cancel_task", wasm => wasm.cancel_task(id) as TaskCancelReply),
    wait: async (id: string, timeoutMs?: number): Promise<TaskSnapshot> =>
      await this.core.requestWasm("wait_task", wasm => wasm.wait_task(id, timeoutMs)) as TaskSnapshot,
  };

  /** Byte parsing is synchronous and therefore does not create a task. */
  loadData(data: Uint8Array, name: string, format: string): CommandOutput {
    const result = this.core.requireWasm("load_data", (wasm) => wasm.load_data(data, name, format) as CommandOutput);
    this.refreshPanels();
    return result;
  }

  /** URL loading uses the common parser and returns accepted task identities. */
  async loadUrl(url: string, options?: { name?: string; format?: string }): Promise<CommandOutput> {
    let command = `load ${JSON.stringify(new URL(url, location.href).href)}`;
    if (options?.name) command += `, object=${JSON.stringify(options.name)}`;
    if (options?.format) command += `, format=${JSON.stringify(options.format)}`;
    return this.execute(command);
  }

  // ---------------------------------------------------------------------------
  // Queries
  // ---------------------------------------------------------------------------

  getObjectNames(): string[] {
    return this.core.queryWasm(
      "get_object_names",
      [],
      (wasm) => wasm.get_object_names() as string[],
    );
  }

  getObjectInfo(name: string): ObjectInfo | null {
    return this.core.queryWasm(
      "get_object_info",
      null,
      (wasm) => wasm.get_object_info(name) as ObjectInfo | null,
    );
  }

  getObjectInfos(): ObjectInfo[] {
    return this.core.queryWasm(
      "get_object_infos",
      [],
      (wasm) => wasm.get_object_infos() as ObjectInfo[],
    );
  }

  getSequenceData(): SequenceChain[] {
    return this.core.queryWasm(
      "get_sequence_data",
      [],
      (wasm) => wasm.get_sequence_data() as SequenceChain[],
    );
  }

  getMovieState(): MovieState {
    return this.core.queryWasm(
      "get_movie_state",
      { frame_count: 0, current_frame: 0, is_playing: false, rock_enabled: false },
      (wasm) => wasm.get_movie_state() as MovieState,
    );
  }

  getSelectionList(): SelectionInfo[] {
    return this.core.queryWasm(
      "get_selection_list",
      [],
      (wasm) => wasm.get_selection_list() as SelectionInfo[],
    );
  }

  getPerformanceSnapshot(): ViewerPerformanceSnapshot {
    return this.core.getPerformanceSnapshot();
  }

  resetPerformanceStats(): void {
    this.core.resetPerformanceStats();
  }

  countAtoms(selection = "all"): number {
    return this.core.requireWasm("count_atoms", (wasm) => wasm.count_atoms(selection));
  }

  // ---------------------------------------------------------------------------
  // Events
  // ---------------------------------------------------------------------------

  on<K extends ViewerEventType>(
    event: K,
    callback: (data: ViewerEventMap[K]) => void
  ): void {
    this.events.on(event, callback);
  }

  off<K extends ViewerEventType>(
    event: K,
    callback: (data: ViewerEventMap[K]) => void
  ): void {
    this.events.off(event, callback);
  }

  // ---------------------------------------------------------------------------
  // Panel management
  // ---------------------------------------------------------------------------

  private refreshPanels(): void {
    for (const panel of this.panels.values()) {
      panel.update();
    }
    this.events.emit("objects-changed", { names: this.getObjectNames() });
  }

  private emitCommandMessages(messages: OutputMessage[]): void {
    for (const message of messages) {
      this.events.emit("command-output", message);
    }
  }

  private emitRendererOutput(message: OutputMessage): void {
    this.events.emit("command-output", message);
    const panel = this.panels.get("repl");
    if (panel instanceof ReplPanel) {
      panel.appendOutputMessage(message);
    }
  }

  destroy(): void {
    for (const panel of this.panels.values()) {
      panel.destroy();
    }
    this.panels.clear();
    this.core.destroy();
  }
}

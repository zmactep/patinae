/** Types matching the WASM bridge serialization format. */

export interface CommandOutput {
  messages: OutputMessage[];
}

export interface OutputMessage {
  level: "info" | "warning" | "error" | "clear";
  text: string;
}

export interface ObjectInfo {
  name: string;
  object_type: "molecule" | "map" | "measurement" | "label";
  atom_count: number;
  enabled: boolean;
  measurement_kind: "distance" | "angle" | "dihedral" | null;
  entity_count: number;
  has_unresolved_entities: boolean;
  focus_disabled_reason: string | null;
  color: [number, number, number];
  multicolor: boolean;
  parent_group: string | null;
  has_representations: boolean;
  can_focus: boolean;
  can_color: boolean;
  can_rename: boolean;
  can_delete: boolean;
  can_group: boolean;
}

export interface SelectionInfo {
  name: string;
  expression: string;
  visible: boolean;
}

export interface SequenceChain {
  object_name: string;
  chain_id: string;
  residues: SequenceResidue[];
}

export interface SequenceResidue {
  resn: string;
  resv: number;
  one_letter: string;
}

export interface MovieState {
  frame_count: number;
  current_frame: number;
  is_playing: boolean;
  rock_enabled: boolean;
}

export interface LabelInfo {
  x: number;
  y: number;
  text: string;
  color: [number, number, number, number];
  size: number;
  alignment:
    | "bottom-left"
    | "bottom-center"
    | "bottom-right"
    | "center-left"
    | "center"
    | "center-right"
    | "top-left"
    | "top-center"
    | "top-right";
  anchor_x: number;
  anchor_y: number;
  owner_id: number;
  insertion_ordinal: number;
  display_order: number;
}

export type PanelSlot = "top" | "right" | "bottom";

export interface PanelPlacement {
  name: PanelName;
  slot: PanelSlot;
  collapsed?: boolean;
}

/** Information about a picked atom returned by `pick_at_screen`. */
export interface PickHitInfo {
  /** Name of the picked object. */
  object_name: string;
  /** Zero-based atom index within the object, or `null` for non-atom hits. */
  atom_index: number | null;
  /** Chain identifier of the hit atom, or `null`. */
  chain: string | null;
  /** Residue sequence number of the hit atom, or `null`. */
  residue: number | null;
  /** Selection expression (depends on `mouse_selection_mode`), or `null`. */
  expression: string | null;
}

export interface ViewerWasmPerformanceSnapshot {
  render_count: number;
  avg_render_ms: number;
  median_render_ms: number;
  p95_render_ms: number;
  last_render_ms: number;
  last_poll_picks_ms: number;
  last_uniforms_ms: number;
  last_prepare_ms: number;
  last_sync_ms: number;
  last_sync_scene_store_object_ms: number;
  last_sync_scene_store_flush_ms: number;
  last_sync_marking_resources_ms: number;
  last_sync_rep_ms: number;
  last_sync_map_ms: number;
  last_sync_order_bounds_ms: number;
  last_sync_compute_dispatch_ms: number;
  last_sync_marker_lut_upload_bytes: number;
  last_sync_marker_lut_upload_ranges: number;
  last_sync_marker_lut_reallocated: boolean;
  last_sync_scene_store_live_atoms: number;
  last_sync_scene_store_allocated_atoms: number;
  last_sync_scene_store_orphaned_atoms: number;
  last_sync_scene_store_live_bonds: number;
  last_sync_scene_store_allocated_bonds: number;
  last_sync_scene_store_orphaned_bonds: number;
  last_sync_scene_store_live_table_slots: number;
  last_sync_scene_store_allocated_table_slots: number;
  last_sync_scene_store_orphaned_table_slots: number;
  last_sync_scene_store_live_bytes: number;
  last_sync_scene_store_allocated_bytes: number;
  last_sync_scene_store_orphaned_bytes: number;
  last_sync_scene_store_capacity_bytes: number;
  last_sync_scene_store_capacity_slack_bytes: number;
  last_sync_scene_store_compacted: boolean;
  last_sync_scene_store_compaction_reclaimed_bytes: number;
  last_sync_scene_store_compaction_capacity_before_bytes: number;
  last_sync_scene_store_compaction_capacity_after_bytes: number;
  last_sync_scene_store_compaction_orphaned_before_bytes: number;
  last_sync_scene_store_compaction_moved_objects: number;
  sphere_lod_active: boolean;
  sphere_lod_sample_shift: number;
  sphere_lod_sample_stride: number;
  sphere_lod_base_sample_shift: number;
  sphere_lod_source_atom_count: number;
  sphere_lod_instance_upper_bound: number;
  sphere_lod_cull_upper_bound: number;
  sphere_lod_viewport_visible_count: number;
  sphere_lod_viewport_full_detail: boolean;
  stick_lod_active: boolean;
  stick_lod_sample_shift: number;
  stick_lod_sample_stride: number;
  stick_lod_base_sample_shift: number;
  stick_lod_source_bond_count: number;
  stick_lod_sampled_bond_upper_bound: number;
  stick_lod_cull_upper_bound: number;
  stick_lod_viewport_visible_count: number;
  stick_lod_viewport_full_detail: boolean;
  overlay_id_marked_only: boolean;
  last_settings_ms: number;
  last_acquire_ms: number;
  last_encode_ms: number;
  last_submit_present_ms: number;
  hover_submitted: number;
  hover_completed: number;
  hover_stale: number;
  hover_queued: number;
  hover_deferred: number;
  hover_throttle_active: boolean;
  hover_cancelled: number;
  click_submitted: number;
  click_completed: number;
  hover_pending: boolean;
  click_pending: boolean;
}

export interface ViewerPerformanceSnapshot {
  frame_count: number;
  avg_frame_ms: number;
  median_frame_ms: number;
  p95_frame_ms: number;
  last_frame_ms: number;
  wasm: ViewerWasmPerformanceSnapshot | null;
}

export type RenderMemoryProfileOption =
  | "auto"
  | "performance"
  | "balanced"
  | "lite"
  | `manual:${number}`;

export interface ViewerOptions {
  /** Simple list — all panels go into a "sidebar" element (legacy). */
  panels?: PanelName[];
  /** Desktop-style layout — panels placed in named slots around the viewport. */
  layout?: PanelPlacement[];
  /** Container elements for each slot (required when using `layout`). */
  slots?: Partial<Record<PanelSlot, HTMLElement>>;
  /** When true, the viewer starts hidden and is revealed only when `show()` is called. */
  defer?: boolean;
  /** Duration of the fade-in transition in milliseconds (default: 150). */
  revealDuration?: number;
  /**
   * When true, left-click picks atoms and updates the `sele` named selection.
   * Fires `atom-picked` events with hit details. Default: false.
   */
  picking?: boolean;
  /**
   * Draw the visible selection / hover overlay. Defaults to `picking`, so
   * read-only embedded viewers stay visually inert unless explicitly enabled.
   */
  selectionOverlay?: boolean;
  /**
   * Force the renderer memory profile at WebGPU initialization.
   * Defaults to adapter-based selection.
   */
  memoryProfile?: RenderMemoryProfileOption;
}

export type PanelName = "repl" | "objects" | "sequence" | "movie";

/** Modifier bitmask constants (must match event.rs). */
export const MOD_SHIFT = 1;
export const MOD_CTRL = 2;
export const MOD_ALT = 4;
export const MOD_META = 8;

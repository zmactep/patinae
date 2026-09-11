import type { TaskChanged } from './types.js';

/** Typed event system for viewer state changes. */

export type ViewerEventType =
  | "objects-changed"
  | "selection-changed"
  | "command-output"
  | "movie-state-changed"
  | "atom-picked"
  | "ready"
  | "tasks.changed";

export interface ViewerEventMap {
  "objects-changed": { names: string[] };
  "selection-changed": { selection: string };
  "command-output": { level: string; text: string };
  "movie-state-changed": { frame: number; total: number; playing: boolean };
  "atom-picked": {
    object_name: string | null;
    atom_index: number | null;
    chain: string | null;
    residue: number | null;
    expression: string | null;
  };
  "ready": {};
  "tasks.changed": TaskChanged;
}

type EventCallback<T> = (data: T) => void;

export class ViewerEvents {
  private listeners = new Map<string, Set<EventCallback<unknown>>>();

  on<K extends ViewerEventType>(
    event: K,
    callback: EventCallback<ViewerEventMap[K]>
  ): void {
    if (!this.listeners.has(event)) {
      this.listeners.set(event, new Set());
    }
    this.listeners.get(event)!.add(callback as EventCallback<unknown>);
  }

  off<K extends ViewerEventType>(
    event: K,
    callback: EventCallback<ViewerEventMap[K]>
  ): void {
    this.listeners.get(event)?.delete(callback as EventCallback<unknown>);
  }

  emit<K extends ViewerEventType>(event: K, data: ViewerEventMap[K]): void {
    for (const cb of this.listeners.get(event) ?? []) {
      try {
        cb(data);
      } catch (e) {
        console.error(`Event handler error (${event}):`, e);
      }
    }
  }
}

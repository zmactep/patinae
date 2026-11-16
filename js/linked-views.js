import { executeCommand, waitForTasks } from './viewer-commands.js';

async function readView(viewer) {
  const reply = await viewer.execute('get_view');
  await waitForTasks(viewer, reply);
  // get_view emits a portable set_view command, including line continuations.
  const text = reply.messages.map(message => message.text).join('\n');
  const tuple = text.match(/set_view\s*\(([^)]+)\)/)?.[1];
  const values = tuple?.replace(/\\/g, '').split(',').map(value => Number(value.trim()));
  if (!values || values.length !== 18 || values.some(value => !Number.isFinite(value))) {
    throw new Error('The viewer did not return a complete camera view.');
  }
  return `set_view (${values.join(', ')})`;
}

// Share camera state, not synthetic mouse events: this also handles different
// viewport sizes, zoom, pan and views that finish loading after an interaction.
export class LinkedViews {
  constructor({
    schedule = callback => requestAnimationFrame(callback),
    cancel = frame => cancelAnimationFrame(frame),
    onError,
  } = {}) {
    this.schedule = schedule;
    this.cancel = cancel;
    this.onError = onError;
    this.viewers = new Map();
    this.view = null;
    this.source = null;
    this.frame = null;
    this.revision = 0;
    this.running = false;
    this.disposed = false;
  }

  async add(name, viewer) {
    if (this.disposed) return;
    this.viewers.set(name, viewer);
    try {
      if (this.view) await executeCommand(viewer, this.view);
      else this.view = await readView(viewer);
    } catch (error) {
      this.remove(name);
      throw error;
    }
  }

  remove(name) {
    this.viewers.delete(name);
    if (this.source === name) this.source = null;
  }

  request(name) {
    if (this.disposed || !this.viewers.has(name)) return;
    this.source = name;
    this.revision++;
    this.queue();
  }

  queue() {
    if (this.disposed || this.running || this.frame !== null || this.source === null) return;
    this.frame = this.schedule(() => {
      this.frame = null;
      return this.flush();
    });
  }

  async flush() {
    const name = this.source;
    const viewer = this.viewers.get(name);
    const revision = this.revision;
    this.source = null;
    if (!viewer || this.disposed) return;
    this.running = true;
    try {
      const view = await readView(viewer);
      // An input in a different panel may have arrived while reading the view.
      if (this.disposed || revision !== this.revision || this.viewers.get(name) !== viewer) return;
      this.view = view;
      for (const [otherName, otherViewer] of this.viewers) {
        if (this.disposed || revision !== this.revision) break;
        if (otherName === name) continue;
        try { await executeCommand(otherViewer, view); }
        catch (error) {
          this.remove(otherName);
          this.onError?.(otherName, error);
        }
      }
    } catch (error) {
      if (!this.disposed && this.viewers.get(name) === viewer) {
        this.remove(name);
        this.onError?.(name, error);
      }
    } finally {
      this.running = false;
      this.queue();
    }
  }

  dispose() {
    this.disposed = true;
    this.revision++;
    if (this.frame !== null) this.cancel(this.frame);
    this.frame = null;
    this.source = null;
    this.viewers.clear();
  }
}

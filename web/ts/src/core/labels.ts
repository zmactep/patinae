/**
 * LabelOverlay — renders projected labels as DOM elements over the canvas.
 *
 * Uses element pooling to avoid DOM allocation churn during camera rotation.
 */

import type { LabelInfo } from "./types.js";

export class LabelOverlay {
  private container: HTMLDivElement;
  private pool: HTMLSpanElement[] = [];
  private activeCount = 0;

  constructor(parent: HTMLElement) {
    this.container = document.createElement("div");
    this.container.style.position = "absolute";
    this.container.style.top = "0";
    this.container.style.left = "0";
    this.container.style.width = "100%";
    this.container.style.height = "100%";
    this.container.style.pointerEvents = "none";
    this.container.style.overflow = "hidden";
    parent.appendChild(this.container);
  }

  update(labels: LabelInfo[], dpr: number): void {
    // Grow pool if needed
    while (this.pool.length < labels.length) {
      const el = document.createElement("span");
      el.style.position = "absolute";
      el.style.whiteSpace = "nowrap";
      el.style.fontFamily =
        "-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif";
      el.style.textShadow =
        "0 0 3px rgba(0,0,0,0.8), 0 0 6px rgba(0,0,0,0.5)";
      el.style.display = "none";
      this.container.appendChild(el);
      this.pool.push(el);
    }

    for (let i = 0; i < labels.length; i++) {
      const label = labels[i];
      const el = this.pool[i];

      const cssX = label.x / dpr;
      const cssY = label.y / dpr;

      el.style.left = `${cssX}px`;
      el.style.top = `${cssY}px`;
      el.textContent = label.text;
      el.style.fontSize = `${label.size}px`;
      el.style.transform = `translate(${-label.anchor_x * 100}%, ${-label.anchor_y * 100}%)`;
      el.style.color = rgbaToCss(label.color);
      el.dataset.alignment = label.alignment;
      el.dataset.ownerId = String(label.owner_id);
      el.dataset.insertionOrdinal = String(label.insertion_ordinal);
      el.dataset.displayOrder = String(label.display_order);

      el.style.display = "";
    }

    // Hide unused elements
    for (let i = labels.length; i < this.activeCount; i++) {
      this.pool[i].style.display = "none";
    }

    this.activeCount = labels.length;
  }

  destroy(): void {
    this.container.remove();
  }
}

function rgbaToCss(color: LabelInfo["color"]): string {
  const red = Math.round(clampUnit(color[0]) * 255);
  const green = Math.round(clampUnit(color[1]) * 255);
  const blue = Math.round(clampUnit(color[2]) * 255);
  const alpha = clampUnit(color[3]);
  return `rgba(${red}, ${green}, ${blue}, ${alpha})`;
}

function clampUnit(value: number): number {
  return Math.min(1, Math.max(0, value));
}

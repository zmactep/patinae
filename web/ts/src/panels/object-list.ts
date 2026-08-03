/** Object List panel for molecules, maps, and annotation collections. */

import type { PatinaeViewer } from "../core/api.js";
import type { ObjectInfo } from "../core/types.js";

const REPS = ["lines", "sticks", "cartoon", "spheres", "surface"] as const;

export class ObjectListPanel {
  private container: HTMLElement;
  private viewer: PatinaeViewer;
  private list: HTMLElement;
  private selectedObject: string | null = null;

  constructor(container: HTMLElement, viewer: PatinaeViewer) {
    this.container = container;
    this.viewer = viewer;

    container.innerHTML = `
      <div class="panel-header">Objects</div>
      <div class="object-list"></div>
    `;

    this.list = container.querySelector(".object-list")!;
  }

  update(): void {
    const objects = this.viewer.getObjectInfos();
    this.list.innerHTML = "";

    for (const info of objects) {
      const name = info.name;

      const row = document.createElement("div");
      row.className = `object-row${info.parent_group ? " grouped" : ""}${this.selectedObject === name ? " selected" : ""}`;
      row.dataset.objectName = name;
      row.setAttribute("role", "button");
      row.setAttribute("aria-selected", String(this.selectedObject === name));
      if (info.focus_disabled_reason) {
        row.setAttribute("aria-description", info.focus_disabled_reason);
      }
      row.tabIndex = 0;
      row.addEventListener("click", (event) => {
        if ((event.target as HTMLElement).closest("button, select")) return;
        this.selectObject(name);
      });
      row.addEventListener("keydown", (event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          this.selectObject(name);
        }
      });
      row.addEventListener("dblclick", () => {
        if (info.can_focus) this.viewer.execute(`zoom ${name}`);
      });

      // Visibility toggle
      const vis = document.createElement("button");
      vis.className = `obj-vis ${info.enabled ? "enabled" : "disabled"}`;
      vis.textContent = info.enabled ? "V" : "-";
      vis.title = info.enabled ? "Hide" : "Show";
      vis.addEventListener("click", () => {
        this.viewer.execute(info.enabled ? `disable ${name}` : `enable ${name}`);
      });

      const icon = document.createElement("span");
      icon.className = `obj-kind obj-kind-${objectKindClass(info)}`;
      if (info.multicolor) icon.classList.add("multicolor");
      icon.textContent = objectIcon(info);
      icon.title = objectKindLabel(info);
      icon.setAttribute("aria-label", objectKindLabel(info));
      icon.style.color = rgbToCss(info.color);

      // Name
      const label = document.createElement("span");
      label.className = "obj-name";
      label.textContent = name;
      label.title = [
        `${name} — ${objectKindLabel(info)}`,
        info.parent_group ? `Group: ${info.parent_group}` : null,
        info.focus_disabled_reason,
      ]
        .filter(Boolean)
        .join(" · ");

      const metadata = document.createElement("span");
      metadata.className = "obj-metadata";
      if (info.object_type === "molecule") {
        metadata.textContent = `${info.atom_count}`;
        metadata.title = `${info.atom_count} atoms`;
      } else if (info.object_type === "measurement") {
        metadata.textContent = `${info.entity_count}`;
        metadata.title = `${info.entity_count} measurement ${plural(info.entity_count, "entity", "entities")}`;
      } else if (info.object_type === "label") {
        metadata.textContent = `${info.entity_count}`;
        metadata.title = `${info.entity_count} ${plural(info.entity_count, "label", "labels")}`;
      } else {
        metadata.textContent = "map";
      }

      row.appendChild(vis);
      row.appendChild(icon);
      row.appendChild(label);
      row.appendChild(metadata);

      if (info.has_unresolved_entities) {
        const warning = document.createElement("span");
        warning.className = "obj-warning";
        warning.textContent = "!";
        warning.title = "One or more annotation entities are unresolved";
        warning.setAttribute("aria-label", warning.title);
        row.appendChild(warning);
      }

      // Representation buttons (molecule-only)
      if (info.has_representations) {
        const reps = document.createElement("span");
        reps.className = "obj-reps";
        for (const rep of REPS) {
          const btn = document.createElement("button");
          btn.className = "rep-btn";
          btn.textContent = rep.charAt(0).toUpperCase();
          btn.title = rep;
          btn.addEventListener("click", () => {
            this.viewer.execute(`show ${rep}, ${name}`);
          });
          reps.appendChild(btn);
        }
        row.appendChild(reps);
      }

      const actions = buildObjectActions(info, (action) => {
        this.runObjectAction(info, action);
      });
      if (actions) row.appendChild(actions);

      this.list.appendChild(row);
    }

    // Named selections
    const selections = this.viewer.getSelectionList();
    if (selections.length > 0) {
      const sep = document.createElement("hr");
      sep.className = "object-list-separator";
      this.list.appendChild(sep);

      for (const sel of selections) {
        const row = document.createElement("div");
        row.className = "object-row selection-row";

        // Visibility toggle
        const vis = document.createElement("button");
        vis.className = `obj-vis selection-vis ${sel.visible ? "enabled" : "disabled"}`;
        vis.textContent = sel.visible ? "V" : "-";
        vis.title = sel.visible ? "Hide indicators" : "Show indicators";
        vis.addEventListener("click", () => {
          this.viewer.execute(`toggle ${sel.name}`);
        });

        // Name in parentheses (PyMOL convention)
        const label = document.createElement("span");
        label.className = "obj-name selection-name";
        label.textContent = `(${sel.name})`;
        label.title = sel.expression;

        // Delete button
        const del = document.createElement("button");
        del.className = "rep-btn selection-delete";
        del.textContent = "X";
        del.title = "Delete selection";
        del.addEventListener("click", () => {
          this.viewer.execute(`deselect ${sel.name}`);
        });

        row.appendChild(vis);
        row.appendChild(label);
        row.appendChild(del);
        this.list.appendChild(row);
      }
    }
  }

  destroy(): void {
    this.container.innerHTML = "";
  }

  private selectObject(name: string): void {
    this.selectedObject = name;
    for (const row of this.list.querySelectorAll<HTMLElement>(".object-row[data-object-name]")) {
      const selected = row.dataset.objectName === name;
      row.classList.toggle("selected", selected);
      row.setAttribute("aria-selected", String(selected));
    }
  }

  private runObjectAction(info: ObjectInfo, action: ObjectAction): void {
    switch (action) {
      case "focus":
        this.viewer.execute(`zoom ${info.name}`);
        break;
      case "color": {
        const color = window.prompt(`Color for ${info.name}:`, "cyan")?.trim();
        if (color) this.viewer.execute(`color ${color}, ${info.name}`);
        break;
      }
      case "rename": {
        const nextName = window.prompt(`Rename ${info.name}:`, info.name)?.trim();
        if (nextName && nextName !== info.name) {
          this.viewer.execute(`set_name ${info.name}, ${nextName}`);
          if (this.selectedObject === info.name) this.selectedObject = nextName;
        }
        break;
      }
      case "group": {
        const groupName = window.prompt(`Add ${info.name} to group:`)?.trim();
        if (groupName) this.viewer.execute(`group ${groupName}, ${info.name}, action=add`);
        break;
      }
      case "delete":
        if (window.confirm(`Delete ${info.name}?`)) {
          this.viewer.execute(`delete ${info.name}`);
          if (this.selectedObject === info.name) this.selectedObject = null;
        }
        break;
    }
  }
}

type ObjectAction = "focus" | "color" | "rename" | "group" | "delete";

function buildObjectActions(
  info: ObjectInfo,
  run: (action: ObjectAction) => void,
): HTMLSelectElement | null {
  const actions: Array<[ObjectAction, string, boolean]> = [];
  if (info.can_focus) {
    actions.push(["focus", "Focus", false]);
  } else if (info.focus_disabled_reason) {
    actions.push(["focus", `Focus — ${info.focus_disabled_reason}`, true]);
  }
  if (info.can_color) actions.push(["color", "Color…", false]);
  if (info.can_rename) actions.push(["rename", "Rename…", false]);
  if (info.can_group) actions.push(["group", "Add to group…", false]);
  if (info.can_delete) actions.push(["delete", "Delete…", false]);
  if (actions.length === 0) return null;

  const select = document.createElement("select");
  select.className = "obj-actions";
  select.title = `Actions for ${info.name}`;
  if (info.focus_disabled_reason) {
    select.title += ` · ${info.focus_disabled_reason}`;
  }
  select.setAttribute("aria-label", select.title);

  const placeholder = document.createElement("option");
  placeholder.value = "";
  placeholder.textContent = "•••";
  select.appendChild(placeholder);

  for (const [value, label, disabled] of actions) {
    const option = document.createElement("option");
    option.value = value;
    option.textContent = label;
    option.disabled = disabled;
    select.appendChild(option);
  }

  select.addEventListener("change", () => {
    const action = select.value as ObjectAction | "";
    select.value = "";
    if (action) run(action);
  });
  return select;
}

function objectKindClass(info: ObjectInfo): string {
  return info.object_type === "measurement"
    ? (info.measurement_kind ?? "measurement")
    : info.object_type;
}

function objectKindLabel(info: ObjectInfo): string {
  switch (info.object_type) {
    case "molecule":
      return "Molecule";
    case "map":
      return "Map";
    case "measurement":
      switch (info.measurement_kind) {
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

function objectIcon(info: ObjectInfo): string {
  switch (info.object_type) {
    case "molecule":
      return "M";
    case "map":
      return "▦";
    case "measurement":
      switch (info.measurement_kind) {
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

function rgbToCss(color: ObjectInfo["color"]): string {
  const channels = color.map((value) => Math.round(Math.min(1, Math.max(0, value)) * 255));
  return `rgb(${channels[0]}, ${channels[1]}, ${channels[2]})`;
}

function plural(value: number, singular: string, pluralForm: string): string {
  return value === 1 ? singular : pluralForm;
}

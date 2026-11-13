import { executeCommand, loadStructure } from "./viewer-commands.js";

// ── Utilities ───────────────────────────────────────────────────────────────

function flashElement(el, className = "flash", ms = 400) {
  el.classList.add(className);
  setTimeout(() => el.classList.remove(className), ms);
}

async function execCmdString(viewer, cmdStr) {
  for (const cmd of cmdStr.split(";").map(s => s.trim()).filter(Boolean))
    await executeCommand(viewer, cmd);
}

async function loadSources(viewer, src) {
  for (const url of src.split(/\s+/).filter(Boolean))
    await loadStructure(viewer, url);
}

// ── SPA page navigation ─────────────────────────────────────────────────────

/**
 * Initializes SPA-style documentation with hash-routed sections.
 *
 * Each section is a `.doc-page` div with `id` and optional data attributes:
 *   data-src="url"              — structure file(s) to load (whitespace-separated)
 *   data-command="cmd1; cmd2"   — semicolon-separated PyMOL commands
 *
 * Inline commands use `data-cmd` (semicolon-separated) with optional
 * `data-src` for structure file(s) and `data-cmd-alt` for toggle behaviour.
 */
export function initDocPages(viewer) {
  let setupSeq = 0;
  let viewerQueue = Promise.resolve();

  function enqueueViewerOperation(operation) {
    const queued = viewerQueue.then(operation);
    viewerQueue = queued.catch(() => {});
    return queued;
  }

  // ── Inline command clicks ─────────────────────────────────────────────
  document.addEventListener("click", async (e) => {
    const el = e.target.closest("[data-cmd]");
    if (!el) return;
    e.preventDefault();
    const src = el.dataset.src;
    const alt = el.dataset.cmdAlt;
    const toggled = el.classList.contains("toggled");
    const cmdStr = (alt && toggled) ? alt : el.dataset.cmd;
    const seq = setupSeq;
    if (alt) el.classList.toggle("toggled");

    // Radio-group: un-toggle siblings in the same group
    const group = el.dataset.cmdGroup;
    if (group && !toggled) {
      document.querySelectorAll(`[data-cmd-group="${group}"]`).forEach(sib => {
        if (sib !== el) sib.classList.remove("toggled");
      });
    }

    try {
      await enqueueViewerOperation(async () => {
        if (seq !== setupSeq) return;
        if (src) {
          await executeCommand(viewer, "delete all");
          await loadSources(viewer, src);
          if (seq !== setupSeq) return;
        }
        await execCmdString(viewer, cmdStr);
        if (seq === setupSeq) flashElement(el);
      });
    } catch (error) {
      console.error("Interactive viewer command failed:", error);
    }
  });

  // ── Page management ───────────────────────────────────────────────────
  const pages = Array.from(document.querySelectorAll(".doc-page"));
  if (pages.length === 0) return;

  const sidebar = document.querySelector(".docs-sidebar");
  let currentIdx = 0;

  function showPage(idx, pushState = true) {
    if (idx < 0 || idx >= pages.length) return;
    currentIdx = idx;
    const page = pages[idx];

    // Show only the active page
    pages.forEach(p => p.style.display = "none");
    page.style.display = "block";

    // Sidebar highlight
    if (sidebar) {
      sidebar.querySelectorAll("a").forEach((a, i) => {
        a.classList.toggle("active", i === idx);
      });
    }

    // URL hash
    if (pushState) history.pushState(null, "", "#" + page.id);

    // Viewer setup for this section
    setupViewer(page).catch(error => {
      console.error("Viewer setup failed:", error);
    });

    // Prev / Next buttons
    page.querySelectorAll("[data-nav]").forEach(btn => {
      const dir = btn.dataset.nav;
      if (dir === "prev") {
        btn.style.visibility = idx > 0 ? "visible" : "hidden";
        if (idx > 0) btn.textContent = "\u2190 " + (pages[idx - 1].dataset.title || "Previous");
      } else if (dir === "next") {
        btn.style.visibility = idx < pages.length - 1 ? "visible" : "hidden";
        if (idx < pages.length - 1) btn.textContent = (pages[idx + 1].dataset.title || "Next") + " \u2192";
      }
    });

    // Page counter
    const counter = page.querySelector(".page-counter");
    if (counter) counter.textContent = `${idx + 1} of ${pages.length}`;

    // Scroll to top
    window.scrollTo({ top: 0 });
  }

  function setupViewer(page) {
    const src = page.dataset.src;
    const command = page.dataset.command;
    const seq = ++setupSeq;
    if (!src && !command) return Promise.resolve();

    return enqueueViewerOperation(async () => {
      if (seq !== setupSeq) return;

      viewer.core.setDeferred(true);
      // PRS files can carry global settings such as cartoon_color. Start each
      // documentation page with a fresh session so those settings cannot leak
      // into structures loaded by the following page.
      await executeCommand(viewer, "reinitialize");

      if (src) await loadSources(viewer, src);

      if (seq !== setupSeq) return;
      if (command) await execCmdString(viewer, command);

      await viewer.show();
    });
  }

  // ── Navigation wiring ─────────────────────────────────────────────────
  document.addEventListener("click", (e) => {
    const btn = e.target.closest("[data-nav]");
    if (!btn) return;
    e.preventDefault();
    if (btn.dataset.nav === "prev") showPage(currentIdx - 1);
    else if (btn.dataset.nav === "next") showPage(currentIdx + 1);
  });

  if (sidebar) {
    sidebar.querySelectorAll("a").forEach((a, i) => {
      a.addEventListener("click", (e) => {
        e.preventDefault();
        showPage(i);
      });
    });
  }

  // ── Initial navigation ────────────────────────────────────────────────
  function navigateToHash() {
    const hash = location.hash.slice(1);
    const idx = pages.findIndex(p => p.id === hash);
    showPage(idx >= 0 ? idx : 0, false);
  }

  window.addEventListener("popstate", navigateToHash);
  navigateToHash();
}

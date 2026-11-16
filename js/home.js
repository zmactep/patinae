import { PatinaeViewer } from './patinae-viewer.js';
import { executeCommand, loadStructure, waitForTasks } from './viewer-commands.js';
import { LinkedViews } from './linked-views.js';

const scenes = new Map();
const linkedViews = new LinkedViews({ onError: (name, error) => failScene(scenes.get(name), error) });
const linkedInput = new AbortController();
let startupQueue = Promise.resolve();
let heroView = 'fold';
let heroShading = 'classic';
const commandForm = document.getElementById('hero-console');
const commandInput = document.getElementById('hero-command');
const commandRun = document.getElementById('hero-command-run');
const commandOutput = document.getElementById('hero-command-output');
const commandHistory = [];
let historyIndex = 0;
let commandDraft = '';

const ribbonLighting = [
  'set ambient, 0.4', 'set direct, 0.6',
  'set reflect, 0.25', 'set specular, 0.15', 'set depth_cue, off',
  'set fog, 0', 'set silhouettes, off',
];

const heroPresets = {
  fold: {
    commands: ['enable all', 'as cartoon', 'orient', 'zoom name CA, buffer=0'],
    note: '',
  },
  surface: {
    commands: ['enable all', 'as surface', 'orient', 'zoom name CA, buffer=0'],
    note: 'Molecular surface',
  },
  site: {
    commands: ['disable sym*', 'select metal, first (1gvf and elem Zn)', 'select site, byres near_to 5 of metal', 'select site, site and 1gvf', 'hide everything', 'show sticks, site', 'show spheres, metal', 'set stick_color, atomic, site', 'orient site', 'zoom site, buffer=3'],
    note: 'Zinc · neighbourhood within 5 Å',
  },
};

function heroCommands(view) {
  const width = scenes.get('hero').mount.clientWidth;
  return [`set shading_mode, ${heroShading}`, ...heroPresets[view].commands.map(command => command === 'zoom name CA, buffer=0'
    ? `zoom name CA, buffer=${width > 600 ? -30 : -25}` : command)];
}

async function commands(viewer, list) {
  for (const command of list) {
    try { await executeCommand(viewer, command); }
    catch (error) { throw new Error(`${command}: ${error.message}`, { cause: error }); }
  }
}

function setControls(name, disabled) {
  document.querySelectorAll(`[data-reset="${name}"]`).forEach(button => { button.disabled = disabled; });
  if (name === 'hero') {
    document.querySelectorAll('[data-hero-view], [data-hero-shading], #try-site').forEach(button => { button.disabled = disabled; });
    commandRun.disabled = disabled;
    commandInput.disabled = scenes.get('hero')?.element.dataset.state !== 'ready';
    commandInput.readOnly = disabled;
  }
}

function printCommandOutput(text, kind = 'info') {
  const line = document.createElement('div');
  line.className = kind;
  line.textContent = text;
  commandOutput.appendChild(line);
  while (commandOutput.childElementCount > 200) commandOutput.firstElementChild.remove();
  commandOutput.scrollTop = commandOutput.scrollHeight;
}

commandForm.addEventListener('submit', async event => {
  event.preventDefault();
  const scene = scenes.get('hero');
  const command = commandInput.value.trim();
  if (!command || !scene?.viewer || scene.busy || scene.element.dataset.state !== 'ready') return;
  scene.busy = true;
  setControls('hero', true);
  commandForm.setAttribute('aria-busy', 'true');
  commandRun.textContent = 'Running…';
  commandHistory.push(command);
  if (commandHistory.length > 100) commandHistory.shift();
  historyIndex = commandHistory.length;
  commandDraft = '';
  commandInput.value = '';
  printCommandOutput(`Patinae> ${command}`, 'command');
  const viewer = scene.viewer;
  let outputCount = 0;
  let commandSucceeded = false;
  const onOutput = message => {
    if (message.level === 'clear') commandOutput.replaceChildren();
    else {
      printCommandOutput(message.text, message.level);
      outputCount++;
    }
  };
  viewer.on('command-output', onOutput);
  try {
    const reply = await viewer.execute(command);
    await waitForTasks(viewer, reply);
    commandSucceeded = true;
    if (!outputCount) printCommandOutput('Done.');
  } catch (error) {
    // An invalid user command must not destroy a working molecular viewer.
    printCommandOutput(error.message, 'error');
  } finally {
    viewer.off('command-output', onOutput);
    // Commands may change shading too; keep the toolbar and later presets in sync.
    try {
      const reply = await viewer.execute('get shading_mode');
      const mode = reply.messages.map(message => message.text).join('\n').match(/shading_mode\s*=\s*([012])\b/);
      if (mode) {
        heroShading = ['classic', 'skripkin', 'full'][Number(mode[1])];
        document.querySelectorAll('[data-hero-shading]').forEach(button => {
          button.setAttribute('aria-pressed', String(button.dataset.heroShading === heroShading));
        });
        const ambient = command.match(/^set\s+ambient\s*,\s*([^,]+)(?:,|$)/i);
        if (commandSucceeded && heroShading === 'skripkin' && ambient) {
          printCommandOutput(`ambient affects Classic and Full shading. For Skripkin, use: set skripkin_ambient, ${ambient[1].trim()}`, 'warning');
        }
      }
    } catch { /* A stopped renderer cannot report its settings. */ }
    scene.busy = false;
    commandForm.setAttribute('aria-busy', 'false');
    commandRun.textContent = 'Run ↵';
    setControls('hero', scene.element.dataset.state !== 'ready');
    if (document.activeElement === commandRun) commandInput.focus({ preventScroll: true });
  }
});

commandInput.addEventListener('keydown', event => {
  if (commandInput.readOnly || !['ArrowUp', 'ArrowDown'].includes(event.key)) return;
  event.preventDefault();
  if (event.key === 'ArrowUp') {
    if (historyIndex === commandHistory.length) commandDraft = commandInput.value;
    historyIndex = Math.max(0, historyIndex - 1);
  } else {
    historyIndex = Math.min(commandHistory.length, historyIndex + 1);
  }
  commandInput.value = commandHistory[historyIndex] ?? commandDraft;
  commandInput.setSelectionRange(commandInput.value.length, commandInput.value.length);
});

document.getElementById('hero-command-clear').addEventListener('click', () => commandOutput.replaceChildren());

function failScene(scene, error) {
  linkedViews.remove(scene.name);
  scene.element.dataset.state = 'error';
  scene.status.textContent = navigator.gpu
    ? 'This 3D view could not start. You can still explore the desktop app.'
    : 'Live 3D needs a browser with WebGPU. Try the desktop app below.';
  if (scene.name === 'hero') document.getElementById('hero-help').textContent = 'Interactive view unavailable';
  setControls(scene.name, true);
  if (scene.viewer) {
    try { scene.viewer.destroy(); } catch { /* The renderer may already have stopped. */ }
    scene.viewer = null;
  }
  console.warn(`Patinae ${scene.name} view:`, error);
}

async function startScene(scene) {
  if (scene.element.dataset.state !== 'idle') return;
  scene.element.dataset.state = 'loading';
  scene.element.setAttribute('aria-busy', 'true');
  scene.status.textContent = scene.name === 'hero' ? 'Loading 1GVF…' : 'Loading interactive structure…';
  try {
    if (!navigator.gpu) throw new Error('WebGPU is unavailable');
    const viewer = new PatinaeViewer(scene.mount, { defer: true, memoryProfile: 'balanced', revealDuration: matchMedia('(prefers-reduced-motion: reduce)').matches ? 0 : 150 });
    scene.viewer = viewer;
    await viewer.init();
    await commands(viewer, ['bg_color 0b0e10', 'set depth_cue, off', 'set fog, 0', 'set specular, 0.15']);
    if (scene.name === 'hero') {
      await loadStructure(viewer, './assets/pdb/1GVF.pdb', { name: '1gvf', format: 'pdb' });
      // Use base atom colours so console commands can override the demo palette.
      // Lighting is initial setup, not part of representation changes.
      await commands(viewer, ['as cartoon', 'color chain', 'symexp sym, 1gvf, 1gvf, 1, 1', 'color 0xC88F6B, chain A', 'color 0xA4C9B7, chain B', ...ribbonLighting, ...heroCommands('fold')]);
    } else {
      await loadStructure(viewer, './assets/pdb/1fsd.cif', { name: '1fsd', format: 'cif' });
      await commands(viewer, [`as ${scene.name}`, 'color chain', 'orient', 'zoom all, buffer=-3']);
      if (scene.name === 'cartoon') await commands(viewer, ['set cartoon_color, 0xA4C9B7', 'set cartoon_color, 0xC88F6B, ss H', ...ribbonLighting]);
      if (scene.name === 'surface') await commands(viewer, ['set surface_color, 0x438D87', 'set surface_transparency, 0.35', 'show cartoon', 'set cartoon_color, 0xA4C9B7', 'set cartoon_color, 0xC88F6B, ss H']);
      if (scene.name === 'sticks') await executeCommand(viewer, 'set stick_color, atomic');
      if (scene.name === 'spheres') await executeCommand(viewer, 'set sphere_color, atomic');
    }
    const canvas = scene.mount.querySelector('canvas');
    if (scene.name !== 'hero') {
      await linkedViews.add(scene.name, viewer);
      const sync = () => linkedViews.request(scene.name);
      const options = { signal: linkedInput.signal, passive: true };
      // These listeners run after the viewer's input handlers. The next frame
      // reads the resulting camera after the renderer processes queued input.
      canvas.addEventListener('pointermove', event => { if (event.buttons) sync(); }, options);
      for (const type of ['pointerdown', 'pointerup', 'pointercancel', 'lostpointercapture', 'wheel']) {
        canvas.addEventListener(type, sync, options);
      }
    }
    canvas.setAttribute('aria-label', scene.name === 'hero'
      ? '1GVF aldolase molecular view. Drag to rotate, scroll to zoom. Use the reset button to restore the view.'
      : `1FSD ${scene.name} molecular view. Drag to rotate all four views, scroll to zoom all four. The reset button resets all four cameras.`);
    await viewer.show();
    scene.element.dataset.state = 'ready';
    setControls(scene.name, false);
  } catch (error) {
    failScene(scene, error);
  } finally {
    scene.element.setAttribute('aria-busy', 'false');
  }
}

function enqueueScene(scene) {
  if (!scene.startPromise) {
    scene.startPromise = startupQueue.then(() => startScene(scene));
    startupQueue = scene.startPromise;
  }
  return scene.startPromise;
}

// Serialise GPU startup; the four representations join the shared camera as they load.
const observer = new IntersectionObserver(entries => {
  for (const entry of entries) {
    if (!entry.isIntersecting) continue;
    observer.unobserve(entry.target);
    const scene = scenes.get(entry.target.dataset.scene);
    enqueueScene(scene);
  }
}, { rootMargin: '180px' });

for (const element of document.querySelectorAll('[data-scene]')) {
  const scene = { name: element.dataset.scene, element, mount: element.querySelector('.viewer-mount'), status: element.querySelector('.scene-status'), viewer: null };
  scenes.set(scene.name, scene);
  observer.observe(element);
}

async function changeHero(view, scrollToViewer = false) {
  const scene = scenes.get('hero');
  await enqueueScene(scene);
  if (scene.element.dataset.state !== 'ready' || scene.busy) return;
  scene.busy = true;
  setControls('hero', true);
  scene.element.setAttribute('aria-busy', 'true');
  try {
    await commands(scene.viewer, heroCommands(view));
    heroView = view;
    document.querySelectorAll('[data-hero-view]').forEach(button => button.setAttribute('aria-pressed', String(button.dataset.heroView === view)));
    document.querySelector('.scene-note').textContent = heroPresets[view].note;
    if (scrollToViewer) {
      document.getElementById('structure').scrollIntoView({ behavior: 'instant', block: 'center' });
      document.querySelector(`[data-hero-view="${view}"]`).disabled = false;
      document.querySelector(`[data-hero-view="${view}"]`).focus({ preventScroll: true });
    }
  } catch (error) {
    failScene(scene, error);
  } finally {
    scene.busy = false;
    scene.element.setAttribute('aria-busy', 'false');
    if (scene.element.dataset.state === 'ready') setControls('hero', false);
  }
}

document.querySelectorAll('[data-hero-view]').forEach(button => {
  button.addEventListener('click', () => changeHero(button.dataset.heroView));
});
document.querySelectorAll('[data-hero-shading]').forEach(button => {
  button.addEventListener('click', async () => {
    const scene = scenes.get('hero');
    if (scene.element.dataset.state !== 'ready' || scene.busy) return;
    scene.busy = true;
    setControls('hero', true);
    scene.element.setAttribute('aria-busy', 'true');
    try {
      // Change lighting alone: preserve the current camera and representation.
      await executeCommand(scene.viewer, `set shading_mode, ${button.dataset.heroShading}`);
      heroShading = button.dataset.heroShading;
      document.querySelectorAll('[data-hero-shading]').forEach(choice => {
        choice.setAttribute('aria-pressed', String(choice.dataset.heroShading === heroShading));
      });
    } catch (error) {
      failScene(scene, error);
    } finally {
      scene.busy = false;
      scene.element.setAttribute('aria-busy', 'false');
      if (scene.element.dataset.state === 'ready') setControls('hero', false);
    }
  });
});
document.getElementById('try-site').addEventListener('click', () => changeHero('site', true));

document.querySelector('.browser-link').addEventListener('click', async event => {
  event.preventDefault();
  const scene = scenes.get('hero');
  commandInput.scrollIntoView({
    behavior: matchMedia('(prefers-reduced-motion: reduce)').matches ? 'instant' : 'smooth',
    block: 'center',
  });
  await enqueueScene(scene);
  if (scene.element.dataset.state === 'ready') {
    commandInput.focus({ preventScroll: true });
  }
});

document.querySelectorAll('[data-reset]').forEach(button => {
  button.addEventListener('click', async () => {
    if (button.dataset.reset === 'hero') return changeHero(heroView);
    const scene = scenes.get(button.dataset.reset);
    if (!scene.viewer || scene.busy) return;
    scene.busy = true;
    button.disabled = true;
    try {
      await commands(scene.viewer, ['orient', 'zoom all, buffer=-3']);
      linkedViews.request(scene.name);
    }
    catch (error) { failScene(scene, error); }
    finally {
      scene.busy = false;
      if (scene.element.dataset.state === 'ready') button.disabled = false;
    }
  });
});

// Preserve browser back/forward caching; release GPU resources on a real exit.
window.addEventListener('pagehide', event => {
  if (event.persisted) return;
  observer.disconnect();
  linkedInput.abort();
  linkedViews.dispose();
  for (const scene of scenes.values()) {
    try { scene.viewer?.destroy(); } catch { /* A failed device can already be disposed. */ }
  }
});

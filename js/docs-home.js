import { PatinaeViewer } from './patinae-viewer.js';
import { executeCommand, loadStructure } from './viewer-commands.js';

const stage = document.querySelector('.docs-hub-stage');
const mount = stage.querySelector('.viewer-mount');
const status = stage.querySelector('.scene-status');
let viewer;
let disposed = false;

async function commands(list) {
  for (const command of list) await executeCommand(viewer, command);
}

async function resetCamera() {
  await commands(['orient capsid', 'zoom capsid and name CA, buffer=-40']);
}

function fail(error) {
  stage.dataset.state = 'error';
  stage.setAttribute('aria-busy', 'false');
  status.textContent = navigator.gpu
    ? 'The interactive view could not load. Explore 7KP3 on RCSB below.'
    : 'Interactive 3D needs WebGPU. Explore 7KP3 on RCSB below.';
  try { viewer?.destroy(); } catch { /* The GPU device may already be gone. */ }
  viewer = null;
  console.warn('Patinae documentation structure:', error);
}

async function start() {
  stage.dataset.state = 'loading';
  try {
    if (!navigator.gpu) throw new Error('WebGPU is unavailable');
    viewer = new PatinaeViewer(mount, {
      defer: true,
      memoryProfile: 'balanced',
      picking: false,
      revealDuration: matchMedia('(prefers-reduced-motion: reduce)').matches ? 0 : 150,
    });
    await viewer.init();
    await commands([
      'bg_color 0b0e10', 'set shading_mode, classic',
      'set ambient, 0.4', 'set direct, 0.6', 'set reflect, 0.25',
      'set specular, 0.15', 'set depth_cue, off', 'set fog, 0',
      'set silhouettes, off',
    ]);
    // Keep one monomer's coordinates; biounit uses the file's assembly operators.
    // Style the source before creating the snapshot so all copies share its colours.
    await loadStructure(viewer, './assets/pdb/7KP3.cif', { name: '7kp3', format: 'cif' });
    await commands([
      'as cartoon', 'color 0xC88F6B', 'color 0xA4C9B7, ss S',
      'biounit 7kp3, capsid, assembly=1', 'disable 7kp3',
    ]);
    await resetCamera();
    mount.querySelector('canvas').setAttribute('aria-label', '7KP3 AAV5 capsid, biological assembly 1. Drag to rotate, scroll to zoom.');
    await viewer.show();
    if (disposed) return;
    stage.dataset.state = 'ready';
    stage.setAttribute('aria-busy', 'false');
  } catch (error) {
    if (!disposed) fail(error);
  }
}

window.addEventListener('pagehide', event => {
  if (event.persisted) return;
  disposed = true;
  try { viewer?.destroy(); } catch { /* The GPU device may already be gone. */ }
});

start();

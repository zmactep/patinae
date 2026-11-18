import { initDocPages } from './docs-interactive.js';
import { initGuideControls, runExample } from './guide-controls.js';

export async function startGuide() {
  const setState = initGuideControls(document, { clipboard: navigator.clipboard });

  let viewer;
  setState('loading');
  try {
    if (!navigator.gpu) throw new Error('WebGPU is unavailable');
    const { PatinaeViewer } = await import('./patinae-viewer.js');
    viewer = new PatinaeViewer(document.getElementById('viewer-container'), {
      defer: true,
      picking: false,
      selectionOverlay: true,
      layout: [{ name: 'repl', slot: 'repl' }],
      slots: { repl: document.getElementById('viewer-repl') },
    });
    await viewer.init();
    initDocPages(viewer, { onStateChange: setState, runCommand: runExample });
  } catch (error) {
    console.warn('Guide viewer:', error);
    try { viewer?.destroy(); } catch { /* Initialization may have stopped early. */ }
    viewer = null;
    setState('unavailable');
    initDocPages(null);
  }

  window.addEventListener('pagehide', event => {
    if (!event.persisted) viewer?.destroy();
  });
}

import { waitForTasks } from './viewer-commands.js';

const STATE_MESSAGES = {
  loading: 'Preparing this view…',
  ready: 'Drag to rotate · Right-drag to zoom',
  error: 'The action could not finish. Check the response below or try Reset chapter.',
  unavailable: 'Interactive 3D is unavailable. You can still read every chapter and use the commands in desktop Patinae. Try a browser with WebGPU enabled.',
};

/** Wire lesson controls independently of GPU initialization. */
export function initGuideControls(root, { clipboard, schedule = setTimeout, cancel = clearTimeout }) {
  const status = root.querySelector('#guide-status');
  const panel = root.querySelector('.docs-viewer-panel');

  // Run and Copy read the same displayed command. Inline actions still use data-cmd.
  for (const button of root.querySelectorAll('[data-cmd-ref]')) {
    const code = root.getElementById(button.dataset.cmdRef);
    if (!code) throw new Error('Missing command example: ' + button.dataset.cmdRef);
    button.dataset.cmd = code.textContent.trim();
    button.setAttribute('aria-label', 'Run ' + button.dataset.cmd);
  }
  const controls = root.querySelectorAll('[data-cmd], [data-run-unavailable], [data-reset-chapter]');
  const copyTimers = new WeakMap();

  root.addEventListener('click', async event => {
    const button = event.target.closest('[data-copy]');
    if (!button) return;
    const code = root.getElementById(button.dataset.copy);
    try {
      await clipboard.writeText(code.textContent.trim());
      button.textContent = 'Copied';
    } catch {
      button.textContent = 'Select the command to copy';
    }
    cancel(copyTimers.get(button));
    copyTimers.set(button, schedule(() => {
      button.textContent = 'Copy';
      copyTimers.delete(button);
    }, 2200));
  });

  return function setState(state) {
    panel.setAttribute('aria-busy', String(state === 'loading'));
    const disabled = state === 'loading' || state === 'unavailable';
    controls.forEach(button => { button.disabled = disabled || button.hasAttribute('data-run-unavailable'); });
    const input = panel.querySelector('.repl-input');
    if (input) input.disabled = disabled;
    status.textContent = STATE_MESSAGES[state];
  };
}

// Use the same history and output renderer as the embedded command prompt.
export async function runExample(viewer, command) {
  const repl = viewer.panels.get('repl');
  repl.history.push(command);
  repl.historyIdx = repl.history.length;
  repl.appendLine(`Patinae> ${command}`, 'cmd');
  let reportedError = false;
  try {
    const reply = await viewer.execute(command);
    for (const message of reply.messages) {
      const level = message.kind.toLowerCase();
      repl.appendOutputMessage({ level, text: message.text, format: message.format });
      if (level === 'error') reportedError = true;
    }
    await waitForTasks(viewer, reply);
  } catch (error) {
    if (!reportedError) repl.appendOutputMessage({ level: 'error', text: error.message || String(error), format: 'text' });
    throw error;
  }
}

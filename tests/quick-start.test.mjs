import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { initGuideControls, runExample } from '../js/guide-controls.js';

function fixture(text) {
  const button = desktopOnly => ({
    disabled: false, dataset: {}, attributes: {},
    hasAttribute: () => desktopOnly,
    setAttribute(name, value) { this.attributes[name] = value; },
  });
  const run = button(false);
  run.dataset.cmdRef = 'example';
  const buttons = [run, button(false), button(true)];
  const copyButton = { dataset: { copy: 'example' } };
  const input = { disabled: false };
  const status = {};
  const panel = { setAttribute() {}, querySelector: () => input };
  const timers = new Map();
  let click, copied, nextTimer = 0;
  const root = {
    querySelector: selector => selector === '#guide-status' ? status : panel,
    querySelectorAll: selector => selector === '[data-cmd-ref]' ? [run] : buttons,
    getElementById: () => ({ textContent: text }),
    addEventListener: (_, handler) => { click = handler; },
  };
  const setState = initGuideControls(root, {
    clipboard: { writeText: async value => { copied = value; } },
    schedule: callback => { timers.set(++nextTimer, callback); return nextTimer; },
    cancel: id => timers.delete(id),
  });
  return {
    buttons, input, setState, runExample, timers, copyButton,
    async copy() {
      await click({ target: { closest: () => copyButton } });
      return copied;
    },
  };
}

test('every Copy button provides one complete command for a single-line prompt', async () => {
  const html = await readFile(new URL('../docs/quick-start.html', import.meta.url), 'utf8');
  const code = new Map([...html.matchAll(/<code id="([^"]+)">([\s\S]*?)<\/code>/g)].map(match => [match[1], match[2]]));
  const targets = [...html.matchAll(/data-copy="([^"]+)"/g)].map(match => match[1]);
  assert.ok(targets.length > 0);
  for (const id of targets) {
    assert.ok(code.has(id), `Missing copy target: ${id}`);
    const copied = await fixture(code.get(id)).copy();
    assert.ok(!/[\r\n;]/.test(copied), `Copy must provide one command: ${id}`);
    assert.equal(copied, code.get(id).trim());
  }
});

test('copy preserves a single command and spaces inside a quoted path', async () => {
  assert.equal(await fixture('save "my scenes/hemoglobin.prs"').copy(), 'save "my scenes/hemoglobin.prs"');
});

test('chapter loading blocks both action buttons and command-line submissions', () => {
  const f = fixture('');
  f.setState('loading');
  assert.ok(f.buttons.every(button => button.disabled));
  assert.equal(f.input.disabled, true);
  f.setState('ready');
  assert.ok(f.buttons.slice(0, 2).every(button => !button.disabled));
  assert.equal(f.buttons[2].disabled, true, 'Desktop-only Run stays disabled');
  assert.equal(f.input.disabled, false);
});


test('standalone examples have matching Run controls, except the copy-only session reset', async () => {
  const html = await readFile(new URL('../docs/quick-start.html', import.meta.url), 'utf8');
  const rows = [...html.matchAll(/<div class="command-line">([\s\S]*?)<\/div>/g)];
  assert.ok(rows.length > 0);
  for (const [, row] of rows) {
    const command = row.match(/<code[^>]*>([^<]+)<\/code>/)[1];
    if (command === 'reinitialize') {
      assert.doesNotMatch(row, /data-run-example|data-cmd=/);
      assert.match(row, /data-copy=/);
      continue;
    }
    assert.match(row, /data-run-example/);
    if (/^(save|png|load)\s/.test(command)) {
      assert.match(row, /data-run-unavailable title="[^"]+"/);
      assert.doesNotMatch(row, /data-cmd=/);
    } else {
      const id = row.match(/<code id="([^"]+)"/)[1];
      assert.equal(row.match(/data-cmd-ref="([^"]+)"/)[1], id);
      assert.equal(fixture(command).buttons[0].dataset.cmd, command.trim());
      assert.doesNotMatch(row, /data-cmd=/, 'Run must not store a second copy of the command');
    }
  }
});

test('Run records the command and renders responses before reporting a failure', async () => {
  const f = fixture('');
  const lines = [], messages = [];
  const repl = { history: [], appendLine: (...line) => lines.push(line), appendOutputMessage: message => messages.push(message) };
  let reply = { task_ids: [], result: { Ok: null }, messages: [{ kind: 'Info', text: 'Color help', format: 'markdown' }] };
  const viewer = { panels: new Map([['repl', repl]]), execute: async () => reply };
  await f.runExample(viewer, 'help color');
  assert.deepEqual(repl.history, ['help color']);
  assert.equal(repl.historyIdx, 1);
  assert.deepEqual(lines[0], ['Patinae> help color', 'cmd']);
  assert.equal(messages[0].text, 'Color help');
  assert.equal(messages[0].format, 'markdown');
  reply = { task_ids: [], result: { Err: 'missing color' }, messages: [{ kind: 'Error', text: 'missing color', format: 'text' }] };
  await assert.rejects(f.runExample(viewer, 'color'), /missing color/);
  assert.equal(messages[1].level, 'error');
  assert.deepEqual(repl.history, ['help color', 'color']);
  assert.equal(messages.length, 2, 'Do not duplicate reported errors');
  reply = { task_ids: [], result: { Err: 'missing color' }, messages: [] };
  await assert.rejects(f.runExample(viewer, 'color'), /missing color/);
  assert.equal(messages[2].text, 'missing color');
});


test('Run and Copy use the displayed text, including quoted paths', async () => {
  const f = fixture('  load "my scenes/hemoglobin.pdb"  ');
  assert.equal(f.buttons[0].dataset.cmd, await f.copy());
  assert.equal(f.buttons[0].attributes['aria-label'], 'Run load "my scenes/hemoglobin.pdb"');
});

test('repeated Copy clicks replace the previous feedback timer', async () => {
  const f = fixture('help');
  await f.copy();
  await f.copy();
  assert.equal(f.timers.size, 1);
  assert.equal(f.copyButton.textContent, 'Copied');
  [...f.timers.values()][0]();
  assert.equal(f.copyButton.textContent, 'Copy');
});

test('Run waits for background tasks before completing', async () => {
  const f = fixture('');
  let finish;
  const task = new Promise(resolve => { finish = resolve; });
  const repl = { history: [], appendLine() {}, appendOutputMessage() {} };
  const viewer = {
    panels: new Map([['repl', repl]]),
    execute: async () => ({ task_ids: [1], result: { Ok: null }, messages: [] }),
    tasks: { wait: () => task },
  };
  let completed = false;
  const running = f.runExample(viewer, 'fetch 1hho').then(() => { completed = true; });
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(completed, false);
  finish({ state: 'succeeded' });
  await running;
  assert.equal(completed, true);
});

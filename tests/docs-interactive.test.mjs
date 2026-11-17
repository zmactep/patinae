import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import vm from 'node:vm';

const source = (await readFile(new URL('../js/docs-interactive.js', import.meta.url), 'utf8'))
  .replace(/^import .*;\n/m, '').replace('export function initDocPages', 'function initDocPages');

function element(dataset = {}) {
  const classes = new Set();
  const listeners = {};
  return {
    dataset, style: {}, attributes: {}, listeners,
    classList: {
      add: name => classes.add(name), remove: name => classes.delete(name),
      contains: name => classes.has(name),
      toggle(name, force = !classes.has(name)) { force ? classes.add(name) : classes.delete(name); },
    },
    setAttribute(name, value) { this.attributes[name] = value; },
    removeAttribute(name) { delete this.attributes[name]; },
    hasAttribute(name) { return name in this.attributes; },
    addEventListener(name, callback) { listeners[name] = callback; },
  };
}

function fixture({ viewer = true, failCommand = false, load = async () => {} } = {}) {
  const pages = ['intro', 'exercise'].map(id => {
    const page = element({ title: id, src: `${id}.pdb`, command: `style ${id}` });
    page.id = id;
    page.buttons = [element({ nav: 'prev' }), element({ nav: 'next' })];
    page.counter = {};
    page.querySelectorAll = () => page.buttons;
    page.querySelector = () => page.counter;
    return page;
  });
  const links = pages.map(() => element());
  const listeners = {};
  const events = {};
  const history = [];
  const commands = [];
  const states = [];
  const errors = [];
  const inline = element({ cmd: 'colour test', cmdAlt: 'colour restored' });
  const document = {
    querySelectorAll(selector) {
      if (selector === '.doc-page') return pages;
      if (selector === '[data-cmd].toggled') return inline.classList.contains('toggled') ? [inline] : [];
      return [];
    },
    querySelector: () => ({ querySelectorAll: () => links }),
    addEventListener(name, callback) { (listeners[name] ??= []).push(callback); },
  };
  const context = vm.createContext({
    document, location: { hash: '#intro' },
    history: { pushState: (...args) => history.push(args[2]) },
    window: { scrollTo() {}, addEventListener(name, callback) { events[name] = callback; } },
    console: { error: (...args) => errors.push(args) },
    setTimeout: callback => callback(),
    executeCommand: async (_, command) => {
      commands.push(command);
      if (failCommand && command === 'colour test') throw new Error('Invalid selection');
    },
    loadStructure: async (_, url) => { commands.push(`load ${url}`); await load(url); },
  });
  vm.runInContext(source, context);
  context.initDocPages(viewer ? { core: { setDeferred() {} }, show: async () => {} } : null,
    { onStateChange: state => states.push(state) });
  const settle = () => new Promise(resolve => setImmediate(resolve));
  async function click(target, kind) {
    const event = { preventDefault() {}, target: { closest(selector) {
      return selector === '[data-cmd]' ? (kind === 'command' ? target : null)
        : (kind === 'nav' ? target : null);
    } } };
    await Promise.all(listeners.click.map(callback => callback(event)));
    await settle();
  }
  return { pages, links, commands, states, errors, inline, history, context, events, settle, click };
}

test('chapters, counters and history remain usable without a GPU viewer', async () => {
  const f = fixture({ viewer: false });
  await f.settle();
  await f.click(f.pages[0].buttons[1], 'nav');
  assert.equal(f.pages[1].style.display, 'block');
  assert.equal(f.pages[0].style.display, 'none');
  assert.equal(f.pages[1].counter.textContent, '2 of 2');
  assert.equal(f.links[1].attributes['aria-current'], 'step');
  assert.deepEqual(f.history, ['#exercise']);
  f.context.location.hash = '#intro';
  f.events.popstate();
  assert.equal(f.pages[0].style.display, 'block');
  assert.deepEqual(f.commands, []);
  assert.deepEqual(f.errors, []);
});

test('reset restores the chapter from its source and clears toggle state without adding history', async () => {
  const f = fixture();
  await f.settle();
  await f.click(f.inline, 'command');
  assert.equal(f.inline.classList.contains('toggled'), true);
  const reset = element();
  reset.setAttribute('data-reset-chapter', '');
  await f.click(reset, 'nav');
  assert.deepEqual(f.commands.slice(-4), ['reinitialize', 'load intro.pdb', 'style intro', 'bg_color 0b0e10']);
  assert.equal(f.inline.classList.contains('toggled'), false);
  assert.deepEqual(f.history, []);
  assert.equal(f.states.at(-1), 'ready');
});

test('a failed command reports an error and a later chapter can still load', async () => {
  const f = fixture({ failCommand: true });
  await f.settle();
  await f.click(f.inline, 'command');
  assert.equal(f.states.at(-1), 'error');
  await f.click(f.pages[0].buttons[1], 'nav');
  assert.equal(f.states.at(-1), 'ready');
  assert.equal(f.pages[1].style.display, 'block');
  assert.ok(f.commands.includes('load exercise.pdb'));
});

test('rapid chapter changes discard superseded queued scene setups', async () => {
  const f = fixture();
  f.links[1].listeners.click({ preventDefault() {} });
  await f.settle();
  assert.ok(!f.commands.includes('load intro.pdb'));
  assert.deepEqual(f.commands, ['reinitialize', 'load exercise.pdb', 'style exercise', 'bg_color 0b0e10']);
  assert.equal(f.states.at(-1), 'ready');
});

test('a superseded load failure cannot unlock a chapter that is still loading', async () => {
  let rejectIntro, finishExercise;
  const intro = new Promise((_, reject) => { rejectIntro = reject; });
  const exercise = new Promise(resolve => { finishExercise = resolve; });
  const f = fixture({ load: url => url === 'intro.pdb' ? intro : exercise });
  await f.settle();
  f.links[1].listeners.click({ preventDefault() {} });
  rejectIntro(new Error('Old chapter failed to load'));
  await f.settle();
  assert.equal(f.pages[1].style.display, 'block');
  assert.equal(f.states.at(-1), 'loading');
  finishExercise();
  await f.settle();
  assert.equal(f.states.at(-1), 'ready');
});

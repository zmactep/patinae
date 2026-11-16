import test from 'node:test';
import assert from 'node:assert/strict';
import { LinkedViews } from '../js/linked-views.js';

const initial = [1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 45, 2, 3, 4, 10, 80, 20];
const rotated = [0, 1, 0, -1, 0, 0, 0, 0, 1, 0, 0, 45, 2, 3, 4, 10, 80, 20];
const zoomed = [...rotated.slice(0, 11), 30, ...rotated.slice(12)];

function viewer() {
  return {
    camera: [...initial], writes: 0, failWrites: false,
    async execute(command) {
      if (command === 'get_view') {
        const message = `### cut below here and paste into the command line ###\nset_view (\\\n${this.camera.map(value => value.toFixed(6)).join(',\\\n')})\n### cut above here and paste into the command line ###`;
        return { result: { Ok: null }, task_ids: [], messages: [{ text: message }] };
      }
      if (this.failWrites) throw new Error('Device lost');
      assert.match(command, /^set_view \(/);
      this.camera = command.match(/\((.*)\)/)[1].split(',').map(Number);
      this.writes++;
      return { result: { Ok: null }, task_ids: [], messages: [] };
    },
  };
}

function setup() {
  let frames = [];
  const errors = [];
  const group = new LinkedViews({
    schedule: callback => { frames.push(callback); return callback; },
    cancel: callback => { frames = frames.filter(frame => frame !== callback); },
    onError: (name, error) => errors.push({ name, error }),
  });
  return { group, errors, tick: () => frames.shift()?.(), pending: () => frames.length };
}

test('any representation can lead rotation, zoom and a return to the initial camera', async () => {
  const { group, tick } = setup();
  const views = Array.from({ length: 4 }, viewer);
  for (const [i, view] of views.entries()) await group.add(String(i), view);
  for (const [source, camera] of [[2, rotated], [1, zoomed], [3, initial]]) {
    views[source].camera = [...camera];
    group.request(String(source));
    await tick();
    for (const view of views) assert.deepEqual(view.camera, camera);
  }
});

test('a view loaded later joins the current camera without resetting existing views', async () => {
  const { group, tick } = setup();
  const first = viewer(), later = viewer();
  await group.add('cartoon', first);
  first.camera = [...zoomed];
  group.request('cartoon');
  await tick();
  await group.add('surface', later);
  assert.deepEqual(later.camera, zoomed);
  assert.deepEqual(first.camera, zoomed);
});

test('rapid input is coalesced to one frame and the most recent source wins', async () => {
  const { group, tick, pending } = setup();
  const a = viewer(), b = viewer();
  await group.add('a', a); await group.add('b', b);
  a.camera = [...rotated]; b.camera = [...zoomed];
  group.request('a'); group.request('a'); group.request('b');
  assert.equal(pending(), 1);
  await tick();
  assert.deepEqual(a.camera, zoomed);
  assert.deepEqual(b.camera, zoomed);
  assert.equal(pending(), 0);
});

test('a slow camera read cannot overwrite a newer interaction in another view', async () => {
  const { group, tick, pending } = setup();
  const a = viewer(), b = viewer();
  await group.add('a', a); await group.add('b', b);
  a.camera = [...rotated];
  const staleReply = await a.execute('get_view');
  let release;
  a.execute = () => new Promise(resolve => { release = () => resolve(staleReply); });
  group.request('a');
  const firstFrame = tick();
  b.camera = [...zoomed]; group.request('b');
  release(); await firstFrame;
  assert.deepEqual(b.camera, zoomed);
  assert.equal(pending(), 1);
  group.remove('a');
  await tick();
  assert.equal(pending(), 0);
});

test('a failed target is removed while the remaining views keep following', async () => {
  const { group, tick, errors } = setup();
  const a = viewer(), b = viewer(), c = viewer();
  await group.add('a', a); await group.add('b', b); await group.add('c', c);
  b.failWrites = true; a.camera = [...rotated];
  group.request('a'); await tick();
  assert.deepEqual(c.camera, rotated);
  assert.equal(errors.length, 1);
  assert.equal(errors[0].name, 'b');
  a.camera = [...zoomed]; group.request('a'); await tick();
  assert.deepEqual(c.camera, zoomed);
  assert.equal(errors.length, 1);
});

test('malformed camera output is rejected instead of changing other cameras', async () => {
  const { group } = setup();
  const bad = viewer();
  bad.execute = async () => ({ result: { Ok: null }, task_ids: [], messages: [{ text: 'set_view (1, 2)' }] });
  await assert.rejects(group.add('bad', bad), /complete camera/);
});

test('unregistered views and disposed groups schedule no synchronization', async () => {
  const { group, pending } = setup();
  await group.add('cartoon', viewer());
  group.request('hero'); assert.equal(pending(), 0);
  group.request('cartoon'); assert.equal(pending(), 1);
  group.dispose(); assert.equal(pending(), 0);
  group.request('cartoon'); assert.equal(pending(), 0);
});

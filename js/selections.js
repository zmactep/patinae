import { startGuide } from './guide-viewer.js';

function overview(hemeRepresentation) {
  return [
    'hide everything',
    'show cartoon, polymer.protein',
    'show ' + hemeRepresentation + ', resn HEM',
    'color gray',
    'color chain, polymer',
    'color orange, resn HEM',
    'label polymer.protein and resi 1 and name CA, chain',
    'orient polymer.protein',
    'zoom polymer.protein, buffer=3',
  ];
}

const proteinAndHeme = [
  'select protein_a, chain A and polymer.protein',
  'select heme_a, chain A2 and resn HEM',
];
const nearAtoms = 'select near_a, protein_a within 4 of heme_a';
const widerAtoms = 'select wider_a, protein_a within 6 of heme_a';
const pocketResidues = 'select pocket_a, byres (near_a)';
const neighborhoodStyle = [
  'disable *_a',
  'hide everything',
  'color gray',
  'color chain, polymer',
  'show lines, byres (wider_a)',
  'show sticks, heme_a',
  'color orange, heme_a',
  'set sphere_scale, 0.36',
];
const hemeCamera = ['orient heme_a', 'turn y, 75', 'zoom heme_a, buffer=5'];
const highlightNear = 'color yellow, near_a';
const hemeOverview = overview('sticks');

const scenes = {
  overview: hemeOverview,
  'heme-lines': overview('lines'),
  reference: ['select hemes, resn HEM', 'disable hemes', ...hemeOverview],
  distance: [...proteinAndHeme, widerAtoms, ...neighborhoodStyle, ...hemeCamera],
  residues: [
    ...proteinAndHeme, widerAtoms, nearAtoms, pocketResidues, ...neighborhoodStyle,
    highlightNear, 'show spheres, near_a', ...hemeCamera,
  ],
};

// Deselect removes the names, so each comparison recreates its own inputs.
const hideMarkers = ['disable *_a', 'disable sample'];
const distanceComparison = [
  ...proteinAndHeme, nearAtoms, ...hideMarkers, highlightNear, 'hide spheres, protein_a',
];
const residueComparison = [...proteinAndHeme, nearAtoms, pocketResidues, ...hideMarkers];
const views = {
  prepare: ['hide cartoon, polymer.protein', 'show sticks, not solvent', 'show dots, solvent'],
  restore: ['hide dots, solvent', 'hide sticks, not solvent', 'show cartoon, polymer.protein', 'show sticks, resn HEM'],
  'four-angstroms': [...distanceComparison, 'show spheres, near_a'],
  'six-angstroms': [...distanceComparison, widerAtoms, 'disable wider_a', 'show spheres, wider_a'],
  'nearby-atoms': [...residueComparison, 'hide sticks, protein_a'],
  'whole-residues': [...residueComparison, 'show sticks, pocket_a'],
};

for (const chapter of document.querySelectorAll('[data-scene]')) {
  chapter.dataset.command = scenes[chapter.dataset.scene].join('; ');
}
for (const button of document.querySelectorAll('[data-view]')) {
  button.dataset.cmd = views[button.dataset.view].join('; ');
  if (button.dataset.viewAlt) button.dataset.cmdAlt = views[button.dataset.viewAlt].join('; ');
}
for (const button of document.querySelectorAll('.keyword-example')) {
  const command = 'select sample, ' + button.querySelector('code').textContent.trim();
  button.dataset.cmd = command;
  button.title = 'Run ' + command;
  button.setAttribute('aria-label', button.title);
}

const template = document.getElementById('command-line-actions');
for (const code of document.querySelectorAll('.command-line > code')) {
  const actions = template.content.cloneNode(true);
  actions.querySelector('[data-run-example]').dataset.cmdRef = code.id;
  const copy = actions.querySelector('[data-copy]');
  copy.dataset.copy = code.id;
  copy.setAttribute('aria-label', 'Copy ' + code.textContent.trim());
  code.after(actions);
}

await startGuide();

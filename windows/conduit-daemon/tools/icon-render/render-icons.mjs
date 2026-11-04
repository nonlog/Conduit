import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { Resvg } from '@resvg/resvg-js';

const here = path.dirname(fileURLToPath(import.meta.url));
const assets = path.resolve(here, '..', '..', 'assets');

// Exact-size monochrome link glyphs, generated from the same source as Rune.
// No colored tile, orange arrows, raster logo, or image-service dependency.
const brandSizes = [16, 20, 24, 30, 32, 36, 40, 48, 60, 64, 72, 80, 96, 256];
const traySizes = [16, 20, 24, 32, 40, 48, 64];
const explorerSizes = [16, 20, 24, 32, 40, 48];
const mark = fs.readFileSync(path.join(assets, 'conduit-mark.fragment.svg'), 'utf8');
if (!mark.includes('currentColor') || /<image|<script|https?:/i.test(mark)) {
  throw new Error('The app mark must be a self-contained themeable vector');
}
function markSvg(color) {
  return `<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24">${mark.replaceAll('currentColor', color)}</svg>`;
}

function render(svg, size) {
  const image = new Resvg(svg, { fitTo: { mode: 'width', value: size } });
  return Buffer.from(image.render().asPng());
}

function writeIco(file, frames) {
  const count = frames.length;
  const header = Buffer.alloc(6 + count * 16);
  header.writeUInt16LE(0, 0);
  header.writeUInt16LE(1, 2);
  header.writeUInt16LE(count, 4);
  let offset = header.length;
  frames.forEach(({ size, png }, index) => {
    const entry = 6 + index * 16;
    header[entry] = size >= 256 ? 0 : size;
    header[entry + 1] = size >= 256 ? 0 : size;
    header[entry + 2] = 0;
    header[entry + 3] = 0;
    header.writeUInt16LE(1, entry + 4);
    header.writeUInt16LE(32, entry + 6);
    header.writeUInt32LE(png.length, entry + 8);
    header.writeUInt32LE(offset, entry + 12);
    offset += png.length;
  });
  fs.writeFileSync(file, Buffer.concat([header, ...frames.map((frame) => frame.png)]));
}

fs.mkdirSync(assets, { recursive: true });

function writeThemeIcon(stem, color) {
  const svg = markSvg(color);
  const frames = brandSizes.map((size) => ({ size, png: render(svg, size) }));
  writeIco(path.join(assets, `${stem}.ico`), frames);
  fs.writeFileSync(path.join(assets, `${stem}.png`), render(svg, 512));
}

writeThemeIcon('conduit-icon-light', '#273247');
writeThemeIcon('conduit-icon-dark', '#E7EDF5');
writeThemeIcon('conduit-icon', '#4169E1');

const trayLightSvg = markSvg('#273247');
const trayDarkSvg = markSvg('#E7EDF5');
writeIco(
  path.join(assets, 'conduit-tray-light.ico'),
  traySizes.map((size) => ({ size, png: render(trayLightSvg, size) })),
);
writeIco(
  path.join(assets, 'conduit-tray-dark.ico'),
  traySizes.map((size) => ({ size, png: render(trayDarkSvg, size) })),
);

const explorerLightSvg = markSvg('#273247');
const explorerDarkSvg = markSvg('#E7EDF5');
writeIco(
  path.join(assets, 'conduit-explorer-light.ico'),
  explorerSizes.map((size) => ({ size, png: render(explorerLightSvg, size) })),
);
writeIco(
  path.join(assets, 'conduit-explorer-dark.ico'),
  explorerSizes.map((size) => ({ size, png: render(explorerDarkSvg, size) })),
);

for (const name of [
  'conduit-icon.png',
  'conduit-icon.ico',
  'conduit-icon-light.png',
  'conduit-icon-light.ico',
  'conduit-icon-dark.png',
  'conduit-icon-dark.ico',
  'conduit-tray-light.ico',
  'conduit-tray-dark.ico',
  'conduit-explorer-light.ico',
  'conduit-explorer-dark.ico',
]) {
  console.log(path.join(assets, name));
}

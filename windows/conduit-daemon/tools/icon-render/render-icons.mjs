import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { Resvg } from '@resvg/resvg-js';

const here = path.dirname(fileURLToPath(import.meta.url));
const assets = path.resolve(here, '..', '..', 'assets');
const fluent = path.join(assets, 'fluent');

const brandSizes = [16, 20, 24, 28, 32, 34, 40, 43, 44, 48, 51, 55, 60, 64, 66, 68, 77, 88, 96, 128, 256];
const traySizes = [16, 20, 24, 28, 32, 40, 48, 64];
const brandGlyph = '#5B5BD6';
const darkGlyph = '#202020';
const lightGlyph = '#F6F6F6';

function parseSource(file) {
  const svg = fs.readFileSync(file, 'utf8');
  const viewBox = svg.match(/viewBox="0 0 ([\d.]+) ([\d.]+)"/);
  const pathData = svg.match(/<path d="([^"]+)"/s);
  if (!viewBox || !pathData) throw new Error(`Could not parse Fluent source: ${file}`);
  return { width: Number(viewBox[1]), height: Number(viewBox[2]), d: pathData[1] };
}

function brandSource(size) {
  const sourceSize = size <= 18 ? 16 : size <= 22 ? 20 : 24;
  const weight = sourceSize < 24 ? 'regular' : 'filled';
  return parseSource(path.join(fluent, `ic_fluent_phone_desktop_${sourceSize}_${weight}.svg`));
}

function traySource(size) {
  const sourceSize = size <= 18 ? 16 : size <= 22 ? 20 : 24;
  return parseSource(path.join(fluent, `ic_fluent_phone_desktop_${sourceSize}_regular.svg`));
}

function brandSvg(size) {
  const source = brandSource(size);
  // Keep the mark transparent and monoline/monochrome like Sefirah instead of placing it in a
  // rounded app tile. Small frames use the matching Fluent 16/20px path so Explorer, the taskbar
  // and notification attribution never have to downscale the 24px geometry.
  const scale = size <= 20 ? 0.90 : 0.86;
  const pad = (source.width * (1 - scale) / 2).toFixed(3);
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${source.width}" height="${source.height}" viewBox="0 0 ${source.width} ${source.height}">
  <g transform="translate(${pad} ${pad}) scale(${scale})"><path d="${source.d}" fill="${brandGlyph}"/></g>
</svg>`;
}

function traySvg(size, color) {
  const source = traySource(size);
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${source.width}" height="${source.height}" viewBox="0 0 ${source.width} ${source.height}">
  <path d="${source.d}" fill="${color}"/>
</svg>`;
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
const brandFrames = brandSizes.map((size) => ({ size, png: render(brandSvg(size), size) }));
writeIco(path.join(assets, 'conduit-icon.ico'), brandFrames);
fs.writeFileSync(path.join(assets, 'conduit-icon.png'), render(brandSvg(512), 512));

const trayLight = traySizes.map((size) => ({ size, png: render(traySvg(size, darkGlyph), size) }));
const trayDark = traySizes.map((size) => ({ size, png: render(traySvg(size, lightGlyph), size) }));
writeIco(path.join(assets, 'conduit-tray-light.ico'), trayLight);
writeIco(path.join(assets, 'conduit-tray-dark.ico'), trayDark);

console.log(path.join(assets, 'conduit-icon.png'));
console.log(path.join(assets, 'conduit-icon.ico'));
console.log(path.join(assets, 'conduit-tray-light.ico'));
console.log(path.join(assets, 'conduit-tray-dark.ico'));
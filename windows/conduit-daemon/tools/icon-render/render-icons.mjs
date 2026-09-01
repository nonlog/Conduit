import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { Resvg } from '@resvg/resvg-js';

const here = path.dirname(fileURLToPath(import.meta.url));
const assets = path.resolve(here, '..', '..', 'assets');
const fluent = path.join(assets, 'fluent');

const brandSizes = [16, 20, 24, 28, 32, 34, 40, 43, 44, 48, 51, 55, 60, 64, 66, 68, 77, 88, 96, 128, 256];
const traySizes = [16, 20, 24, 28, 32, 40, 48, 64];
const darkGlyph = '#202020';
const lightGlyph = '#F6F6F6';

function parseSource(file) {
  const svg = fs.readFileSync(file, 'utf8');
  const viewBox = svg.match(/viewBox="0 0 ([\d.]+) ([\d.]+)"/);
  const pathData = svg.match(/<path d="([^"]+)"/s);
  if (!viewBox || !pathData) throw new Error(`Could not parse Fluent source: ${file}`);
  return { width: Number(viewBox[1]), height: Number(viewBox[2]), d: pathData[1] };
}

function glyphSource(size) {
  const sourceSize = size <= 18 ? 16 : size <= 22 ? 20 : 24;
  return parseSource(path.join(fluent, `ic_fluent_phone_desktop_${sourceSize}_regular.svg`));
}

function glyphSvg(size, color) {
  const source = glyphSource(size);
  // The Fluent Phone Desktop path itself has generous optical insets. Scale it slightly past the
  // source viewBox centre so the visible mark carries the same weight as neighbouring Explorer and
  // notification-area glyphs. The 16/20/24 source paths still leave enough native margin at 1.10x.
  const scale = 1.10;
  const padX = (source.width * (1 - scale) / 2).toFixed(3);
  const padY = (source.height * (1 - scale) / 2).toFixed(3);
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${source.width}" height="${source.height}" viewBox="0 0 ${source.width} ${source.height}">
  <g transform="translate(${padX} ${padY}) scale(${scale})"><path d="${source.d}" fill="${color}"/></g>
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

function writeThemeIcon(stem, color) {
  const frames = brandSizes.map((size) => ({ size, png: render(glyphSvg(size, color), size) }));
  writeIco(path.join(assets, `${stem}.ico`), frames);
  fs.writeFileSync(path.join(assets, `${stem}.png`), render(glyphSvg(512, color), 512));
}

writeThemeIcon('conduit-icon-light', darkGlyph);
writeThemeIcon('conduit-icon-dark', lightGlyph);

// Compatibility aliases stay monochrome. New shell integrations select a theme-specific asset.
writeThemeIcon('conduit-icon', darkGlyph);

const trayLight = traySizes.map((size) => ({ size, png: render(glyphSvg(size, darkGlyph), size) }));
const trayDark = traySizes.map((size) => ({ size, png: render(glyphSvg(size, lightGlyph), size) }));
writeIco(path.join(assets, 'conduit-tray-light.ico'), trayLight);
writeIco(path.join(assets, 'conduit-tray-dark.ico'), trayDark);

for (const name of [
  'conduit-icon.png',
  'conduit-icon.ico',
  'conduit-icon-light.png',
  'conduit-icon-light.ico',
  'conduit-icon-dark.png',
  'conduit-icon-dark.ico',
  'conduit-tray-light.ico',
  'conduit-tray-dark.ico',
]) {
  console.log(path.join(assets, name));
}

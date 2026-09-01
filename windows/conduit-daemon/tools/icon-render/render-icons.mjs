import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { Resvg } from '@resvg/resvg-js';

const here = path.dirname(fileURLToPath(import.meta.url));
const assets = path.resolve(here, '..', '..', 'assets');
const fluent = path.join(assets, 'fluent');

// Windows 11 looks for exact target-size frames before falling back to resampling a nearby one.
// Keep this list aligned with the documented AppList/taskbar/notification-area target sizes so
// 125%, 150%, 175%, 200%, and 250% scaling never turns a neighbouring bitmap into a soft icon.
const brandSizes = [16, 20, 24, 30, 32, 36, 40, 48, 60, 64, 72, 80, 96, 256];
const traySizes = [16, 20, 24, 32, 40, 48, 64];
// Explorer's compact context menu gives the glyph a small fixed slot. A filled Fluent source has
// the same phone/desktop silhouette but carries more optical weight there without making the app,
// taskbar, notification, or tray icon oversized.
const explorerSizes = [16, 20, 24, 32, 40, 48];
const darkGlyph = '#000000';
const lightGlyph = '#FFFFFF';

function parseSource(file) {
  const svg = fs.readFileSync(file, 'utf8');
  const viewBox = svg.match(/viewBox="0 0 ([\d.]+) ([\d.]+)"/);
  const pathData = svg.match(/<path d="([^"]+)"/s);
  if (!viewBox || !pathData) throw new Error(`Could not parse Fluent source: ${file}`);
  return { width: Number(viewBox[1]), height: Number(viewBox[2]), d: pathData[1] };
}

function glyphSource(size) {
  // Prefer a Fluent source whose native canvas scales by an integer. That preserves its hand-tuned
  // 1 px / half-pixel geometry in the small ICO frames instead of introducing a second fractional
  // transform before Windows draws it. The remaining target sizes use the richest 24 px source.
  const sourceSize = size >= 128 ? 24 : size % 24 === 0 ? 24 : size % 20 === 0 ? 20 : size % 16 === 0 ? 16 : 24;
  return parseSource(path.join(fluent, `ic_fluent_phone_desktop_${sourceSize}_regular.svg`));
}

function glyphSvg(size, color) {
  const source = glyphSource(size);
  // Do not enlarge this path with a fractional transform. In particular, the former 1.10x
  // transform moved the native 20 px tray strokes off the pixel grid at 125% DPI and made the icon
  // visibly fuzzy. Resvg now rasterises the original Fluent geometry directly at the requested
  // Windows target size.
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${source.width}" height="${source.height}" viewBox="0 0 ${source.width} ${source.height}">
  <path d="${source.d}" fill="${color}"/>
</svg>`;
}

function explorerGlyphSvg(color) {
  const source = parseSource(path.join(fluent, 'ic_fluent_phone_desktop_24_filled.svg'));
  // Explorer's modern menu gives third-party classic verbs a smaller optical icon box than its
  // built-ins. Tighten only this dedicated canvas around the Fluent path (~2..22) so Windows still
  // receives exact-size raster frames, but the mark reads at the same weight as Edit/Open with.
  return `<svg xmlns="http://www.w3.org/2000/svg" width="21" height="21" viewBox="1.5 1.5 21 21">
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

const explorerLight = explorerSizes.map((size) => ({ size, png: render(explorerGlyphSvg(darkGlyph), size) }));
const explorerDark = explorerSizes.map((size) => ({ size, png: render(explorerGlyphSvg(lightGlyph), size) }));
writeIco(path.join(assets, 'conduit-explorer-light.ico'), explorerLight);
writeIco(path.join(assets, 'conduit-explorer-dark.ico'), explorerDark);

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

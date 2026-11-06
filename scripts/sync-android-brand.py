#!/usr/bin/env python3
"""Keep Android platform icons on the same vector geometry as the Rust desktop.

Run without arguments to regenerate, or --check in CI. No image services or raster art.
"""
from pathlib import Path
import argparse
import xml.etree.ElementTree as ET


def render(paths: list[str], *, adaptive: bool = False, mono: bool = False, tile: bool = False) -> str:
    size = 108 if adaptive or tile else 24
    color = "#FF000000" if mono else "#FFFFFFFF"
    lines = ['<?xml version="1.0" encoding="utf-8"?>',
             '<!-- Generated from conduit-mark.fragment.svg by scripts/sync-android-brand.py. -->',
             '<vector xmlns:android="http://schemas.android.com/apk/res/android"',
             f'    android:width="{size}dp" android:height="{size}dp"',
             f'    android:viewportWidth="{size}" android:viewportHeight="{size}">']
    if tile:
        lines.append('    <path android:fillColor="#FF252B31" android:pathData="M24,0H84C97.25,0 108,10.75 108,24V84C108,97.25 97.25,108 84,108H24C10.75,108 0,97.25 0,84V24C0,10.75 10.75,0 24,0Z" />')
    if adaptive or tile:
        lines.append('    <group android:scaleX="2.5" android:scaleY="2.5" android:translateX="24" android:translateY="24">')
    for path in paths:
        lines.append(f'    <path android:pathData="{path}" android:fillColor="#00000000" android:strokeColor="{color}" android:strokeWidth="2" android:strokeLineCap="round" android:strokeLineJoin="round" />')
    if adaptive or tile:
        lines.append('    </group>')
    lines.append('</vector>')
    return '\n'.join(lines) + '\n'


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    source = root / 'windows/conduit-daemon/assets/conduit-mark.fragment.svg'
    fragment = ET.fromstring('<svg>' + source.read_text(encoding='utf-8') + '</svg>')
    paths = [node.attrib['d'] for node in fragment.findall('path')]
    if len(paths) != 2:
        raise SystemExit('Expected the two outlined connection paths')
    drawable = root / 'android/app/src/main/res/drawable'
    files = {
        'ic_brand_sync.xml': render(paths),
        'ic_stat_link.xml': render(paths),
        'ic_launcher_foreground.xml': render(paths, adaptive=True),
        'ic_launcher_monochrome.xml': render(paths, adaptive=True, mono=True),
        'ic_share_target.xml': render(paths, tile=True),
        'ic_launcher_background.xml': '<?xml version="1.0" encoding="utf-8"?>\n<shape xmlns:android="http://schemas.android.com/apk/res/android" android:shape="rectangle">\n    <solid android:color="#FF252B31" />\n</shape>\n',
    }
    for name, expected in files.items():
        path = drawable / name
        if args.check:
            if not path.is_file() or path.read_text(encoding='utf-8') != expected:
                raise SystemExit(f'Android icon is stale: {name}; run {Path(__file__).name}')
        else:
            path.write_text(expected, encoding='utf-8', newline='\n')
    print('ANDROID_BRAND_GEOMETRY_MATCHES_DESKTOP')


if __name__ == '__main__':
    main()

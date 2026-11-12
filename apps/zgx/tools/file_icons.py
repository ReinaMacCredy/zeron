#!/usr/bin/env python3
"""File and folder icons for the design system: PKief's Material Icon Theme
(https://github.com/material-extensions/vscode-material-icon-theme, MIT),
the icons its mapping uses copied flat under ui/assets/icons/files/, and its
lookup tables written as ui/ds/file_icons_map.luau. Run it again on a newer
copy of the theme (an unpacked VS Code extension, e.g.
~/.vscode/extensions/pkief.material-icon-theme-5.40.0):

    python3 apps/zgx/tools/file_icons.py <theme dir>
"""

import json
import os
import shutil
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ZGX = os.path.dirname(HERE)
OUT_ASSETS = os.path.join(ZGX, "ui", "assets", "icons", "files")
OUT_MAP = os.path.join(ZGX, "ui", "ds", "file_icons_map.luau")
LIGHT = "_light"
OPEN = "-open"


def luau_table(name, mapping):
    lines = [f"\t{name} = {{"]
    for key in sorted(mapping):
        lines.append(f"\t\t[{json.dumps(key, ensure_ascii=False)}] = {json.dumps(mapping[key], ensure_ascii=False)},")
    lines.append("\t},")
    return "\n".join(lines)


def lowered(table):
    out = {}
    for key, icon in table.items():
        out.setdefault(key.lower(), icon)
    return out


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    theme_dir = sys.argv[1]
    manifest = os.path.join(theme_dir, "dist", "material-icons.json")
    with open(manifest) as f:
        theme = json.load(f)
    with open(os.path.join(theme_dir, "package.json")) as f:
        version = json.load(f)["version"]
    defs = theme["iconDefinitions"]

    names = lowered(theme["fileNames"])
    extensions = lowered(theme["fileExtensions"])
    folders = lowered(theme["folderNames"])
    for key, icon in folders.items():
        assert theme["folderNamesExpanded"].get(key, icon + OPEN) == icon + OPEN, key

    used = {theme["file"], theme["folder"], theme["rootFolder"]}
    used |= set(names.values()) | set(extensions.values())
    for icon in set(folders.values()) | {theme["folder"], theme["rootFolder"]}:
        used |= {icon, icon + OPEN}
    used |= {icon + LIGHT for icon in used if icon + LIGHT in defs}
    missing = sorted(icon for icon in used if icon not in defs)
    assert not missing, missing

    if os.path.isdir(OUT_ASSETS):
        shutil.rmtree(OUT_ASSETS)
    os.makedirs(OUT_ASSETS)
    for icon in sorted(used):
        shutil.copy(os.path.join(theme_dir, "dist", defs[icon]["iconPath"]), os.path.join(OUT_ASSETS, icon + ".svg"))
    shutil.copy(os.path.join(theme_dir, "LICENSE.txt"), os.path.join(OUT_ASSETS, "LICENSE.material-icon-theme.txt"))

    light = {icon: True for icon in sorted(used) if icon + LIGHT in used}
    with open(OUT_MAP, "w") as f:
        f.write(f"-- Written by tools/file_icons.py from the Material Icon Theme {version}.\n")
        f.write("-- Don't edit; run the script again. Each name is an icon under\n")
        f.write("-- assets/icons/files/; a folder's open one is its name and `-open`,\n")
        f.write("-- and one in `light` has its own drawing for light themes, `_light`.\n\n")
        f.write("return {\n")
        f.write(f'\tfile = "{theme["file"]}",\n')
        f.write(f'\tfolder = "{theme["folder"]}",\n')
        f.write(f'\troot = "{theme["rootFolder"]}",\n')
        f.write(luau_table("names", names) + "\n")
        f.write(luau_table("extensions", extensions) + "\n")
        f.write(luau_table("folders", folders) + "\n")
        f.write(luau_table("light", light) + "\n")
        f.write("}\n")
    print(f"{len(used)} icons -> {os.path.relpath(OUT_ASSETS, ZGX)}, map -> {os.path.relpath(OUT_MAP, ZGX)}")


if __name__ == "__main__":
    main()

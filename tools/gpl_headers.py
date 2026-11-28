# Hangar - a community-made updater and launcher for DCS World.
# Copyright (C) 2026 ChonkieMonkie
# SPDX-License-Identifier: GPL-3.0-or-later
# Free software under the GNU GPL v3 or later, WITHOUT ANY WARRANTY. See LICENSE or <https://www.gnu.org/licenses/>.

"""Adds or checks the GPL notice at the top of every Hangar source file.

    python tools/gpl_headers.py          add the notice where it is missing
    python tools/gpl_headers.py --check  list files without it and exit 1 (used in CI)

JSON files cannot hold comments and generated files (icons, lock files) get no notice.
"""

import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LINES = [
    "Hangar - a community-made updater and launcher for DCS World.",
    "Copyright (C) 2026 ChonkieMonkie",
    "SPDX-License-Identifier: GPL-3.0-or-later",
    "Free software under the GNU GPL v3 or later, WITHOUT ANY WARRANTY. See LICENSE or <https://www.gnu.org/licenses/>.",
]
DIRS = ["app/src", "app/src-tauri/src", "app/src-tauri/examples", "tools"]
FILES = ["app/src-tauri/build.rs", "app/vite.config.js"]
EXT = {".rs", ".ts", ".js", ".svelte", ".css", ".html", ".py", ".ps1"}


def header(ext):
    if ext in (".rs", ".ts", ".js"):
        return "".join(f"// {l}\n" for l in LINES) + "\n"
    if ext in (".py", ".ps1"):
        return "".join(f"# {l}\n" for l in LINES) + "\n"
    if ext == ".css":
        return "/* " + "\n   ".join(LINES) + " */\n\n"
    return "<!--\n" + "".join(f"  {l}\n" for l in LINES) + "-->\n"


def sources():
    out = [os.path.join(ROOT, f) for f in FILES]
    for d in DIRS:
        for base, dirs, names in os.walk(os.path.join(ROOT, d)):
            dirs[:] = [x for x in dirs if x not in ("node_modules", "target", ".svelte-kit")]
            out += [os.path.join(base, n) for n in names if os.path.splitext(n)[1] in EXT]
    return sorted(set(out))


def main():
    check = "--check" in sys.argv
    missing = []
    for path in sources():
        with open(path, encoding="utf-8-sig") as f:
            text = f.read()
        if "SPDX-License-Identifier: GPL-3.0-or-later" in text[:600]:
            continue
        rel = os.path.relpath(path, ROOT).replace("\\", "/")
        missing.append(rel)
        if check:
            continue
        ext = os.path.splitext(path)[1]
        if ext == ".html" and text.lower().startswith("<!doctype"):
            first, _, rest = text.partition("\n")
            text = first + "\n" + header(ext) + rest  # a comment before the doctype would break standards mode
        else:
            text = header(ext) + text
        with open(path, "w", encoding="utf-8", newline="") as f:
            f.write(text)
    if check:
        if missing:
            print("Missing the GPL notice (run python tools/gpl_headers.py):\n  " + "\n  ".join(missing))
            return 1
        print("All source files carry the GPL notice.")
        return 0
    print(f"Added the notice to {len(missing)} files." + ("".join(f"\n  {m}" for m in missing)))
    return 0


if __name__ == "__main__":
    sys.exit(main())

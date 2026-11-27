"""Renders Hangar's app icon (logo E "Bay marker", B-024) as PNG/ICO.

Usage:  python tools/make_icons.py <out_dir>
Writes icon-1024.png (source for `npx tauri icon`) and icon.ico / 32x32.png with
hand-tuned small sizes: heavier lines at 32 px and below, only the H at 16 px.
Needs Pillow.
"""
import os
import sys

from PIL import Image, ImageDraw

INK = (232, 237, 235, 255)
PHOSPHOR = (79, 214, 156, 255)
TILE = (21, 29, 32, 255)
BORDER = (44, 55, 60, 255)

# One variant per size range: (bracket polylines, H lines, stroke, corner radius, round crossbar)
BRACKETS_FULL = [[(7, 15), (7, 7), (15, 7)], [(33, 7), (41, 7), (41, 15)], [(41, 33), (41, 41), (33, 41)], [(15, 41), (7, 41), (7, 33)]]
BRACKETS_32 = [[(7, 14), (7, 7), (14, 7)], [(34, 7), (41, 7), (41, 14)], [(41, 34), (41, 41), (34, 41)], [(14, 41), (7, 41), (7, 34)]]
BRACKETS_24 = [[(7, 13), (7, 7), (13, 7)], [(35, 7), (41, 7), (41, 13)], [(41, 35), (41, 41), (35, 41)], [(13, 41), (7, 41), (7, 35)]]


def variant(size):
    if size >= 96:
        return BRACKETS_FULL, [((18, 15), (18, 33)), ((30, 15), (30, 33))], ((21, 24), (27, 24)), 3.4, 11, True
    if size >= 48:
        return BRACKETS_FULL, [((18, 15), (18, 33)), ((30, 15), (30, 33))], ((21, 24), (27, 24)), 4.0, 11, True
    if size >= 32:
        return BRACKETS_32, [((18, 15), (18, 33)), ((30, 15), (30, 33))], ((21, 24), (27, 24)), 5.0, 11, False
    if size >= 24:
        return BRACKETS_24, [((17, 15), (17, 33)), ((31, 15), (31, 33))], ((17, 24), (31, 24)), 6.0, 10, False
    return [], [((16, 13), (16, 35)), ((32, 13), (32, 35))], ((16, 24), (32, 24)), 7.5, 9, False


def render(size):
    ss = 8 if size <= 64 else 4
    S = size * ss
    k = S / 48.0
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    brackets, posts, bar, stroke, radius, butt_bar = variant(size)
    border = size >= 48
    d.rounded_rectangle([0, 0, S - 1, S - 1], radius=radius * k, fill=TILE,
                        outline=BORDER if border else None, width=max(1, round(S / 96)) if border else 0)

    def pt(p):  # logo space (48 grid, scaled 0.7 and centred like the canvas tiles)
        return (7.2 + 0.7 * p[0]) * k, (7.2 + 0.7 * p[1]) * k

    w = stroke * 0.7 * k

    def line(points, color, round_caps=True):
        pts = [pt(p) for p in points]
        d.line(pts, fill=color, width=round(w), joint="curve")
        if round_caps:
            for x, y in (pts[0], pts[-1]):
                d.ellipse([x - w / 2, y - w / 2, x + w / 2, y + w / 2], fill=color)

    for b in brackets:
        line(b, INK)
    for p in posts:
        line(p, PHOSPHOR)
    line(bar, PHOSPHOR, round_caps=not butt_bar)
    return img.resize((size, size), Image.LANCZOS)


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else "."
    os.makedirs(out, exist_ok=True)
    render(1024).save(os.path.join(out, "icon-1024.png"))
    sizes = [16, 20, 24, 32, 40, 48, 64, 128, 256]
    frames = [render(s) for s in sizes]
    frames[-1].save(os.path.join(out, "icon.ico"), sizes=[(s, s) for s in sizes], append_images=frames[:-1])
    render(32).save(os.path.join(out, "32x32.png"))
    render(64).save(os.path.join(out, "favicon.png"))
    print("ok")


if __name__ == "__main__":
    main()

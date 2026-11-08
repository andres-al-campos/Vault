#!/usr/bin/env python3
"""Draw the app icon's V and regenerate every icon file from it.

    python3 dev/icon.py

Writes src-tauri/Vault.icon/Assets/v.svg (the layered icon macOS 26 draws), renders
the full tile to src-tauri/icon-src.png with Chrome, and runs `tauri icon` on it for
the old-format set in src-tauri/icons. Rebuild with ./build.sh to see it.
"""
import math
import pathlib
import shutil
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent

# Corner radius on the 1024 canvas, the same at every corner, the notch included.
# Tuned by eye: at 8 and 10 the rounding didn't show in a 59pt Dock (about 1px).
RADIUS = 16

# The V's outline in the original drawing's units, placed by translate(191 234) scale(4).
# A downward play triangle with a cut in its top edge; the numbers are from the
# equilateral version (AGENTS.md, the app-icon Principle).
POINTS = [(1, 1), (63.40, 1), (80.25, 29.19), (97.10, 1), (159.5, 1), (80.25, 138.0)]
OUTSET = 1  # the old art was this outline stroked 2 wide, so its edge sat 1 unit out
SCALE, DX, DY = 4, 191, 234

TILE_RX = 230
GRADIENT = ("#ee3148", "#d81f36")


def offset(pts, d):
    """Move every edge outward by d and intersect neighbours."""
    n = len(pts)
    area = sum(pts[i][0] * pts[(i + 1) % n][1] - pts[(i + 1) % n][0] * pts[i][1] for i in range(n))
    s = -1 if area > 0 else 1
    lines = []
    for i in range(n):
        (x1, y1), (x2, y2) = pts[i], pts[(i + 1) % n]
        dx, dy = x2 - x1, y2 - y1
        length = math.hypot(dx, dy)
        lines.append(((x1 - dy / length * s * d, y1 + dx / length * s * d), (dx, dy)))
    out = []
    for i in range(n):
        (mp, mv), (lp, lv) = lines[i - 1], lines[i]
        cross = mv[0] * lv[1] - mv[1] * lv[0]
        t = ((lp[0] - mp[0]) * lv[1] - (lp[1] - mp[1]) * lv[0]) / cross
        out.append((mp[0] + mv[0] * t, mp[1] + mv[1] * t))
    return out


def filleted_path(pts, r):
    """An SVG path through pts with an arc of radius r at every corner, convex or not."""
    n = len(pts)
    parts = []
    for i in range(n):
        a, v, b = pts[i - 1], pts[i], pts[(i + 1) % n]
        u1 = ((a[0] - v[0]), (a[1] - v[1]))
        u2 = ((b[0] - v[0]), (b[1] - v[1]))
        l1, l2 = math.hypot(*u1), math.hypot(*u2)
        u1, u2 = (u1[0] / l1, u1[1] / l1), (u2[0] / l2, u2[1] / l2)
        angle = math.acos(max(-1, min(1, u1[0] * u2[0] + u1[1] * u2[1])))
        t = r / math.tan(angle / 2)
        t1 = (v[0] + u1[0] * t, v[1] + u1[1] * t)
        t2 = (v[0] + u2[0] * t, v[1] + u2[1] * t)
        turn = (v[0] - a[0]) * (b[1] - v[1]) - (v[1] - a[1]) * (b[0] - v[0])
        sweep = 1 if turn > 0 else 0
        parts.append(f"{'M' if i == 0 else 'L'}{t1[0]:.2f} {t1[1]:.2f}")
        parts.append(f"A{r} {r} 0 0 {sweep} {t2[0]:.2f} {t2[1]:.2f}")
    return "".join(parts) + "Z"


def v_path():
    outline = [(DX + SCALE * x, DY + SCALE * y) for x, y in offset(POINTS, OUTSET)]
    return filleted_path(outline, RADIUS)


def main():
    path = v_path()
    svg = ROOT / "src-tauri/Vault.icon/Assets/v.svg"
    svg.write_text(
        '<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024">'
        f'<path d="{path}" fill="#fff"/></svg>\n'
    )

    # The old-format icon is the whole tile, drawn here since nothing else does.
    tile = (
        '<!doctype html><html><body style="margin:0">'
        '<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024">'
        '<defs><linearGradient id="g" gradientTransform="rotate(70 .5 .5)">'
        f'<stop offset="0" stop-color="{GRADIENT[0]}"/><stop offset="1" stop-color="{GRADIENT[1]}"/>'
        f'</linearGradient></defs><rect width="1024" height="1024" rx="{TILE_RX}" fill="url(#g)"/>'
        f'<path d="{path}" fill="#fff"/></svg></body></html>'
    )
    chrome = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
    if not pathlib.Path(chrome).exists():
        sys.exit("Google Chrome wasn't found, so the old-format icons can't be rendered. Install Chrome, "
                 "then re-run python3 dev/icon.py. v.svg was written.")
    png = ROOT / "src-tauri/icon-src.png"
    with tempfile.TemporaryDirectory() as tmp:
        html = pathlib.Path(tmp, "tile.html")
        html.write_text(tile)
        shot = pathlib.Path(tmp, "tile.png")
        proc = subprocess.Popen(
            [chrome, "--headless=new", "--disable-gpu", "--hide-scrollbars",
             "--default-background-color=00000000", "--window-size=1024,1024",
             f"--user-data-dir={tmp}/profile", f"--screenshot={shot}", html.as_uri()],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        # Headless Chrome can stay running after writing the screenshot, so this waits
        # for the file rather than for the process.
        for _ in range(300):
            if shot.exists() and shot.stat().st_size > 0:
                break
            if proc.poll() is not None:
                break
            subprocess.run(["sleep", "0.1"])
        proc.kill()
        if not shot.exists():
            sys.exit("Chrome didn't write the icon render. Run Chrome headless by hand with the "
                     "command in dev/icon.py to see why, then re-run it. v.svg was written.")
        shutil.copy(shot, png)

    subprocess.run(["npx", "tauri", "icon", str(png), "-o", str(ROOT / "src-tauri/icons")],
                   cwd=ROOT, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    print(f"Icon drawn at radius {RADIUS}. Run ./build.sh to install it.")


if __name__ == "__main__":
    main()

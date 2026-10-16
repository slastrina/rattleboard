"""Generates the app icon source and the tray icons.

    python3 scripts/make_icons.py        (needs Pillow)
    npx tauri icon assets/app-icon.png   (expands the app icon into src-tauri/icons)
"""
from pathlib import Path
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parent.parent
FLAPS = ['#d93a2f', '#ec7a26', '#f2c230', '#2f9a52', '#2b6bd1', '#7d4cc2']


def app_icon(size=1024):
    s = 4  # supersample
    W = size * s
    im = Image.new('RGBA', (W, W), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    m = int(W * 0.09)
    d.rounded_rectangle([m, m, W - m, W - m], radius=int(W * 0.2), fill='#0a0a0a')
    cols, rows = 3, 2
    gx, gy = int(W * 0.035), int(W * 0.05)
    tw = (W - 2 * m - 2 * int(W * 0.1) - gx * (cols - 1)) // cols
    th = int(tw * 1.42)
    x0 = (W - (tw * cols + gx * (cols - 1))) // 2
    y0 = (W - (th * rows + gy * (rows - 1))) // 2
    for r in range(rows):
        for c in range(cols):
            x, y = x0 + c * (tw + gx), y0 + r * (th + gy)
            d.rounded_rectangle([x, y, x + tw, y + th], radius=int(tw * 0.1), fill=FLAPS[r * cols + c])
            d.rectangle([x, y + th // 2 - s * 3, x + tw, y + th // 2 + s * 3], fill=(0, 0, 0, 230))
    return im.resize((size, size), Image.LANCZOS)


def tray_glyph(px, ink, dot=None, bg=None):
    """The menu-bar glyph from the design: an 18x12 rounded frame holding a 3x2 grid."""
    s = 8
    W = px * s
    im = Image.new('RGBA', (W, W), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    if bg:
        d.rounded_rectangle([0, 0, W - 1, W - 1], radius=int(W * 0.22), fill=bg)
    u = W / 22  # one design pixel
    fw, fh = 18 * u, 12 * u
    x0, y0 = (W - fw) / 2, (W - fh) / 2
    d.rounded_rectangle([x0, y0, x0 + fw, y0 + fh], radius=3 * u, outline=ink, width=round(1.5 * u))
    pad, gap = 3 * u, 1 * u
    cw = (fw - 2 * pad - 2 * gap) / 3
    ch = (fh - 2 * pad - gap) / 2
    for r in range(2):
        for c in range(3):
            cx, cy = x0 + pad + c * (cw + gap), y0 + pad + r * (ch + gap)
            d.rectangle([cx, cy, cx + cw, cy + ch], fill=ink)
    if dot:
        rr = 3.2 * u
        cx, cy = x0 + fw - 0.5 * u, y0 + 0.5 * u
        d.ellipse([cx - rr - u, cy - rr - u, cx + rr + u, cy + rr + u], fill=(0, 0, 0, 0) if not bg else bg)
        d.ellipse([cx - rr, cy - rr, cx + rr, cy + rr], fill=dot)
    return im.resize((px, px), Image.LANCZOS)


if __name__ == '__main__':
    (ROOT / 'assets').mkdir(exist_ok=True)
    app_icon().save(ROOT / 'assets' / 'app-icon.png')
    out = ROOT / 'src-tauri' / 'icons'
    out.mkdir(parents=True, exist_ok=True)
    # macOS: template images (black + alpha), tinted by the system for light/dark menu bars
    tray_glyph(44, 'black').save(out / 'tray-mac.png')
    tray_glyph(44, 'black', dot='black').save(out / 'tray-mac-unread.png')
    # Windows: a self-contained tile that reads on both light and dark taskbars
    tray_glyph(32, '#f0eee8', bg='#1c1c1c').save(out / 'tray-win.png')
    tray_glyph(32, '#f0eee8', dot='#e8711f', bg='#1c1c1c').save(out / 'tray-win-unread.png')
    print('icons written')

"""Generate the app icon set (no external assets needed)."""
from PIL import Image, ImageDraw, ImageFilter
import os

S = 1024
OUT = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "assets")
os.makedirs(OUT, exist_ok=True)

A = (110, 123, 255)    # indigo
B = (186, 128, 255)    # violet
C = (86, 214, 214)     # teal accent


def lerp(c1, c2, t):
    return tuple(int(c1[i] + (c2[i] - c1[i]) * t) for i in range(3))


def gradient_diag(size, c1, c2):
    base = Image.new("RGB", (2, 2))
    base.putpixel((0, 0), c1)
    base.putpixel((1, 0), lerp(c1, c2, 0.5))
    base.putpixel((0, 1), lerp(c1, c2, 0.5))
    base.putpixel((1, 1), c2)
    return base.resize((size, size), Image.BICUBIC)


def squircle_mask(size, radius_ratio=0.26):
    m = Image.new("L", (size * 4, size * 4), 0)
    d = ImageDraw.Draw(m)
    d.rounded_rectangle(
        [0, 0, size * 4 - 1, size * 4 - 1],
        radius=int(size * 4 * radius_ratio),
        fill=255,
    )
    return m.resize((size, size), Image.LANCZOS)


def make_icon(size=S):
    # background: deep slate with a colored mesh
    bg = Image.new("RGB", (size, size), (10, 11, 18))
    mesh = Image.new("RGB", (size, size), (0, 0, 0))
    md = ImageDraw.Draw(mesh)
    md.ellipse([-size * 0.25, -size * 0.35, size * 0.85, size * 0.6], fill=(58, 66, 140))
    md.ellipse([size * 0.28, size * 0.15, size * 1.2, size * 1.1], fill=(96, 52, 130))
    md.ellipse([size * 0.05, size * 0.62, size * 0.8, size * 1.2], fill=(24, 66, 90))
    mesh = mesh.filter(ImageFilter.GaussianBlur(size * 0.12))
    bg = Image.blend(bg, mesh, 0.85)

    # bubble glyph
    glyph = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    g = ImageDraw.Draw(glyph)
    bw, bh = size * 0.50, size * 0.38
    x0 = (size - bw) / 2
    y0 = size * 0.30
    g.rounded_rectangle([x0, y0, x0 + bw, y0 + bh], radius=bh * 0.42, fill=(255, 255, 255, 255))
    # tail
    g.polygon(
        [(x0 + bw * 0.22, y0 + bh * 0.92), (x0 + bw * 0.30, y0 + bh * 1.42), (x0 + bw * 0.48, y0 + bh * 0.92)],
        fill=(255, 255, 255, 255),
    )
    # dots, colored by the accent gradient
    dot_r = size * 0.028
    cy = y0 + bh / 2
    for i, t in enumerate((0.0, 0.5, 1.0)):
        cx = x0 + bw * (0.5 + (i - 1) * 0.20)
        col = lerp(A, B, t)
        g.ellipse([cx - dot_r, cy - dot_r, cx + dot_r, cy + dot_r], fill=col + (255,))

    img = bg.convert("RGBA")
    img.alpha_composite(glyph)

    # glass sheen across the top half
    sheen = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    sd = ImageDraw.Draw(sheen)
    sd.ellipse([-size * 0.15, -size * 0.55, size * 1.15, size * 0.52], fill=(255, 255, 255, 46))
    sheen = sheen.filter(ImageFilter.GaussianBlur(size * 0.05))
    img.alpha_composite(sheen)

    img.putalpha(squircle_mask(size))
    return img


def make_tray(size=64):
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    bw, bh = size * 0.72, size * 0.54
    x0, y0 = (size - bw) / 2, (size - bh) / 2 - size * 0.04
    grad = gradient_diag(size, A, B).convert("RGBA")
    shape = Image.new("L", (size, size), 0)
    sd = ImageDraw.Draw(shape)
    sd.rounded_rectangle([x0, y0, x0 + bw, y0 + bh], radius=bh * 0.38, fill=255)
    sd.polygon([(x0 + bw * 0.24, y0 + bh * 0.86), (x0 + bw * 0.32, y0 + bh * 1.36), (x0 + bw * 0.52, y0 + bh * 0.86)], fill=255)
    img.paste(grad, (0, 0), shape)
    # dots punched out
    hole = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    hd = ImageDraw.Draw(hole)
    r = size * 0.045
    cy = y0 + bh / 2
    for i in range(3):
        cx = x0 + bw * (0.5 + (i - 1) * 0.22)
        hd.ellipse([cx - r, cy - r, cx + r, cy + r], fill=(255, 255, 255, 255))
    img = Image.alpha_composite(img, hole)
    return img


icon = make_icon()
icon.save(os.path.join(OUT, "icon.png"))
icon.save(os.path.join(OUT, "icon.ico"), sizes=[(256, 256), (128, 128), (64, 64), (48, 48), (32, 32), (16, 16)])
icon.resize((256, 256), Image.LANCZOS).save(os.path.join(OUT, "icon@256.png"))
make_tray(64).save(os.path.join(OUT, "tray.png"))
make_tray(32).save(os.path.join(OUT, "tray@32.png"))
print("icons written to", OUT)

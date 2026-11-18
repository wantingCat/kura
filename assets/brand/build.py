"""Generate Kura's Tokyo Night pure-kanji (蔵) logo & web brand assets.

Outputs:
  - assets/brand/logo.png   (1024x1024 master squircle icon — used by README & `tauri icon`)
  - static/brand/logo.png   (512x512 app/UI icon — Onboarding, Settings → About)
  - static/favicon.png      (128x128 browser/webview favicon)
  - static/brand/kanji-white.png / kanji.png / cat.png
"""
import math
import os
from PIL import Image, ImageChops, ImageDraw, ImageFilter

root = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
src_dir = os.path.join(root, "assets", "brand")
out_dir = os.path.join(root, "static", "brand")
os.makedirs(src_dir, exist_ok=True)
os.makedirs(out_dir, exist_ok=True)


def trim(im: Image.Image, pad: int = 0) -> Image.Image:
    a = im.getchannel("A").point(lambda v: 255 if v > 8 else 0)
    bbox = a.getbbox()
    if not bbox:
        return im
    l, t, r, b = bbox
    return im.crop((max(0, l - pad), max(0, t - pad), min(im.width, r + pad), min(im.height, b + pad)))


def fit_w(im: Image.Image, w: int) -> Image.Image:
    h = round(im.height * w / im.width)
    return im.resize((w, h), Image.LANCZOS)


def fit_h(im: Image.Image, h: int) -> Image.Image:
    w = round(im.width * h / im.height)
    return im.resize((w, h), Image.LANCZOS)


def load(name: str) -> Image.Image:
    return Image.open(os.path.join(src_dir, name)).convert("RGBA")


def build_tokyo_night_logo(kanji_white: Image.Image, size: int = 1024) -> Image.Image:
    """Render a pure-kanji (蔵) Tokyo Night squircle icon."""
    # Build smooth gradient + radial glow at 512px then upscale background cleanly
    gsize = 512
    bg = Image.new("RGBA", (gsize, gsize), (0, 0, 0, 255))
    px = bg.load()

    for y in range(gsize):
        ny = y / (gsize - 1)
        for x in range(gsize):
            nx = x / (gsize - 1)
            # Base deep indigo-black diagonal gradient (#131628 -> #090B12)
            t = 0.55 * nx + 0.45 * ny
            r = 19 * (1 - t) + 9 * t
            g = 22 * (1 - t) + 11 * t
            b = 40 * (1 - t) + 18 * t

            # Top-left electric violet bloom (#8B5CF6)
            d1 = math.hypot(nx - 0.24, ny - 0.20)
            w1 = max(0.0, 1.0 - d1 / 0.78) ** 2.1 * 0.46
            r += (139 - r) * w1
            g += (92 - g) * w1
            b += (246 - b) * w1

            # Subtle bottom-right neon pink edge (#EC4899)
            d2 = math.hypot(nx - 0.82, ny - 0.84)
            w2 = max(0.0, 1.0 - d2 / 0.68) ** 2.3 * 0.24
            r += (236 - r) * w2
            g += (72 - g) * w2
            b += (153 - b) * w2

            px[x, y] = (int(min(255, max(0, r))), int(min(255, max(0, g))), int(min(255, max(0, b))), 255)

    canvas = bg.resize((size, size), Image.BICUBIC)

    # Prepare crisp pure-white kanji centered optically (~60% of icon width)
    glyph = trim(kanji_white, 0)
    target_w = int(size * 0.58)
    glyph = fit_w(glyph, target_w)
    gx = (size - glyph.width) // 2
    gy = (size - glyph.height) // 2 - int(size * 0.008)

    alpha = glyph.getchannel("A")

    # Layer 1: Wide ambient electric-violet halo behind 蔵
    glow_wide = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    wide_glyph = Image.merge(
        "RGBA",
        (
            Image.new("L", glyph.size, 139),
            Image.new("L", glyph.size, 92),
            Image.new("L", glyph.size, 246),
            alpha.point(lambda a: int(a * 0.62)),
        ),
    )
    glow_wide.paste(wide_glyph, (gx, gy))
    glow_wide = glow_wide.filter(ImageFilter.GaussianBlur(radius=size * 0.048))
    canvas = Image.alpha_composite(canvas, glow_wide)

    # Layer 2: Tight neon violet-pink core bloom
    glow_tight = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    tight_glyph = Image.merge(
        "RGBA",
        (
            Image.new("L", glyph.size, 216),
            Image.new("L", glyph.size, 114),
            Image.new("L", glyph.size, 246),
            alpha.point(lambda a: int(a * 0.42)),
        ),
    )
    glow_tight.paste(tight_glyph, (gx, gy))
    glow_tight = glow_tight.filter(ImageFilter.GaussianBlur(radius=size * 0.015))
    canvas = Image.alpha_composite(canvas, glow_tight)

    # Layer 3: Crisp off-white (#F8F7FF) 蔵 glyph
    crisp = Image.merge(
        "RGBA",
        (
            Image.new("L", glyph.size, 248),
            Image.new("L", glyph.size, 247),
            Image.new("L", glyph.size, 255),
            alpha,
        ),
    )
    fg_layer = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    fg_layer.paste(crisp, (gx, gy))
    canvas = Image.alpha_composite(canvas, fg_layer)

    # Supersampled rounded-squircle mask + subtle luminous glass rim
    ss = 4
    pad = int(size * 0.03)  # slight inset so taskbar icons have clean breathing room
    radius = int(size * 0.225)

    mask_hi = Image.new("L", (size * ss, size * ss), 0)
    ImageDraw.Draw(mask_hi).rounded_rectangle(
        (pad * ss, pad * ss, (size - pad) * ss - 1, (size - pad) * ss - 1),
        radius=radius * ss,
        fill=255,
    )
    mask = mask_hi.resize((size, size), Image.LANCZOS)

    # Inner rim highlight flush to the squircle edge (thin glass hairline, no gap)
    rim_hi = Image.new("RGBA", (size * ss, size * ss), (0, 0, 0, 0))
    rim_w = max(2, int(size * 0.009)) * ss
    ImageDraw.Draw(rim_hi).rounded_rectangle(
        (pad * ss, pad * ss, (size - pad) * ss - 1, (size - pad) * ss - 1),
        radius=radius * ss,
        outline=(196, 170, 255, 105),
        width=rim_w,
    )
    rim = rim_hi.resize((size, size), Image.LANCZOS)
    canvas = Image.alpha_composite(canvas, rim)

    out = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    out.paste(canvas, (0, 0), mask)
    return out


# Keep auxiliary assets updated
if os.path.exists(os.path.join(src_dir, "cat.png")):
    fit_w(trim(load("cat.png"), 4), 640).save(os.path.join(out_dir, "cat.png"), optimize=True)
kanji_white = load("kura-kanji-white.png")
fit_h(trim(kanji_white, 2), 256).save(os.path.join(out_dir, "kanji-white.png"), optimize=True)
if os.path.exists(os.path.join(src_dir, "kura-kanji.png")):
    fit_h(trim(load("kura-kanji.png"), 2), 256).save(os.path.join(out_dir, "kanji.png"), optimize=True)

# Master 1024x1024 pure-kanji Tokyo Night icon
master_logo = build_tokyo_night_logo(kanji_white, 1024)
master_logo.save(os.path.join(src_dir, "logo.png"), optimize=True)
master_logo.resize((512, 512), Image.LANCZOS).save(os.path.join(out_dir, "logo.png"), optimize=True)
master_logo.resize((128, 128), Image.LANCZOS).save(os.path.join(root, "static", "favicon.png"), optimize=True)

for label, p in [
    ("assets/brand/logo.png", os.path.join(src_dir, "logo.png")),
    ("static/brand/logo.png", os.path.join(out_dir, "logo.png")),
    ("static/favicon.png", os.path.join(root, "static", "favicon.png")),
]:
    im = Image.open(p)
    print(label, im.size, os.path.getsize(p) // 1024, "KB")

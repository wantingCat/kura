"""Generate Windows installer artwork (NSIS + WiX) in Tokyo Night pure-kanji style.

Run:  python assets/brand/installer.py
Outputs BMPs into src-tauri/installer/.
"""
import os
from PIL import Image, ImageFilter

root = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
src = os.path.join(root, "assets", "brand")
out = os.path.join(root, "src-tauri", "installer")
os.makedirs(out, exist_ok=True)

INDIGO = (11, 13, 20)
SURFACE = (18, 20, 31)
WHITE = (255, 255, 255)
SS = 4  # supersample factor for crisp downscaling


def load(name):
    im = Image.open(os.path.join(src, name)).convert("RGBA")
    return im.crop(im.getchannel("A").point(lambda v: 255 if v > 8 else 0).getbbox())


def fit(im, w=None, h=None):
    if w and not h:
        h = round(im.height * w / im.width)
    if h and not w:
        w = round(im.width * h / im.height)
    return im.resize((w, h), Image.LANCZOS)


kanji = load("kura-kanji-white.png")


def canvas(w, h, color):
    return Image.new("RGBA", (w * SS, h * SS), color + (255,))


def paste_glow_kanji(panel, k, x, y):
    alpha = k.getchannel("A")
    glow_layer = Image.new("RGBA", panel.size, (0, 0, 0, 0))
    violet = Image.merge(
        "RGBA",
        (
            Image.new("L", k.size, 139),
            Image.new("L", k.size, 92),
            Image.new("L", k.size, 246),
            alpha.point(lambda a: int(a * 0.7)),
        ),
    )
    glow_layer.paste(violet, (x, y))
    glow_layer = glow_layer.filter(ImageFilter.GaussianBlur(radius=max(4, k.width * 0.12)))
    panel.alpha_composite(glow_layer, (0, 0))
    panel.alpha_composite(k, (x, y))


def save(img, w, h, name):
    img.resize((w, h), Image.LANCZOS).convert("RGB").save(os.path.join(out, name), "BMP")
    print("wrote", name, (w, h))


def sidebar(w, h, art_w=None):
    """Tokyo Night indigo panel with glowing pure 蔵 kanji mark."""
    art_w = art_w or w
    img = canvas(w, h, WHITE)
    panel = canvas(art_w, h, INDIGO)
    k = fit(kanji, w=int(art_w * SS * 0.56))
    x = (panel.width - k.width) // 2
    y = int(panel.height * 0.36) - k.height // 2
    paste_glow_kanji(panel, k, x, y)
    img.alpha_composite(panel, (0, 0))
    return img


# NSIS welcome/finish sidebar: 164x314
save(sidebar(164, 314), 164, 314, "nsis-sidebar.bmp")

# NSIS header (top-right of inner pages): 150x57 — pure kanji centered on dark indigo
hdr = canvas(150, 57, SURFACE)
k = fit(kanji, h=int(57 * SS * 0.66))
paste_glow_kanji(hdr, k, (hdr.width - k.width) // 2, (hdr.height - k.height) // 2)
save(hdr, 150, 57, "nsis-header.bmp")

# WiX dialog background: 493x312, art occupies the left 164px, rest stays white for installer text
save(sidebar(493, 312, art_w=164), 493, 312, "wix-dialog.bmp")

# WiX top banner: 493x58, white with a dark Tokyo Night tab on the right holding the kanji
ban = canvas(493, 58, WHITE)
tab_w = 86 * SS
tab = Image.new("RGBA", (tab_w, ban.height), INDIGO + (255,))
k = fit(kanji, h=int(58 * SS * 0.66))
paste_glow_kanji(tab, k, (tab_w - k.width) // 2, (ban.height - k.height) // 2)
ban.alpha_composite(tab, (ban.width - tab_w, 0))
save(ban, 493, 58, "wix-banner.bmp")

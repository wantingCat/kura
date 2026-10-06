"""Generate Windows installer artwork (NSIS + WiX) from the brand sources.

Run:  python assets/brand/installer.py
Outputs BMPs into src-tauri/installer/.
"""
import os
from PIL import Image

root = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
src = os.path.join(root, "assets", "brand")
out = os.path.join(root, "src-tauri", "installer")
os.makedirs(out, exist_ok=True)

CORAL = (247, 109, 77)
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


kanji = load("kura-kanji.png")
word = load("kura-text.png")
cat = load("cat.png")


def canvas(w, h, color):
    return Image.new("RGBA", (w * SS, h * SS), color + (255,))


def paste_center_x(base, im, y, x_center=None):
    x = (x_center if x_center is not None else base.width // 2) - im.width // 2
    base.alpha_composite(im, (x, y))


def save(img, w, h, name):
    img.resize((w, h), Image.LANCZOS).convert("RGB").save(os.path.join(out, name), "BMP")
    print("wrote", name, (w, h))


def sidebar(w, h, art_w=None):
    """Coral panel: kanji + wordmark, with the cat peeking up from the bottom edge."""
    art_w = art_w or w
    img = canvas(w, h, WHITE)
    panel = canvas(art_w, h, CORAL)
    k = fit(kanji, w=int(art_w * SS * 0.62))
    paste_center_x(panel, k, int(h * SS * 0.16))
    wm = fit(word, w=int(art_w * SS * 0.34))
    paste_center_x(panel, wm, int(h * SS * 0.16) + k.height + int(10 * SS))
    c = fit(cat, w=int(art_w * SS * 0.78))
    panel.alpha_composite(c, ((panel.width - c.width) // 2, panel.height - int(c.height * 0.72)))
    img.alpha_composite(panel, (0, 0))
    return img


# NSIS welcome/finish sidebar: 164x314
save(sidebar(164, 314), 164, 314, "nsis-sidebar.bmp")

# NSIS header (top-right of inner pages): 150x57
hdr = canvas(150, 57, CORAL)
k = fit(kanji, h=int(57 * SS * 0.7))
wm = fit(word, h=int(57 * SS * 0.28))
gap = 8 * SS
total = k.width + gap + wm.width
x0 = (hdr.width - total) // 2
hdr.alpha_composite(k, (x0, (hdr.height - k.height) // 2))
hdr.alpha_composite(wm, (x0 + k.width + gap, (hdr.height - wm.height) // 2))
save(hdr, 150, 57, "nsis-header.bmp")

# WiX dialog background: 493x312, art occupies the left 164px, rest stays white for text
save(sidebar(493, 312, art_w=164), 493, 312, "wix-dialog.bmp")

# WiX top banner: 493x58, white with a coral tab on the right holding the kanji
ban = canvas(493, 58, WHITE)
tab_w = 86 * SS
tab = Image.new("RGBA", (tab_w, ban.height), CORAL + (255,))
k = fit(kanji, h=int(58 * SS * 0.7))
tab.alpha_composite(k, ((tab_w - k.width) // 2, (ban.height - k.height) // 2))
ban.alpha_composite(tab, (ban.width - tab_w, 0))
save(ban, 493, 58, "wix-banner.bmp")

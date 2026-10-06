"""Move brand sources into assets/brand and emit trimmed, web-sized copies into static/brand."""
import os, shutil
from PIL import Image

root = r"c:\Users\acer\Documents\code\kura"
src_dir = os.path.join(root, "assets", "brand")
out_dir = os.path.join(root, "static", "brand")
os.makedirs(src_dir, exist_ok=True)
os.makedirs(out_dir, exist_ok=True)

names = ["logo.png", "cat.png", "kura-kanji.png", "kura-kanji-white.png", "kura-text.png", "kura-text-white.png"]
for n in names:
    p = os.path.join(root, n)
    if os.path.exists(p):
        shutil.move(p, os.path.join(src_dir, n))

def trim(im, pad=0):
    a = im.getchannel("A").point(lambda v: 255 if v > 8 else 0)
    l, t, r, b = a.getbbox()
    return im.crop((max(0, l - pad), max(0, t - pad), min(im.width, r + pad), min(im.height, b + pad)))

def fit_w(im, w):
    h = round(im.height * w / im.width)
    return im.resize((w, h), Image.LANCZOS)

def fit_h(im, h):
    w = round(im.width * h / im.height)
    return im.resize((w, h), Image.LANCZOS)

def load(n):
    return Image.open(os.path.join(src_dir, n)).convert("RGBA")

fit_w(trim(load("cat.png"), 4), 640).save(os.path.join(out_dir, "cat.png"), optimize=True)
fit_h(trim(load("kura-kanji-white.png"), 2), 256).save(os.path.join(out_dir, "kanji-white.png"), optimize=True)
fit_h(trim(load("kura-kanji.png"), 2), 256).save(os.path.join(out_dir, "kanji.png"), optimize=True)
fit_h(trim(load("kura-text-white.png"), 2), 120).save(os.path.join(out_dir, "wordmark-white.png"), optimize=True)
fit_h(trim(load("kura-text.png"), 2), 120).save(os.path.join(out_dir, "wordmark.png"), optimize=True)

logo = load("logo.png")
logo.resize((512, 512), Image.LANCZOS).save(os.path.join(out_dir, "logo.png"), optimize=True)
# Rounded-square favicon/app mark
fav = logo.resize((128, 128), Image.LANCZOS)
mask = Image.new("L", (128 * 4, 128 * 4), 0)
from PIL import ImageDraw
ImageDraw.Draw(mask).rounded_rectangle((0, 0, 128 * 4 - 1, 128 * 4 - 1), radius=28 * 4, fill=255)
fav.putalpha(mask.resize((128, 128), Image.LANCZOS))
fav.save(os.path.join(root, "static", "favicon.png"), optimize=True)

for f in sorted(os.listdir(out_dir)):
    im = Image.open(os.path.join(out_dir, f))
    print(f, im.size, os.path.getsize(os.path.join(out_dir, f)) // 1024, "KB")

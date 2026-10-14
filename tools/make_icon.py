"""Builds the VigilClip icon set.

Run it from the repository root:

    python tools/make_icon.py

Everything is drawn from one 1024 px master and then resampled down, which is what keeps
the small sizes from looking like a different icon. The mark is deliberately simple: a
rounded gradient square with three stacked cards on it, because the icon has to still say
something at 16 px in a taskbar.
"""

from __future__ import annotations

import os
from PIL import Image, ImageDraw, ImageFilter

MASTER = 1024
OUT = os.path.join("src-tauri", "icons")

# Deep indigo, so the light cards have something to sit on.
BACKDROP_TOP = (27, 17, 70)
BACKDROP_BOTTOM = (60, 36, 150)

# Card gradients, back to front. Front-most is the largest, and reads as the clip. The
# stack keeps a wide margin on every side, because an icon that fills its square turns to
# mush in a taskbar at 16 px.
CARDS = [
    # (centre x, centre y, width, height, top colour, bottom colour)
    (0.585, 0.395, 0.335, 0.420, (169, 139, 255), (124, 92, 255)),
    (0.512, 0.485, 0.365, 0.450, (111, 208, 255), (79, 140, 255)),
    (0.440, 0.575, 0.400, 0.480, (124, 240, 216), (42, 211, 184)),
]

ICON_SIZES = {
    "32x32.png": 32,
    "64x64.png": 64,
    "128x128.png": 128,
    "128x128@2x.png": 256,
    "StoreLogo.png": 50,
    "Square30x30Logo.png": 30,
    "Square44x44Logo.png": 44,
    "Square71x71Logo.png": 71,
    "Square89x89Logo.png": 89,
    "Square107x107Logo.png": 107,
    "Square142x142Logo.png": 142,
    "Square150x150Logo.png": 150,
    "Square284x284Logo.png": 284,
    "Square310x310Logo.png": 310,
}

# Windows picks whichever of these fits what it is drawing, so all of them are included.
ICO_SIZES = [(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)]


def vertical_gradient(size: int, top: tuple, bottom: tuple) -> Image.Image:
    """A `size` x `size` image going from `top` to `bottom`."""
    gradient = Image.new("RGBA", (1, size))
    for y in range(size):
        ratio = y / max(size - 1, 1)
        gradient.putpixel(
            (0, y),
            tuple(round(top[i] + (bottom[i] - top[i]) * ratio) for i in range(3)) + (255,),
        )
    return gradient.resize((size, size), Image.Resampling.NEAREST)


def rounded_mask(size: tuple, radius: float) -> Image.Image:
    """A mask the size of `size`, white inside a rounded rectangle and transparent outside."""
    width, height = size
    mask = Image.new("L", (width, height), 0)
    ImageDraw.Draw(mask).rounded_rectangle(
        (0, 0, width - 1, height - 1), radius=radius, fill=255
    )
    return mask


def draw_cards(target: Image.Image) -> None:
    """Paints the three cards onto `target`, back to front."""
    for cx, cy, w, h, top, bottom in CARDS:
        width = int(MASTER * w)
        height = int(MASTER * h)
        left = int(MASTER * cx - width / 2)
        top_y = int(MASTER * cy - height / 2)
        box = (left, top_y, left + width, top_y + height)
        radius = int(min(width, height) * 0.16)

        # A soft shadow under each card is what makes the stack read as a stack rather
        # than as three flat shapes.
        shadow = Image.new("RGBA", (MASTER, MASTER), (0, 0, 0, 0))
        ImageDraw.Draw(shadow).rounded_rectangle(
            (box[0] + 6, box[1] + 18, box[2] + 6, box[3] + 18),
            radius=radius,
            fill=(10, 6, 40, 130),
        )
        target.alpha_composite(shadow.filter(ImageFilter.GaussianBlur(18)))

        gradient = vertical_gradient(MASTER, top, bottom).crop(box)
        target.paste(gradient, box, rounded_mask((width, height), radius))

        # A light stroke keeps the card edges visible where two gradients meet.
        ImageDraw.Draw(target).rounded_rectangle(
            box, radius=radius, outline=(255, 255, 255, 90), width=4
        )


def build_master() -> Image.Image:
    backdrop = Image.new("RGBA", (MASTER, MASTER), (0, 0, 0, 0))
    square = vertical_gradient(MASTER, BACKDROP_TOP, BACKDROP_BOTTOM)
    backdrop.paste(square, (0, 0), rounded_mask((MASTER, MASTER), MASTER * 0.22))
    draw_cards(backdrop)
    return backdrop


def build_tray(colour: tuple) -> Image.Image:
    """The mark as a single-colour silhouette, which is what a tray icon should be."""
    glyph = Image.new("RGBA", (MASTER, MASTER), (0, 0, 0, 0))
    for cx, cy, w, h, _top, _bottom in CARDS:
        width = int(MASTER * w)
        height = int(MASTER * h)
        left = int(MASTER * cx - width / 2)
        top_y = int(MASTER * cy - height / 2)
        ImageDraw.Draw(glyph).rounded_rectangle(
            (left, top_y, left + width, top_y + height),
            radius=int(min(width, height) * 0.16),
            fill=colour,
        )
    return glyph


def main() -> None:
    master = build_master()

    for name, size in ICON_SIZES.items():
        master.resize((size, size), Image.Resampling.LANCZOS).save(os.path.join(OUT, name))
        print("wrote", name, size)

    master.resize((256, 256), Image.Resampling.LANCZOS).save(
        os.path.join(OUT, "icon.ico"), sizes=ICO_SIZES
    )
    print("wrote icon.ico", [size[0] for size in ICO_SIZES])

    # The tray has no background to sit on, so it gets the silhouette instead: white for
    # a dark taskbar, near-black for a light one.
    build_tray((255, 255, 255, 255)).resize((44, 44), Image.Resampling.LANCZOS).save(
        os.path.join(OUT, "tray_white.png")
    )
    build_tray((26, 22, 48, 255)).resize((44, 44), Image.Resampling.LANCZOS).save(
        os.path.join(OUT, "tray.png")
    )
    print("wrote tray.png and tray_white.png", 44)


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Generate Satelite icons: moon face 🌚 (app + monochrome tray).

  pip install pillow
  python3 scripts/generate-icons.py
"""
from __future__ import annotations

import shutil
import struct
import subprocess
import tempfile
from io import BytesIO
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter, ImageChops

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "src-tauri" / "icons"


def ell(cx, cy, rx, ry):
    return [cx - rx, cy - ry, cx + rx, cy + ry]


def draw_moon_face(
    size: int, *, mono=False, mono_color=(0, 0, 0, 255), tray=False
) -> Image.Image:
    hi = min(1024, max(256, size * 6))
    img = Image.new("RGBA", (hi, hi), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    cx = cy = hi / 2
    R = hi * (0.46 if tray else 0.44)

    if mono:
        feature_cut = True
    else:
        face = (42, 44, 52, 255)
        face_hi = (72, 74, 86, 255)
        feature = (18, 18, 22, 255)
        feature_cut = False
        bg = Image.new("RGBA", (hi, hi), (12, 12, 16, 255))
        glow = Image.new("RGBA", (hi, hi), (0, 0, 0, 0))
        ImageDraw.Draw(glow).ellipse(
            ell(cx, cy, R * 1.15, R * 1.15), fill=(88, 100, 140, 40)
        )
        glow = glow.filter(ImageFilter.GaussianBlur(radius=hi * 0.08))
        img = Image.alpha_composite(bg, glow)
        d = ImageDraw.Draw(img)

    if not mono:
        d.ellipse(ell(cx, cy, R, R), fill=face)
        hi_layer = Image.new("RGBA", (hi, hi), (0, 0, 0, 0))
        ImageDraw.Draw(hi_layer).ellipse(
            ell(cx - R * 0.2, cy - R * 0.25, R * 0.55, R * 0.5),
            fill=(*face_hi[:3], 90),
        )
        hi_layer = hi_layer.filter(ImageFilter.GaussianBlur(radius=R * 0.25))
        img = Image.alpha_composite(img, hi_layer)
        d = ImageDraw.Draw(img)
        rim_w = max(2, int(hi * 0.012))
        d.ellipse(ell(cx, cy, R, R), outline=(140, 145, 165, 180), width=rim_w)
        for ox, oy, cr in [(-0.35, -0.2, 0.08), (0.3, 0.15, 0.06), (-0.15, 0.35, 0.05)]:
            d.ellipse(
                ell(cx + R * ox, cy + R * oy, R * cr, R * cr),
                outline=(30, 32, 38, 100),
                width=max(1, int(hi * 0.004)),
            )
    else:
        d.ellipse(ell(cx, cy, R, R), fill=mono_color)

    eye_y = cy - R * 0.12
    eye_dx = R * 0.28
    eye_rx = R * (0.11 if tray else 0.10)
    eye_ry = R * (0.13 if tray else 0.12)

    if mono and feature_cut:
        feat = Image.new("L", (hi, hi), 0)
        fd = ImageDraw.Draw(feat)
        fd.ellipse(ell(cx - eye_dx, eye_y, eye_rx, eye_ry), fill=255)
        fd.ellipse(ell(cx + eye_dx, eye_y, eye_rx, eye_ry), fill=255)
        mouth_r = R * 0.38
        mouth_cy = cy + R * 0.18
        fd.pieslice(
            ell(cx, mouth_cy - mouth_r * 0.15, mouth_r, mouth_r * 0.85),
            start=15,
            end=165,
            fill=255,
        )
        cut_r = mouth_r * 0.72
        fd.ellipse(ell(cx, mouth_cy - mouth_r * 0.35, cut_r, cut_r * 0.75), fill=0)
        feat = feat.filter(ImageFilter.GaussianBlur(radius=max(1, hi * 0.004)))
        feat = feat.point(lambda p: 255 if p > 80 else 0)
        r, g, b, a = img.split()
        inv = feat.point(lambda p: 0 if p > 128 else 255)
        a = ImageChops.multiply(a, inv)
        solid = Image.new("RGBA", (hi, hi), mono_color)
        out = Image.new("RGBA", (hi, hi), (0, 0, 0, 0))
        out.paste(solid, (0, 0), a)
        img = out
    else:
        d.ellipse(ell(cx - eye_dx, eye_y, eye_rx, eye_ry), fill=feature)
        d.ellipse(ell(cx + eye_dx, eye_y, eye_rx, eye_ry), fill=feature)
        hl = R * 0.03
        d.ellipse(
            ell(cx - eye_dx - eye_rx * 0.2, eye_y - eye_ry * 0.25, hl, hl),
            fill=(200, 200, 210, 160),
        )
        d.ellipse(
            ell(cx + eye_dx - eye_rx * 0.2, eye_y - eye_ry * 0.25, hl, hl),
            fill=(200, 200, 210, 160),
        )
        mouth_r = R * 0.36
        mouth_cy = cy + R * 0.22
        mouth_layer = Image.new("RGBA", (hi, hi), (0, 0, 0, 0))
        md = ImageDraw.Draw(mouth_layer)
        width = max(3, int(R * 0.09))
        md.arc(
            ell(cx, mouth_cy - mouth_r * 0.2, mouth_r, mouth_r * 0.9),
            start=20,
            end=160,
            fill=feature,
            width=width,
        )
        img = Image.alpha_composite(img, mouth_layer)
        blush = Image.new("RGBA", (hi, hi), (0, 0, 0, 0))
        bd = ImageDraw.Draw(blush)
        br = R * 0.1
        bd.ellipse(
            ell(cx - R * 0.38, cy + R * 0.08, br, br * 0.7),
            fill=(180, 100, 120, 50),
        )
        bd.ellipse(
            ell(cx + R * 0.38, cy + R * 0.08, br, br * 0.7),
            fill=(180, 100, 120, 50),
        )
        blush = blush.filter(ImageFilter.GaussianBlur(radius=R * 0.08))
        img = Image.alpha_composite(img, blush)

    return img.resize((size, size), Image.Resampling.LANCZOS)


def rounded_mask(size: int) -> Image.Image:
    m = Image.new("L", (size, size), 0)
    ImageDraw.Draw(m).rounded_rectangle(
        [0, 0, size - 1, size - 1], radius=int(size * 0.223), fill=255
    )
    return m


def make_app_icon(size: int) -> Image.Image:
    im = draw_moon_face(size, mono=False, tray=False)
    r, g, b, a = im.split()
    a = ImageChops.multiply(a, rounded_mask(size))
    im = Image.merge("RGBA", (r, g, b, a))
    border = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    ins = max(1, size // 100)
    ImageDraw.Draw(border).rounded_rectangle(
        [ins, ins, size - 1 - ins, size - 1 - ins],
        radius=int(size * 0.223),
        outline=(60, 62, 72, 180),
        width=max(1, size // 120),
    )
    im = Image.alpha_composite(im, border)
    r, g, b, a = im.split()
    a = ImageChops.multiply(a, rounded_mask(size))
    return Image.merge("RGBA", (r, g, b, a))


def make_tray(size: int, color: tuple[int, int, int, int]) -> Image.Image:
    return draw_moon_face(size, mono=True, mono_color=color, tray=True)


def write_ico(path: Path) -> None:
    sizes = [16, 24, 32, 48, 64, 128, 256]
    entries, blobs = [], []
    for s in sizes:
        buf = BytesIO()
        make_app_icon(s).save(buf, format="PNG")
        data = buf.getvalue()
        entries.append((s, len(data)))
        blobs.append(data)
    offset = 6 + 16 * len(sizes)
    header = struct.pack("<HHH", 0, 1, len(sizes))
    dire = body = b""
    for (s, sz), data in zip(entries, blobs):
        w = h = 0 if s >= 256 else s
        dire += struct.pack("<BBBBHHII", w, h, 0, 0, 1, 32, sz, offset)
        body += data
        offset += sz
    path.write_bytes(header + dire + body)


def write_icns() -> None:
    iconset = Path(tempfile.mkdtemp(suffix=".iconset"))
    try:
        for fname, s in [
            ("icon_16x16.png", 16),
            ("icon_16x16@2x.png", 32),
            ("icon_32x32.png", 32),
            ("icon_32x32@2x.png", 64),
            ("icon_128x128.png", 128),
            ("icon_128x128@2x.png", 256),
            ("icon_256x256.png", 256),
            ("icon_256x256@2x.png", 512),
            ("icon_512x512.png", 512),
            ("icon_512x512@2x.png", 1024),
        ]:
            make_app_icon(s).save(iconset / fname, format="PNG")
        subprocess.check_call(
            ["iconutil", "-c", "icns", str(iconset), "-o", str(OUT / "icon.icns")]
        )
    finally:
        shutil.rmtree(iconset, ignore_errors=True)


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    make_app_icon(1024).save(OUT / "icon.png", format="PNG")
    for name, sz in [
        ("32x32.png", 32),
        ("128x128.png", 128),
        ("128x128@2x.png", 256),
        ("Square30x30Logo.png", 30),
        ("Square44x44Logo.png", 44),
        ("Square71x71Logo.png", 71),
        ("Square89x89Logo.png", 89),
        ("Square107x107Logo.png", 107),
        ("Square142x142Logo.png", 142),
        ("Square150x150Logo.png", 150),
        ("Square284x284Logo.png", 284),
        ("Square310x310Logo.png", 310),
        ("StoreLogo.png", 50),
    ]:
        make_app_icon(sz).save(OUT / name, format="PNG")

    for size, white, black in [
        (64, "tray-icon.png", "tray-icon-template.png"),
        (32, "tray-icon-32.png", "tray-icon-template-32.png"),
        (22, "tray-icon-22.png", "tray-icon-template-22.png"),
    ]:
        make_tray(size, (255, 255, 255, 255)).save(OUT / white, format="PNG")
        make_tray(size, (0, 0, 0, 255)).save(OUT / black, format="PNG")

    write_ico(OUT / "icon.ico")
    write_icns()
    print(f"Moon-face icons written → {OUT}")


if __name__ == "__main__":
    main()

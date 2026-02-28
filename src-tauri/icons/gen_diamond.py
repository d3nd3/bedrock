#!/usr/bin/env python3
"""Generate ⯁ (black medium diamond) source for app icon. Run then: cargo tauri icon src-tauri/icons/icon.png -o src-tauri/icons"""
from pathlib import Path
from PIL import Image, ImageDraw

def diamond(size: int, margin: float = 0.12) -> Image.Image:
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    m = int(size * margin)
    ImageDraw.Draw(img).polygon(
        [(size // 2, m), (size - m, size // 2), (size // 2, size - m), (m, size // 2)],
        fill=(0, 0, 0, 255),
    )
    return img

diamond(512).save(Path(__file__).parent / "icon.png")

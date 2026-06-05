import json
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parents[1]
DATA = json.loads((ROOT / "screenshot-data.json").read_text(encoding="utf-8"))
OUT = ROOT / "screenshots"
OUT.mkdir(exist_ok=True)

W, H = 1280, 800
BG = (5, 7, 13)
PANEL = (8, 12, 24)
CARD = (11, 16, 32)
BORDER = (37, 48, 77)
TEXT = (215, 226, 255)
MUTED = (127, 143, 175)
CYAN = (125, 227, 255)
GREEN = (145, 246, 168)
PINK = (255, 139, 209)
YELLOW = (255, 209, 102)
BLUE = (138, 180, 255)
WHITE = (255, 255, 255)


def font(size, bold=False):
    candidates = [
        "C:/Windows/Fonts/CascadiaMono.ttf",
        "C:/Windows/Fonts/consola.ttf",
        "C:/Windows/Fonts/Consola.ttf",
        "C:/Windows/Fonts/arial.ttf",
    ]
    for c in candidates:
        if Path(c).exists():
            return ImageFont.truetype(c, size=size)
    return ImageFont.load_default()

F12, F14, F15, F16, F18, F22, F30 = [font(s) for s in (12, 14, 15, 16, 18, 22, 30)]


def fmt(n):
    return f"{int(n):,}"


def rounded(draw, xy, fill, outline=BORDER, radius=18, width=1):
    draw.rounded_rectangle(xy, radius=radius, fill=fill, outline=outline, width=width)


def base(title):
    img = Image.new("RGB", (W, H), BG)
    d = ImageDraw.Draw(img)
    # simple radial-ish accent with translucent circles
    for r, color in [(420, (26, 42, 85)), (260, (16, 32, 70)), (120, (28, 54, 115))]:
        overlay = Image.new("RGBA", (W, H), (0, 0, 0, 0))
        od = ImageDraw.Draw(overlay)
        od.ellipse((80-r, -180-r, 80+r, -180+r), fill=(*color, 90))
        img.alpha_composite(overlay) if img.mode == "RGBA" else None
    d = ImageDraw.Draw(img)
    rounded(d, (34, 34, W-34, H-34), PANEL, BORDER, 18, 1)
    d.rounded_rectangle((34, 34, W-34, 76), radius=18, fill=(13, 19, 36), outline=BORDER)
    d.rectangle((34, 56, W-34, 76), fill=(13, 19, 36))
    for i, c in enumerate([(255, 95, 87), (255, 189, 46), (40, 200, 64)]):
        d.ellipse((52+i*22, 50, 64+i*22, 62), fill=c)
    d.text((126, 47), title, fill=(142, 160, 201), font=F15)
    return img, d


def stat(d, x, y, label, value):
    rounded(d, (x, y, x+280, y+80), (13, 22, 44), (38, 53, 85), 12, 1)
    d.text((x+14, y+12), label, fill=MUTED, font=F12)
    d.text((x+14, y+34), value, fill=WHITE, font=F22)


def bar_line(d, x, y, label, value, max_value, color=CYAN, width=360):
    d.text((x, y), label, fill=BLUE, font=F15)
    bx = x + 122
    by = y + 3
    d.rounded_rectangle((bx, by, bx+width, by+16), radius=7, fill=(21, 29, 52))
    fillw = max(2, int(width * value / max_value)) if max_value else 0
    d.rounded_rectangle((bx, by, bx+fillw, by+16), radius=7, fill=color)
    d.text((bx+width+18, y), f"{fmt(value)} tokens", fill=TEXT, font=F15)


def overview():
    img, d = base("dexuse — TUI overview")
    total = DATA["total"]
    d.text((64, 102), "dexuse — OpenAI usage explorer", fill=WHITE, font=F30)
    d.text((64, 140), "Codex + Hermes Desktop/CLI • local-only token analytics • day drilldown", fill=MUTED, font=F15)
    for i, (label, value) in enumerate([
        ("Total tokens", fmt(total["total_tokens"])),
        ("Cached input", fmt(total["cached_input_tokens"])),
        ("Output tokens", fmt(total["output_tokens"])),
        ("API calls", fmt(total["api_calls"])),
    ]):
        stat(d, 64 + i*296, 178, label, value)
    tabs = ["Timeline", "Models", "Sources", "JSON"]
    tx = 64
    for i, t in enumerate(tabs):
        fill = (27, 44, 85) if i == 0 else PANEL
        rounded(d, (tx, 288, tx+96, 324), fill, (79, 117, 199) if i == 0 else (49, 65, 95), 8, 1)
        d.text((tx+12, 297), t, fill=WHITE if i == 0 else (142, 160, 201), font=F14)
        tx += 108
    rounded(d, (64, 348, 790, 705), CARD, BORDER, 14, 1)
    d.text((84, 368), "Usage by day", fill=WHITE, font=F18)
    maxb = max(b["usage"]["total_tokens"] for b in DATA["buckets"])
    y = 408
    for b in DATA["buckets"]:
        bar_line(d, 84, y, b["key"], b["usage"]["total_tokens"], maxb, CYAN, 390)
        y += 42
    rounded(d, (818, 348, 1216, 705), CARD, BORDER, 14, 1)
    d.text((838, 368), "Shortcuts", fill=WHITE, font=F18)
    for i, line in enumerate(["[y] year    [m] month", "[w] week    [d] day", "[←]/[→] tabs [q] quit"]):
        d.text((838, 410+i*32), line, fill=TEXT, font=F16)
    d.text((838, 536), "Sources", fill=WHITE, font=F18)
    y = 580
    for k, v in DATA["by_source"].items():
        d.text((838, y), k, fill=GREEN, font=F16)
        d.text((960, y), fmt(v["total_tokens"]), fill=TEXT, font=F16)
        y += 34
    img.save(OUT / "dexuse-overview.png")


def models():
    img, d = base("dexuse — model breakdown")
    total = DATA["total"]
    d.text((64, 102), "Model + provider breakdown", fill=WHITE, font=F30)
    d.text((64, 140), "A single Codex session can include multiple models; token events are attributed to the active model.", fill=MUTED, font=F15)
    rounded(d, (64, 188, 790, 705), CARD, BORDER, 14, 1)
    d.text((84, 208), "Models", fill=WHITE, font=F18)
    maxm = max(v["total_tokens"] for v in DATA["by_model"].values())
    y = 260
    for name, v in sorted(DATA["by_model"].items(), key=lambda kv: kv[1]["total_tokens"], reverse=True):
        bar_line(d, 84, y, name, v["total_tokens"], maxm, PINK if name != "gpt-5.5" else CYAN, 410)
        y += 54
    rounded(d, (818, 188, 1216, 705), CARD, BORDER, 14, 1)
    d.text((838, 208), "Providers", fill=WHITE, font=F18)
    y = 252
    for k, v in DATA["by_provider"].items():
        d.text((838, y), k, fill=YELLOW, font=F16)
        d.text((838, y+28), f"{fmt(v['total_tokens'])} tokens • {fmt(v['api_calls'])} calls", fill=TEXT, font=F15)
        y += 74
    d.text((838, 410), "Token buckets", fill=WHITE, font=F18)
    y = 448
    for label, key in [("input", "input_tokens"), ("cached input", "cached_input_tokens"), ("cache writes", "cache_write_tokens"), ("output", "output_tokens"), ("reasoning", "reasoning_tokens")]:
        d.text((838, y), f"{label:<14} {fmt(total[key])}", fill=TEXT, font=F15)
        y += 32
    img.save(OUT / "dexuse-models.png")


def jsonshot():
    img, d = base("dexuse — JSON output")
    d.text((64, 102), "Machine-readable JSON mode", fill=WHITE, font=F30)
    d.text((64, 140), "npx @yashau/dexuse --json    --granularity day    --from / --to", fill=MUTED, font=F15)
    rounded(d, (64, 188, 1216, 705), CARD, BORDER, 14, 1)
    sample = json.dumps({
        "records": DATA["records"],
        "total": DATA["total"],
        "by_model": DATA["by_model"],
        "by_source": DATA["by_source"],
        "buckets": DATA["buckets"][:2],
    }, indent=2)
    y = 208
    for line in sample.splitlines()[:25]:
        color = CYAN if any(s in line for s in ['"total"', '"by_model"', '"buckets"']) else TEXT
        if ':' in line and line.strip().startswith('"'):
            color = BLUE
        d.text((84, y), line[:132], fill=color, font=F14)
        y += 20
    img.save(OUT / "dexuse-json.png")


overview(); models(); jsonshot()
for path in [OUT / "dexuse-overview.png", OUT / "dexuse-models.png", OUT / "dexuse-json.png"]:
    print(path)

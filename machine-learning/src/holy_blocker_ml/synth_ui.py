"""Synthetic UI/text screenshot generator for the benign evaluation corpus.

Fully synthetic: no captured screen content, no third-party images, nothing
that could be mistaken for private material. Every image is built from
primitive shapes and a small built-in word list, so the generator itself is
the only thing that ever needs review — the corpus it produces never needs
review and never enters the repo (see corpus.py, and the gitignored
`**/machine-learning/data` pattern).

This directly targets the coverage gap recorded in
docs/decisions/classifier-operating-point.md ("The screen path operates
outside this measurement"): a classifier calibrated on photographic imagery
has never been scored against rendered UI — flat colour fields, toolbars,
monospace text. This generator is a first, cheap pass at building that
missing corpus.
"""

from __future__ import annotations

import json
import random
from dataclasses import dataclass
from pathlib import Path
from typing import Callable

from PIL import Image, ImageDraw, ImageFont

# Written alongside the generated PNGs so a caller can tell whether a corpus
# directory already holds the exact (seed, count) it's about to ask for,
# rather than a stale run left over from an earlier invocation. Named so it
# never collides with `output_dir.glob("*.png")`.
MANIFEST_NAME = "manifest.json"

# Realistic screen/window resolutions this project's daemons actually see:
# common desktop resolutions, a MacBook's native logical resolution, and two
# phone/tablet sizes (the Android accessibility-overlay path).
RESOLUTIONS = [
    (1920, 1080),
    (1512, 982),  # MacBook Pro 14" logical resolution
    (1440, 900),
    (2560, 1440),
    (1280, 800),
    (390, 844),  # iPhone 14 portrait
    (1024, 1366),  # iPad portrait
]

_WORDS = (
    "the quick brown fox jumps over lazy dog while system status update "
    "config network request response error warning success pending queue "
    "user session token value index array object class function return "
    "import export const let variable string number boolean method field "
    "meeting notes project deadline release version build test deploy "
    "chart table row column header footer sidebar toolbar button label "
    "hello world thanks please confirm cancel submit save load open close"
).split()

# (background, foreground, accent) — kept as plain RGB tuples, no photo-like
# gradients or imagery, since the goal is "rendered UI", not "a stand-in
# photo".
_THEMES: list[tuple[tuple[int, int, int], tuple[int, int, int], tuple[int, int, int]]] = [
    ((255, 255, 255), (20, 20, 20), (0, 102, 204)),  # light document
    ((30, 30, 30), (220, 220, 220), (86, 156, 214)),  # dark editor
    ((246, 246, 248), (30, 30, 30), (52, 120, 246)),  # light chat
    ((0, 0, 0), (120, 255, 120), (255, 255, 255)),  # terminal
    ((245, 245, 245), (40, 40, 40), (230, 100, 40)),  # spreadsheet
]

SceneKind = str
SCENE_KINDS: tuple[SceneKind, ...] = (
    "code_editor",
    "chat",
    "document",
    "terminal",
    "spreadsheet",
    "form",
)


@dataclass(frozen=True)
class SyntheticImageSpec:
    """The parameters for one generated image, independent of rendering.

    Kept separate from `render_image` so scene/resolution/theme selection —
    the part worth unit-testing — never needs Pillow or a real font
    installed to exercise.
    """

    scene: SceneKind
    width: int
    height: int
    theme_index: int
    seed: int


def plan_corpus(count: int, seed: int) -> list[SyntheticImageSpec]:
    """Deterministically choose `count` scene/resolution/theme combinations.

    Pure and Pillow-free: the same (count, seed) always returns the same
    plan, independent of rendering, so the plan is unit-testable without
    drawing anything.
    """
    rng = random.Random(seed)
    specs = []
    for i in range(count):
        scene = rng.choice(SCENE_KINDS)
        width, height = rng.choice(RESOLUTIONS)
        theme_index = rng.randrange(len(_THEMES))
        # Per-image seed derived from (corpus seed, index) rather than
        # shared RNG state, so rendering a single spec later reproduces the
        # same pixels as it had inside a full-corpus run.
        specs.append(
            SyntheticImageSpec(
                scene=scene,
                width=width,
                height=height,
                theme_index=theme_index,
                seed=seed * 1_000_003 + i,
            )
        )
    return specs


def _load_font(size: int):
    for candidate in (
        "/System/Library/Fonts/Menlo.ttc",
        "/System/Library/Fonts/Supplemental/Arial.ttf",
        "/System/Library/Fonts/Helvetica.ttc",
    ):
        try:
            return ImageFont.truetype(candidate, size)
        except OSError:
            continue
    return ImageFont.load_default()


def _lorem_line(rng: random.Random, word_count: int) -> str:
    return " ".join(rng.choice(_WORDS) for _ in range(word_count))


def render_image(spec: SyntheticImageSpec) -> Image.Image:
    """Render one synthetic screenshot-like image from a spec."""
    rng = random.Random(spec.seed)
    bg, fg, accent = _THEMES[spec.theme_index]
    image = Image.new("RGB", (spec.width, spec.height), bg)
    draw = ImageDraw.Draw(image)
    _SCENE_RENDERERS[spec.scene](draw, spec.width, spec.height, fg, accent, rng)
    return image


def _render_code_editor(draw, w, h, fg, accent, rng) -> None:
    font = _load_font(14)
    gutter = int(w * 0.04)
    y = 10
    line_no = 1
    while y < h - 20:
        draw.text((6, y), str(line_no), font=font, fill=accent)
        indent = rng.choice([0, 1, 2, 3]) * 16
        draw.text(
            (gutter + indent, y), _lorem_line(rng, rng.randint(2, 8)), font=font, fill=fg
        )
        y += 20
        line_no += 1


def _render_chat(draw, w, h, fg, accent, rng) -> None:
    font = _load_font(15)
    y = 20
    while y < h - 60:
        bubble_w = rng.randint(int(w * 0.2), int(w * 0.5))
        from_me = rng.random() < 0.5
        x = w - bubble_w - 20 if from_me else 20
        bubble_h = 40
        color = accent if from_me else tuple(min(255, c + 30) for c in fg)
        draw.rounded_rectangle([x, y, x + bubble_w, y + bubble_h], radius=12, fill=color)
        draw.text(
            (x + 10, y + 12),
            _lorem_line(rng, rng.randint(2, 6)),
            font=font,
            fill=(255, 255, 255),
        )
        y += bubble_h + 16


def _render_document(draw, w, h, fg, accent, rng) -> None:
    title_font = _load_font(28)
    body_font = _load_font(16)
    draw.text(
        (int(w * 0.08), int(h * 0.06)),
        _lorem_line(rng, rng.randint(2, 5)).title(),
        font=title_font,
        fill=fg,
    )
    y = int(h * 0.15)
    margin = int(w * 0.08)
    while y < h - 30:
        draw.text((margin, y), _lorem_line(rng, rng.randint(6, 14)), font=body_font, fill=fg)
        y += 26


def _render_terminal(draw, w, h, fg, accent, rng) -> None:
    font = _load_font(14)
    y = 10
    while y < h - 20:
        prompt = rng.choice(["$ ", "> ", "# "])
        draw.text((10, y), prompt + _lorem_line(rng, rng.randint(1, 6)), font=font, fill=fg)
        y += 18


def _render_spreadsheet(draw, w, h, fg, accent, rng) -> None:
    font = _load_font(13)
    col_w = max(60, w // 10)
    row_h = 24
    grid_color = tuple(min(255, c + 20) for c in fg)
    for gy in range(0, h, row_h):
        draw.line([(0, gy), (w, gy)], fill=grid_color)
    for gx in range(0, w, col_w):
        draw.line([(gx, 0), (gx, h)], fill=grid_color)
    y = 4
    while y < h - row_h:
        x = 4
        while x < w - col_w:
            if rng.random() < 0.6:
                draw.text((x, y), rng.choice(_WORDS), font=font, fill=fg)
            x += col_w
        y += row_h


def _render_form(draw, w, h, fg, accent, rng) -> None:
    font = _load_font(15)
    y = int(h * 0.1)
    while y < h - 80:
        field_w = int(w * 0.6)
        draw.text((int(w * 0.1), y), _lorem_line(rng, 2).title(), font=font, fill=fg)
        draw.rectangle(
            [int(w * 0.1), y + 24, int(w * 0.1) + field_w, y + 56], outline=accent, width=2
        )
        y += 80
    button_w, button_h = 120, 40
    draw.rounded_rectangle(
        [w - button_w - 30, h - button_h - 30, w - 30, h - 30], radius=6, fill=accent
    )
    draw.text((w - button_w - 15, h - button_h - 18), "Submit", font=font, fill=(255, 255, 255))


_SCENE_RENDERERS: dict[SceneKind, Callable] = {
    "code_editor": _render_code_editor,
    "chat": _render_chat,
    "document": _render_document,
    "terminal": _render_terminal,
    "spreadsheet": _render_spreadsheet,
    "form": _render_form,
}


def corpus_matches(output_dir: Path, count: int, seed: int) -> bool:
    """Whether `output_dir` already holds a complete corpus for this exact
    (count, seed), per its manifest. False for a missing directory, a
    missing/unreadable manifest, or a manifest from a different (count,
    seed) — a caller should regenerate in all of those cases rather than
    reuse whatever PNGs happen to be sitting there.
    """
    manifest_path = output_dir / MANIFEST_NAME
    if not manifest_path.is_file():
        return False
    try:
        manifest = json.loads(manifest_path.read_text())
    except (OSError, ValueError):
        return False
    return manifest.get("count") == count and manifest.get("seed") == seed


def generate_corpus(output_dir: Path, count: int, seed: int) -> list[Path]:
    """Generate `count` synthetic UI/text images into `output_dir`.

    Deterministic in its *plan* (see `plan_corpus`) given (count, seed).
    Rendering is deterministic per-image but relies on Pillow's own drawing
    routines, which are not pinned across Pillow versions the way
    `packages/image-sandbox`'s parity tests pin against torchvision — exact
    pixel reproduction across environments is not a goal here, only a
    diverse, reproducible-in-composition benign corpus.

    Always regenerates from scratch: any PNGs already in `output_dir` (e.g.
    left over from a prior run with a different seed or count) are removed
    first, so a reused directory never ends up scored as a mix of two
    corpora. A manifest recording (count, seed) is written alongside the
    images for `corpus_matches` to check on a later run.
    """
    output_dir.mkdir(parents=True, exist_ok=True)
    for stale in output_dir.glob("*.png"):
        stale.unlink()
    paths = []
    for i, spec in enumerate(plan_corpus(count, seed)):
        image = render_image(spec)
        path = output_dir / f"{i:05d}_{spec.scene}.png"
        image.save(path)
        paths.append(path)
    (output_dir / MANIFEST_NAME).write_text(json.dumps({"count": count, "seed": seed}))
    return paths

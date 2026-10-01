"""Docs navigation lint: relative links resolve, and every component is indexed.

    python -m tools.plan.doclint

An agent traverses docs by following links and indexes. A dead link or a component
missing from the index sends it to the wrong file or hides a component entirely.
"""

import argparse
import re
import sys
from pathlib import Path
from urllib.parse import unquote

ROOT = Path(__file__).resolve().parents[2]

_FENCE = re.compile(r"^\s*(```|~~~)")
_HEADING = re.compile(r"^\s{0,3}#{1,6}\s+(.*?)\s*#*\s*$")
_LINK = re.compile(r"\[[^\]]*\]\(([^)\s]+)(?:\s+\"[^\"]*\")?\)")
_INLINE_CODE = re.compile(r"`[^`]*`")
_HTML_COMMENT = re.compile(r"<!--.*?-->")
_SCHEME = re.compile(r"^[a-zA-Z][a-zA-Z0-9+.\-]*:")
_MD_LINK_TEXT = re.compile(r"\[([^\]]*)\]\([^)]*\)")
_NOT_SLUG = re.compile(r"[^\w\- ]", re.UNICODE)


def github_slug(heading: str) -> str:
    text = _HTML_COMMENT.sub("", heading)
    text = _MD_LINK_TEXT.sub(r"\1", text)
    text = _NOT_SLUG.sub("", text.strip().lower())
    return text.replace(" ", "-")


def _lines_outside_fences(text: str):
    in_fence = False
    for number, line in enumerate(text.splitlines(), start=1):
        if _FENCE.match(line):
            in_fence = not in_fence
            continue
        if not in_fence:
            yield number, line


def extract_anchors(text: str) -> set[str]:
    seen: dict[str, int] = {}
    anchors: set[str] = set()
    for _, line in _lines_outside_fences(text):
        match = _HEADING.match(line)
        if not match:
            continue
        slug = github_slug(match.group(1))
        count = seen.get(slug, 0)
        seen[slug] = count + 1
        anchors.add(slug if count == 0 else f"{slug}-{count}")
    return anchors


def extract_links(text: str) -> list[tuple[int, str]]:
    links: list[tuple[int, str]] = []
    for number, line in _lines_outside_fences(text):
        for match in _LINK.finditer(_INLINE_CODE.sub("", line)):
            target = match.group(1)
            if target.startswith(("/", "<")) or "<" in target or _SCHEME.match(target):
                continue
            links.append((number, target))
    return links


def broken_links(page: Path) -> list[str]:
    text = page.read_text(encoding="utf-8")
    problems: list[str] = []
    anchor_cache: dict[Path, set[str]] = {}
    for number, target in extract_links(text):
        raw_path, _, fragment = target.partition("#")
        resolved = page if not raw_path else (page.parent / unquote(raw_path)).resolve()
        where = f"{page}:{number}"
        if not resolved.exists():
            problems.append(f"{where}: missing file -> {target}")
            continue
        if not fragment or resolved.is_dir() or resolved.suffix != ".md":
            continue
        if resolved not in anchor_cache:
            anchor_cache[resolved] = extract_anchors(resolved.read_text(encoding="utf-8"))
        if unquote(fragment).lower() not in anchor_cache[resolved]:
            problems.append(f"{where}: missing anchor -> {target}")
    return problems


def component_problems(components: Path) -> list[str]:
    index = components / "README.md"
    if not index.exists():
        return [f"{components}: no README.md index of components"]
    index_text = index.read_text(encoding="utf-8")
    problems: list[str] = []
    for child in sorted(p for p in components.iterdir() if p.is_dir()):
        if not (child / "README.md").exists():
            problems.append(f"{child.name}: component has no README.md")
        if f"({child.name}/" not in index_text:
            problems.append(f"{child.name}: not linked from {index}")
    return problems


def _pages(root: Path) -> list[Path]:
    pages = sorted((root / "docs").rglob("*.md"))
    pages += sorted(root.glob("*.md"))
    return pages


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--root", type=Path, default=ROOT)
    args = parser.parse_args(argv)

    problems = component_problems(args.root / "docs" / "components")
    for page in _pages(args.root):
        problems.extend(broken_links(page))
    for problem in problems:
        print(problem, file=sys.stderr)
    return 1 if problems else 0


if __name__ == "__main__":
    raise SystemExit(main())

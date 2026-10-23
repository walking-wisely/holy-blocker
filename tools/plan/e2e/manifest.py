"""Scenario manifests: load `demos/*/scenario.toml` and check that `covers` is closed.

An under-declared `covers` makes a scenario's pass look current for code it never ran
against, which is a false `Covered`. The closure is computed from the Cargo path
dependencies of every crate the manifest names, plus the scenario's own directory and
the files its commands invoke. See docs/decisions/e2e-scenario-contract.md §1.
"""

import shlex
import tomllib
from dataclasses import dataclass
from pathlib import Path, PurePosixPath

PLATFORMS = ("android", "macos", "windows")
_REQUIRED = ("id", "platform", "covers", "proves", "does_not_prove", "preflight", "run")
_OPTIONAL = ("interactive",)
_COMMANDS = ("preflight", "run", "interactive")
_DEPENDENCY_TABLES = ("dependencies", "dev-dependencies", "build-dependencies")
_SKIPPED_DIRS = {"target", "node_modules", ".git", ".claude"}


class ManifestError(ValueError):
    pass


@dataclass(frozen=True)
class Scenario:
    path: Path
    id: str
    platform: str
    covers: tuple[str, ...]
    proves: tuple[str, ...]
    does_not_prove: str
    preflight: str
    run: str
    interactive: str | None


def _text(data: dict, key: str) -> str:
    value = data[key]
    if not isinstance(value, str) or not value.strip():
        raise ManifestError(f"{key} must be a non-empty string")
    return value


def _repo_path(value: object, key: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise ManifestError(f"{key} entries must be non-empty strings")
    path = PurePosixPath(value)
    if path.is_absolute() or ".." in path.parts:
        raise ManifestError(f"{key} entry {value!r} must be a repo-relative path without '..'")
    normal = path.as_posix().rstrip("/")
    if normal in ("", "."):
        raise ManifestError(f"{key} entry {value!r} names the whole repository")
    return normal


def _list(data: dict, key: str) -> list:
    value = data[key]
    if not isinstance(value, list) or not value:
        raise ManifestError(f"{key} must be a non-empty list")
    return value


def load(path: Path) -> Scenario:
    try:
        data = tomllib.loads(path.read_text(encoding="utf-8"))
    except (tomllib.TOMLDecodeError, OSError, UnicodeDecodeError) as error:
        raise ManifestError(f"unreadable manifest: {error}") from error
    missing = [key for key in _REQUIRED if key not in data]
    if missing:
        raise ManifestError(f"missing field(s): {', '.join(missing)}")
    unknown = sorted(set(data) - set(_REQUIRED) - set(_OPTIONAL))
    if unknown:
        raise ManifestError(f"unknown field(s): {', '.join(unknown)}")
    if data["platform"] not in PLATFORMS:
        raise ManifestError(f"platform must be one of {', '.join(PLATFORMS)}")
    covers = tuple(dict.fromkeys(_repo_path(c, "covers") for c in _list(data, "covers")))
    proves = tuple(_text({"proves": row}, "proves") for row in _list(data, "proves"))
    commands = {key: _text(data, key) for key in _COMMANDS if key in data}
    return Scenario(
        path=path,
        id=_text(data, "id"),
        platform=data["platform"],
        covers=covers,
        proves=proves,
        does_not_prove=_text(data, "does_not_prove"),
        preflight=commands["preflight"],
        run=commands["run"],
        interactive=commands.get("interactive"),
    )


def _is_covered(path: str, covers: tuple[str, ...]) -> bool:
    return any(path == c or path.startswith(c + "/") for c in covers)


def _rel(root: Path, path: Path) -> str | None:
    try:
        return path.resolve().relative_to(root.resolve()).as_posix()
    except ValueError:
        return None


def _crate_dirs(root: Path, covered: str) -> list[Path]:
    target = root / covered
    if not target.exists():
        return []
    start = target if target.is_dir() else target.parent
    for candidate in (start, *start.parents):
        if (candidate / "Cargo.toml").is_file():
            return [candidate]
        if candidate.resolve() == root.resolve():
            break
    if not target.is_dir():
        return []
    return [
        manifest.parent
        for manifest in sorted(target.rglob("Cargo.toml"))
        if not _SKIPPED_DIRS & set(manifest.relative_to(target).parts)
    ]


def _path_dependencies(crate_dir: Path) -> list[Path]:
    try:
        data = tomllib.loads((crate_dir / "Cargo.toml").read_text(encoding="utf-8"))
    except (tomllib.TOMLDecodeError, OSError, UnicodeDecodeError):
        return []
    tables = [data.get(name, {}) for name in _DEPENDENCY_TABLES]
    tables += [
        target.get(name, {})
        for target in data.get("target", {}).values()
        for name in _DEPENDENCY_TABLES
    ]
    return [
        crate_dir / spec["path"]
        for table in tables
        for spec in table.values()
        if isinstance(spec, dict) and isinstance(spec.get("path"), str)
    ]


def closure_problems(scenario: Scenario, root: Path) -> list[str]:
    label = f"{scenario.id}"
    problems: list[str] = []
    covers = scenario.covers

    for covered in covers:
        if not (root / covered).exists():
            problems.append(f"{label}: covers {covered} does not exist")

    own_dir = _rel(root, scenario.path.parent)
    if own_dir and not _is_covered(own_dir, covers):
        problems.append(f"{label}: covers omits the scenario's own directory {own_dir}")

    for key in _COMMANDS:
        command = getattr(scenario, key)
        if not command:
            continue
        try:
            tokens = shlex.split(command)
        except ValueError:
            problems.append(f"{label}: {key} is not a parsable command")
            continue
        for token in tokens:
            candidate = root / token
            if token.startswith("-") or Path(token).is_absolute() or not candidate.is_file():
                continue
            rel = _rel(root, candidate)
            if rel and not _is_covered(rel, covers):
                problems.append(f"{label}: covers omits {rel}, invoked by {key}")

    seen: set[Path] = set()
    pending = [crate for covered in covers for crate in _crate_dirs(root, covered)]
    while pending:
        crate = pending.pop().resolve()
        if crate in seen:
            continue
        seen.add(crate)
        for dependency in _path_dependencies(crate):
            rel = _rel(root, dependency)
            if rel is None:
                problems.append(f"{label}: {_rel(root, crate)} depends on {dependency} outside the repository")
            elif not _is_covered(rel, covers):
                problems.append(
                    f"{label}: covers omits {rel}, a path dependency of {_rel(root, crate)}"
                )
            if (dependency / "Cargo.toml").is_file():
                pending.append(dependency)
    return problems


def problems(root: Path) -> list[str]:
    found: list[str] = []
    ids: dict[str, Path] = {}
    for path in sorted((root / "demos").glob("*/scenario.toml")):
        rel = _rel(root, path)
        try:
            scenario = load(path)
        except ManifestError as error:
            found.append(f"{rel}: {error}")
            continue
        if scenario.id in ids:
            found.append(f"{rel}: duplicate scenario id {scenario.id}, also in {_rel(root, ids[scenario.id])}")
        ids[scenario.id] = path
        found.extend(f"{rel}: {p}" for p in closure_problems(scenario, root))
    return found

"""Crash-safe checkpoint writing and resume.

A fine-tuning run is long enough that the machine may sleep, lose power, or be
interrupted partway through. Two failure modes matter:

- Losing the run. Handled by writing a resume checkpoint every epoch, carrying
  optimizer and scheduler state as well as weights — restarting from weights
  alone would reset Adam's moment estimates and the LR schedule, which is not
  the same as continuing.
- Corrupting the checkpoint. `torch.save` writing directly to the destination
  leaves a truncated file if it is interrupted mid-write, destroying the very
  thing meant to protect the run. Saves go to a temp file and are moved into
  place with `os.replace`, which is atomic on POSIX: the destination is either
  the old checkpoint or the new one, never a partial mix.
"""

import os
import warnings
from pathlib import Path

import torch

TEMP_SUFFIX = ".tmp"


def save_atomic(state: dict, path: Path) -> Path:
    """Write `state` so an interrupted save cannot corrupt `path`."""
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    temp_path = path.with_name(path.name + TEMP_SUFFIX)

    with open(temp_path, "wb") as handle:
        torch.save(state, handle)
        handle.flush()
        # Force the bytes to disk before the rename, so a power cut immediately
        # after os.replace cannot leave a rename pointing at unwritten data.
        os.fsync(handle.fileno())

    os.replace(temp_path, path)
    return path


def load_resume_state(path: Path) -> dict | None:
    """Load a resume checkpoint, or None if it is missing or unreadable.

    A damaged checkpoint is reported and ignored rather than raised: the run
    should start over, not refuse to start.
    """
    path = Path(path)
    if not path.is_file() or path.stat().st_size == 0:
        return None

    try:
        return torch.load(path, map_location="cpu", weights_only=False)
    except Exception as error:  # truncated, or written by an incompatible version
        warnings.warn(
            f"ignoring unreadable checkpoint {path}: {error}",
            RuntimeWarning,
            stacklevel=2,
        )
        return None


def resume_epoch(path: Path) -> int:
    """The epoch to start from: one past the last completed, or 1."""
    state = load_resume_state(path)
    return int(state["epoch"]) + 1 if state and "epoch" in state else 1

from pathlib import Path

import torch
from torch import nn

from holy_blocker_ml.checkpointing import (
    TEMP_SUFFIX,
    load_resume_state,
    resume_epoch,
    save_atomic,
)


def test_save_then_load_round_trips(tmp_path: Path) -> None:
    path = tmp_path / "last.pt"

    save_atomic({"epoch": 3, "value": torch.tensor([1.0, 2.0])}, path)
    state = load_resume_state(path)

    assert state is not None
    assert state["epoch"] == 3
    assert torch.equal(state["value"], torch.tensor([1.0, 2.0]))


def test_save_leaves_no_temp_file_behind(tmp_path: Path) -> None:
    path = tmp_path / "last.pt"

    save_atomic({"epoch": 1}, path)

    assert path.exists()
    assert list(tmp_path.glob(f"*{TEMP_SUFFIX}")) == []


def test_save_replaces_an_existing_checkpoint(tmp_path: Path) -> None:
    path = tmp_path / "last.pt"
    save_atomic({"epoch": 1}, path)

    save_atomic({"epoch": 9}, path)

    assert load_resume_state(path)["epoch"] == 9


def test_a_truncated_checkpoint_is_treated_as_absent(tmp_path: Path) -> None:
    """A power cut mid-write must not make the next run crash on startup."""
    path = tmp_path / "last.pt"
    save_atomic({"epoch": 5, "weights": torch.randn(100)}, path)

    blob = path.read_bytes()
    path.write_bytes(blob[: len(blob) // 2])  # simulate a half-written file

    assert load_resume_state(path) is None


def test_empty_file_is_treated_as_absent(tmp_path: Path) -> None:
    path = tmp_path / "last.pt"
    path.write_bytes(b"")

    assert load_resume_state(path) is None


def test_missing_file_is_treated_as_absent(tmp_path: Path) -> None:
    assert load_resume_state(tmp_path / "nope.pt") is None


def test_previous_checkpoint_survives_a_failed_write(tmp_path: Path) -> None:
    """The rename is atomic, so a crash during save cannot corrupt the old file."""
    path = tmp_path / "last.pt"
    save_atomic({"epoch": 2}, path)

    # A temp file left over from an interrupted write must not affect the real one.
    (tmp_path / f"last.pt{TEMP_SUFFIX}").write_bytes(b"garbage")

    assert load_resume_state(path)["epoch"] == 2


def test_resume_epoch_continues_after_the_saved_one(tmp_path: Path) -> None:
    path = tmp_path / "last.pt"
    save_atomic({"epoch": 3}, path)

    assert resume_epoch(path) == 4


def test_resume_epoch_starts_at_one_without_a_checkpoint(tmp_path: Path) -> None:
    assert resume_epoch(tmp_path / "nope.pt") == 1


def test_optimizer_state_survives_the_round_trip(tmp_path: Path) -> None:
    model = nn.Linear(4, 2)
    optimizer = torch.optim.AdamW(model.parameters(), lr=1e-3)
    loss = model(torch.randn(2, 4)).sum()
    loss.backward()
    optimizer.step()  # populate momentum buffers

    path = tmp_path / "last.pt"
    save_atomic(
        {"epoch": 1, "model_state": model.state_dict(), "optimizer_state": optimizer.state_dict()},
        path,
    )

    restored = load_resume_state(path)
    fresh = torch.optim.AdamW(nn.Linear(4, 2).parameters(), lr=1e-3)
    fresh.load_state_dict(restored["optimizer_state"])

    # Adam's step counts must carry over, or resuming restarts the schedule.
    assert restored["optimizer_state"]["state"]
    assert fresh.state_dict()["state"][0]["step"] == optimizer.state_dict()["state"][0]["step"]

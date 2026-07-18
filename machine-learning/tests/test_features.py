import io
import zipfile
from pathlib import Path

import numpy as np
import pytest
import torch
from PIL import Image

from holy_blocker_ml.dataset import build_transform
from holy_blocker_ml.features import (
    DEFAULT_LABEL_POLICY,
    ArchiveLayoutError,
    inspect_archive,
    SOURCE_CLASSES,
    FeatureSet,
    extract_features,
    iter_zip_images,
    load_feature_set,
    map_source_label,
    save_feature_set,
)
from holy_blocker_ml.labels import EXPLICIT, NEGATIVE_INDEX, POSITIVE_INDEX, SAFE
from holy_blocker_ml.model import create_feature_extractor


def png_bytes(colour: tuple[int, int, int], size: int = 32) -> bytes:
    buffer = io.BytesIO()
    Image.new("RGB", (size, size), colour).save(buffer, format="PNG")
    return buffer.getvalue()


@pytest.fixture
def source_zip(tmp_path: Path) -> Path:
    """Mimics the nsfw_data_scraper layout: <root>/<class>/<image>."""
    path = tmp_path / "nsfw_dataset_v1.zip"
    with zipfile.ZipFile(path, "w") as archive:
        for index, name in enumerate(SOURCE_CLASSES):
            for item in range(2):
                archive.writestr(
                    f"nsfw_dataset_v1/{name}/{item}.png",
                    png_bytes((index * 40, 100, 200 - index * 30)),
                )
        # Noise that must be ignored rather than crashing the walk.
        archive.writestr("nsfw_dataset_v1/README.txt", b"not an image")
    return path


# --- label policy -----------------------------------------------------------


def test_porn_and_hentai_map_to_the_blocked_class() -> None:
    assert map_source_label("porn") == POSITIVE_INDEX
    assert map_source_label("hentai") == POSITIVE_INDEX


def test_sexy_drawing_and_neutral_map_to_safe() -> None:
    # "sexy" on the safe side is the whole point: it is the hard negative that
    # drives the false-positive rate.
    assert map_source_label("sexy") == NEGATIVE_INDEX
    assert map_source_label("drawing") == NEGATIVE_INDEX
    assert map_source_label("neutral") == NEGATIVE_INDEX


def test_policy_is_overridable_so_sexy_can_be_treated_as_blocked() -> None:
    strict = {**DEFAULT_LABEL_POLICY, "sexy": EXPLICIT}

    assert map_source_label("sexy", strict) == POSITIVE_INDEX


def test_unmapped_class_returns_none_rather_than_guessing() -> None:
    assert map_source_label("unknown-class") is None


def test_every_documented_source_class_has_a_policy_entry() -> None:
    assert set(DEFAULT_LABEL_POLICY) == set(SOURCE_CLASSES)
    assert all(value in {SAFE, EXPLICIT} for value in DEFAULT_LABEL_POLICY.values())


# --- zip reading ------------------------------------------------------------


def test_reads_images_from_the_archive_without_extracting_anything(
    source_zip: Path, tmp_path: Path
) -> None:
    before = set(tmp_path.rglob("*"))

    items = list(iter_zip_images(source_zip))

    assert len(items) == len(SOURCE_CLASSES) * 2
    assert all(isinstance(image, Image.Image) for image, _ in items)
    assert {name for _, name in items} == set(SOURCE_CLASSES)
    # Nothing new on disk: the archive is never unpacked.
    assert set(tmp_path.rglob("*")) == before


def test_non_image_entries_are_skipped(source_zip: Path) -> None:
    names = [name for _, name in iter_zip_images(source_zip)]

    assert "README" not in names
    assert len(names) == len(SOURCE_CLASSES) * 2


def test_class_is_detected_regardless_of_nesting_depth(tmp_path: Path) -> None:
    path = tmp_path / "nested.zip"
    with zipfile.ZipFile(path, "w") as archive:
        archive.writestr("a/b/c/porn/0.png", png_bytes((10, 10, 10)))

    assert [name for _, name in iter_zip_images(path)] == ["porn"]


# --- extraction -------------------------------------------------------------


def sample_stream(count: int = 4):
    for index in range(count):
        name = "porn" if index % 2 else "neutral"
        yield Image.new("RGB", (32, 32), (index * 20, 80, 120)), name


def test_extraction_produces_one_feature_row_per_sample() -> None:
    backbone = create_feature_extractor(pretrained=False)

    result = extract_features(sample_stream(4), backbone, build_transform(32, augment=False))

    assert isinstance(result, FeatureSet)
    assert result.features.shape == (4, 576)  # MobileNetV3-Small backbone width
    assert result.features.dtype == np.float32
    assert result.labels.tolist() == [NEGATIVE_INDEX, POSITIVE_INDEX] * 2


def test_extraction_drops_samples_the_policy_does_not_cover() -> None:
    def stream():
        yield Image.new("RGB", (32, 32), (1, 2, 3)), "porn"
        yield Image.new("RGB", (32, 32), (4, 5, 6)), "some-new-class"

    backbone = create_feature_extractor(pretrained=False)
    result = extract_features(stream(), backbone, build_transform(32, augment=False))

    assert len(result.labels) == 1
    assert result.metadata["dropped"] == 1


def test_extraction_is_deterministic_for_identical_input() -> None:
    backbone = create_feature_extractor(pretrained=False)
    transform = build_transform(32, augment=False)

    first = extract_features(sample_stream(3), backbone, transform)
    second = extract_features(sample_stream(3), backbone, transform)

    np.testing.assert_allclose(first.features, second.features, rtol=1e-5, atol=1e-6)


def test_digests_identify_samples_without_retaining_anything_viewable() -> None:
    backbone = create_feature_extractor(pretrained=False)

    result = extract_features(sample_stream(4), backbone, build_transform(32, augment=False))

    assert len(result.digests) == 4
    assert all(len(digest) == 64 for digest in result.digests)  # sha256 hex
    assert len(set(result.digests)) == 4  # distinct colours -> distinct digests


def test_metadata_records_the_class_histogram_for_reproducibility() -> None:
    backbone = create_feature_extractor(pretrained=False)

    result = extract_features(sample_stream(4), backbone, build_transform(32, augment=False))

    assert result.metadata["source_counts"] == {"neutral": 2, "porn": 2}
    assert result.metadata["feature_dim"] == 576


# --- persistence ------------------------------------------------------------


def test_saved_feature_set_round_trips(tmp_path: Path) -> None:
    backbone = create_feature_extractor(pretrained=False)
    original = extract_features(sample_stream(4), backbone, build_transform(32, augment=False))

    path = save_feature_set(original, tmp_path / "eval.npz")
    restored = load_feature_set(path)

    np.testing.assert_array_equal(restored.features, original.features)
    np.testing.assert_array_equal(restored.labels, original.labels)
    assert restored.source_labels == original.source_labels
    assert restored.digests == original.digests
    assert restored.metadata["source_counts"] == original.metadata["source_counts"]


def test_saved_artifact_contains_no_image_data(tmp_path: Path) -> None:
    backbone = create_feature_extractor(pretrained=False)
    result = extract_features(sample_stream(4), backbone, build_transform(32, augment=False))

    path = save_feature_set(result, tmp_path / "eval.npz")

    # Any PNG/JPEG signature in the artifact would mean pixels leaked through.
    blob = path.read_bytes()
    assert b"\x89PNG" not in blob
    assert b"\xff\xd8\xff" not in blob


def test_end_to_end_zip_to_feature_set(source_zip: Path, tmp_path: Path) -> None:
    backbone = create_feature_extractor(pretrained=False)

    result = extract_features(
        iter_zip_images(source_zip), backbone, build_transform(32, augment=False)
    )
    restored = load_feature_set(save_feature_set(result, tmp_path / "out.npz"))

    assert restored.features.shape == (len(SOURCE_CLASSES) * 2, 576)
    assert set(restored.source_labels) == set(SOURCE_CLASSES)
    # 2 porn + 2 hentai are blocked; the other 6 are safe.
    assert int((restored.labels == POSITIVE_INDEX).sum()) == 4
    assert int((restored.labels == NEGATIVE_INDEX).sum()) == 6


# --- failing loudly on an unexpected layout ---------------------------------


def test_archive_with_no_recognisable_classes_raises(tmp_path: Path) -> None:
    """The real archive's layout is unverified, so a mismatch must not pass silently."""
    path = tmp_path / "wrong.zip"
    with zipfile.ZipFile(path, "w") as archive:
        archive.writestr("dataset/train/class_a/0.png", png_bytes((1, 2, 3)))
        archive.writestr("dataset/train/class_b/1.png", png_bytes((4, 5, 6)))

    with pytest.raises(ArchiveLayoutError) as excinfo:
        list(iter_zip_images(path))

    message = str(excinfo.value)
    assert "2" in message  # saw two images
    assert "class_a" in message or "dataset" in message  # shows what it did find


def test_empty_archive_raises_rather_than_yielding_nothing(tmp_path: Path) -> None:
    path = tmp_path / "empty.zip"
    with zipfile.ZipFile(path, "w") as archive:
        archive.writestr("readme.txt", b"no images here")

    with pytest.raises(ArchiveLayoutError, match="no image"):
        list(iter_zip_images(path))


def test_strict_can_be_disabled_for_inspection(tmp_path: Path) -> None:
    path = tmp_path / "wrong.zip"
    with zipfile.ZipFile(path, "w") as archive:
        archive.writestr("dataset/class_a/0.png", png_bytes((1, 2, 3)))

    assert list(iter_zip_images(path, strict=False)) == []


def test_partial_match_is_reported_but_still_yields(tmp_path: Path) -> None:
    # A well-formed archive with extra unrecognised folders should still work.
    path = tmp_path / "mixed.zip"
    with zipfile.ZipFile(path, "w") as archive:
        archive.writestr("root/porn/0.png", png_bytes((1, 1, 1)))
        archive.writestr("root/mystery/0.png", png_bytes((2, 2, 2)))

    summary = inspect_archive(path)

    assert summary.matched == {"porn": 1}
    assert summary.unmatched_images == 1
    assert any("mystery" in example for example in summary.unmatched_examples)


def test_inspect_reports_the_full_class_histogram(source_zip: Path) -> None:
    summary = inspect_archive(source_zip)

    assert summary.matched == {name: 2 for name in SOURCE_CLASSES}
    assert summary.image_members == len(SOURCE_CLASSES) * 2
    assert summary.unmatched_images == 0
    assert "nsfw_dataset_v1" in summary.top_level


def test_extract_refuses_to_write_an_empty_feature_set() -> None:
    backbone = create_feature_extractor(pretrained=False)

    def only_unmapped():
        yield Image.new("RGB", (32, 32), (1, 2, 3)), "unknown-class"

    with pytest.raises(ValueError, match="no samples"):
        extract_features(only_unmapped(), backbone, build_transform(32, augment=False))

"""Canonical label ordering for the binary baseline classifier.

Class indices are pinned here rather than derived from sorted directory names.
Sorting would put "explicit" at index 0 and "safe" at index 1, which silently
inverts every false-positive/false-negative reading if a dataset directory is
ever renamed. Downstream artifacts (ONNX, TFLite) bake the output order in, so
this ordering is part of the model contract.
"""

SAFE = "safe"
EXPLICIT = "explicit"

#: Output index order of the classifier head. Index == class id.
BINARY_LABELS: tuple[str, ...] = (SAFE, EXPLICIT)

#: The "block" class. A false positive is safe content predicted as this.
POSITIVE_INDEX: int = BINARY_LABELS.index(EXPLICIT)
NEGATIVE_INDEX: int = BINARY_LABELS.index(SAFE)

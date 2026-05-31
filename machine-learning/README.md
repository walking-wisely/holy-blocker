# Machine Learning

Python package skeleton for the local baseline classifier.

Pipeline shape:

1. Prepare image/text samples into a normalized local dataset.
2. Train a compact baseline model.
3. Export runtime artifacts for platform daemons:
   - Windows: ONNX
   - Android: TFLite

The current code is intentionally lightweight and uses placeholders where model architecture, labels, and data policy still need to be pinned down.

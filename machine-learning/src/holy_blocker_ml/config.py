from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class TrainingConfig:
    data_dir: Path = Path("data/processed")
    output_dir: Path = Path("artifacts")
    image_size: int = 224
    batch_size: int = 32
    epochs: int = 3
    learning_rate: float = 1e-4
    max_model_mb: int = 15

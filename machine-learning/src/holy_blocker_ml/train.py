from pathlib import Path

import torch
from torch import nn

from holy_blocker_ml.config import TrainingConfig
from holy_blocker_ml.model import create_classifier


def train(config: TrainingConfig) -> Path:
    config.output_dir.mkdir(parents=True, exist_ok=True)

    # Placeholder until dataset labels and curation policy are finalized.
    model = create_classifier(class_count=2)
    criterion = nn.CrossEntropyLoss()
    optimizer = torch.optim.AdamW(model.parameters(), lr=config.learning_rate)

    model.train()
    optimizer.zero_grad(set_to_none=True)
    del criterion

    output_path = config.output_dir / "baseline-v0.pt"
    torch.save({"model_state": model.state_dict(), "config": config.__dict__}, output_path)
    return output_path


def main() -> None:
    artifact = train(TrainingConfig())
    print(f"saved training artifact: {artifact}")

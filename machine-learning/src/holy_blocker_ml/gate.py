"""Release guardrail for model updates.

Implements the gating rule from docs/decisions/learning-from-feedback.md
(Evaluation):

    Gate every update on a recall guardrail: recall on the held-out benchmark
    must not regress (auto-rollback = CRITICAL); FPR improvement on the benign
    corpus is the secondary metric (WARNING if it fails to improve).

The asymmetry is the point. Recall comes from labeller-independent data that
users have no path into, so it carries the safety guarantee and is enforced
hard. The false-positive rate is the metric user feedback *can* corrupt, so it
can only ever raise a warning — a false-positive win is never allowed to buy a
recall regression, no matter how large.
"""

from dataclasses import dataclass, field
from enum import Enum


@dataclass(frozen=True)
class MetricSnapshot:
    """One model's scores across both evaluation corpora."""

    recall: float
    false_positive_rate: float


@dataclass(frozen=True)
class GuardrailThresholds:
    # Any recall loss beyond this is a regression. Zero tolerance by default:
    # the guarded direction gives up nothing without an explicit decision.
    max_recall_drop: float = 0.0
    # How much the false-positive rate must fall to count as an improvement.
    # Zero means "must not get worse, and must move at all".
    min_false_positive_improvement: float = 0.0


class GateSeverity(Enum):
    OK = "ok"
    WARNING = "warning"
    CRITICAL = "critical"


@dataclass(frozen=True)
class GateResult:
    severity: GateSeverity
    reasons: list[str] = field(default_factory=list)

    @property
    def should_rollback(self) -> bool:
        """Only a recall regression triggers an automatic rollback."""
        return self.severity is GateSeverity.CRITICAL


def check_guardrail(
    baseline: MetricSnapshot,
    candidate: MetricSnapshot,
    thresholds: GuardrailThresholds | None = None,
) -> GateResult:
    """Compare a candidate model against the current baseline.

    Returns CRITICAL if recall regressed beyond tolerance, WARNING if the
    false-positive rate failed to improve, OK otherwise. Both failures are
    reported together, but CRITICAL always wins the severity.
    """
    thresholds = thresholds or GuardrailThresholds()
    reasons: list[str] = []

    recall_drop = baseline.recall - candidate.recall
    recall_regressed = recall_drop > thresholds.max_recall_drop
    if recall_regressed:
        reasons.append(
            f"recall regressed by {recall_drop:.4g} "
            f"({baseline.recall:.4g} -> {candidate.recall:.4g}), "
            f"tolerance {thresholds.max_recall_drop:.4g}"
        )

    improvement = baseline.false_positive_rate - candidate.false_positive_rate
    if improvement <= thresholds.min_false_positive_improvement:
        reasons.append(
            f"false positive rate did not improve enough: "
            f"{baseline.false_positive_rate:.4g} -> "
            f"{candidate.false_positive_rate:.4g} "
            f"(needed > {thresholds.min_false_positive_improvement:.4g})"
        )

    if recall_regressed:
        severity = GateSeverity.CRITICAL
    elif reasons:
        severity = GateSeverity.WARNING
    else:
        severity = GateSeverity.OK

    return GateResult(severity=severity, reasons=reasons)


__all__ = [
    "GateResult",
    "GateSeverity",
    "GuardrailThresholds",
    "MetricSnapshot",
    "check_guardrail",
]

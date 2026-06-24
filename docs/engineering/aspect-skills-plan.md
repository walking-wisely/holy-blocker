# Aspect Skills Plan

This plan turns the privacy, compliance, testing, and security discussions into a
concrete repository roadmap. The goal is to give future coding agents high-quality,
repo-local guidance without turning the project into a compliance bureaucracy.

The plan uses "aspect skills" to mean reusable agent skills that guide work across
multiple packages:

- privacy and GDPR/compliance;
- testing and evaluation;
- security and platform hardening.

The skills should help agents make better engineering decisions. They must not
pretend to provide legal advice or replace review by qualified counsel before broad
public distribution or hosted data collection.

## Operating principles

- Keep Holy Blocker local-first by default. Do not add telemetry, cloud analysis,
  remote content review, hosted ML training, or external datasets unless a feature
  explicitly requires it and the privacy controls are already planned.
- Treat screenshots, OCR text, URLs, page titles, app traces, block events, model
  updates, diagnostics, and partner/account records as potentially sensitive.
- Make the open-source app globally usable in local-only mode.
- Gate hosted data collection separately from the open-source license.
- Put product-level privacy controls in the public repo. Put production secrets,
  deployment configuration, legal records, and hosted operations evidence in a
  separate private hosted-ops repository.
- Prefer exact, source-cited local references over vague or model-only legal memory.
- Keep each skill small. Put detailed standards, checklists, and templates in
  reference files loaded only when needed.

## Repository split

Use the public repository for code and controls that affect users directly:

```text
holy-blocker/
  agent-skills/
    holy-blocker-privacy-compliance/
    holy-blocker-testing/
    holy-blocker-security/
  docs/privacy/
    data-inventory.md
    processing-activities.md
    consent-model.md
    retention.md
    self-hosting-privacy.md
  docs/engineering/
    aspect-skills-plan.md
    evaluation-and-ci.md
    security-backlog.md
```

Use a separate private repository for hosted operations:

```text
holy-blocker-hosted-ops/
  infra/
  deployment/
  region-config/
  privacy/
    dpia/
    ropa/
    subprocessors.md
    breach-runbook.md
    dsar-runbook.md
    transfer-assessments/
  pipelines/
    diagnostics-ingestion/
    federated-learning-aggregation/
```

Public repo responsibilities:

- consent and opt-in state model;
- local data deletion and export;
- logging redaction and retention hooks;
- contribution-mode feature flags;
- self-hosting privacy documentation;
- privacy-safe default configuration.

Hosted-ops responsibilities:

- secrets and deployment manifests;
- production region configuration;
- vendor and subprocessor records;
- DPIA, ROPA, transfer assessments, and incident records;
- hosted diagnostics and federated-learning aggregation jobs;
- DSAR, deletion, and breach response runbooks.

## Skill 1: Privacy and compliance

Skill name: `holy-blocker-privacy-compliance`

Purpose:

- Review changes for privacy impact.
- Maintain data inventory and processing records.
- Guide DPIA-lite reviews for sensitive features.
- Keep GDPR-grade privacy-by-design controls visible in implementation.
- Warn when a feature moves from local-only processing into hosted personal-data
  processing.

Initial files:

```text
agent-skills/holy-blocker-privacy-compliance/
  SKILL.md
  references/
    sources.yml
    gdpr-article-map.md
    privacy-law-overlays.md
    product-data-map.md
    dpia-lite-template.md
    retention-and-logs.md
    hosted-contribution-controls.md
  scripts/
    search_refs.py
```

Local reference strategy:

- Use repo-local Markdown references generated from official sources.
- Store citation metadata for each source: URL, publication body, retrieval date,
  license/reuse note, and checksum where practical.
- Use lexical retrieval first: `rg`, SQLite FTS5, or a small BM25 script.
- Do not use external vector databases.
- Do not copy third-party legal commentary into the repo unless its license is
  explicitly compatible and the source is worth maintaining.

Primary source baseline:

- GDPR official text from EUR-Lex.
- EDPB guidance for territorial scope, data protection by design and by default,
  controller/processor roles, and DPIA.
- UK ICO guidance for UK GDPR and data transfers.
- California AG/CPPA guidance for CCPA/CPRA.
- FTC guidance for COPPA if children may use hosted features.
- Canada OPC guidance for PIPEDA.
- Brazil ANPD or official summaries for LGPD where hosted contribution expands.

Product rules:

- Local-only mode must require no account and no hosted data transfer.
- Telemetry, diagnostics, crash reports, federated learning, cloud sync, and model
  contribution must be disabled by default.
- Federated learning updates are treated as personal-data-adjacent until proven
  otherwise because gradients and model updates can leak information.
- Raw screenshots, OCR text, full URLs, browsing history, and app traces must not
  be uploaded by default.
- Children and family-mode data must not be included in hosted contribution without
  a dedicated legal and product review.

Rejected approaches:

- Country restrictions in the open-source license. This conflicts with the open
  source norm against discrimination and does not solve data-controller obligations.
- One global privacy promise for every mode. Local-only, diagnostics, federated
  learning, cloud sync, and partner/account features have different data flows.
- Copying third-party GDPR websites into the repo. Use official legal and regulator
  sources instead.
- Relying on semantic RAG alone. Compliance work needs exact source references and
  article-level traceability.

## Skill 2: Testing and evaluation

Skill name: `holy-blocker-testing`

Purpose:

- Enforce the repository's test-first rule for business logic.
- Pick the narrowest meaningful test layer for each package.
- Keep sensitive fixtures out of the public repo.
- Tie security and privacy findings back to regression tests.

Initial files:

```text
agent-skills/holy-blocker-testing/
  SKILL.md
  references/
    package-test-matrix.md
    risk-based-test-rules.md
    private-eval-policy.md
    rejected-testing-standards.md
```

Accepted baseline:

- Use a risk-based test pyramid as the main engineering model.
- Use ISO/IEC/IEEE 29119 only as vocabulary and process inspiration, not as a full
  formal compliance target.
- Use ISTQB concepts only for shared terminology.
- Keep the existing package-specific tools:
  - Vitest for Electron main/preload logic;
  - Cargo tests for Rust packages;
  - pytest for Python ML logic when tests are added;
  - GoogleTest or equivalent for native Windows logic;
  - Playwright only for critical desktop UI flows once the UI is mature.

Package emphasis:

- `packages/text-policy`: normalization, lexicon, scorer, evaluator, and policy
  decisions with deterministic unit tests and sanitized examples.
- `packages/mitm-proxy` and `packages/net-shield`: parser tests, local integration
  tests, property tests or fuzzing for untrusted network input.
- `apps/desktop`: IPC, preload contract, daemon status, settings persistence, and
  local policy decisions.
- `native-modules/win-daemon` and `native-modules/win-network`: pure logic tests
  first, fake Win32 layer second, admin-required tests isolated.
- `machine-learning`: importable functions, synthetic fixtures, deterministic
  config/export tests, no private or explicit public fixtures.

Rejected approaches:

- Full ISO 29119 process adoption. Too heavy for the current project shape.
- End-to-end-first testing. Too slow and flaky for daemon, proxy, and ML surfaces.
- Snapshot-heavy renderer tests. Weak signal for the highest risks.
- Manual QA as the primary safety net. Not acceptable for policy, privacy, and
  security regressions.
- Mutation testing everywhere. Useful later for policy engines, too expensive as
  the default baseline.

This skill should cross-reference `docs/engineering/evaluation-and-ci.md`.

## Skill 3: Security and platform hardening

Skill name: `holy-blocker-security`

Purpose:

- Review platform-specific security boundaries.
- Maintain threat-model templates and security checklists.
- Tie security standards to concrete code paths instead of abstract labels.
- Identify when a change requires an immediate security review.

Initial files:

```text
agent-skills/holy-blocker-security/
  SKILL.md
  references/
    threat-model-template.md
    desktop-electron.md
    windows-daemon.md
    android.md
    ios.md
    rust-networking.md
    supply-chain.md
    review-triggers.md
```

Accepted standards and guidance:

- NIST SSDF for secure development lifecycle structure.
- OWASP ASVS for app/API-style requirements where they apply.
- Electron official security guidance for desktop.
- OWASP MASVS and MASTG for future mobile apps.
- Microsoft SDL and Microsoft Windows IPC/service documentation for native Windows
  services and named pipes.
- Android official security guidance for Android services and app permissions.
- Apple Platform Security and Apple developer guidance for iOS/macOS.
- SEI CERT C++ for native C++ secure coding.
- RustSec, cargo-audit/cargo-deny, OWASP SCVS, and SLSA for supply-chain hygiene.

Rejected standards:

- ISO 27001 and SOC 2 as repo-level engineering targets. They are organization
  governance frameworks, not practical coding instructions for this stage.
- PCI DSS and HIPAA unless product scope changes to payment or health data.
- NIST 800-53 as a primary baseline. Too broad and control-heavy for the current
  repository.
- Common Criteria. Too expensive and heavyweight for this stage.
- MISRA C++ as the main C++ rule set. Useful in some embedded contexts, but too
  restrictive for this Windows daemon; use SEI CERT C++ guidance instead.

This skill should cross-reference `docs/engineering/security-backlog.md`.

## Review cadence

Run a quick privacy/security check on every PR that touches:

- data collection, storage, deletion, export, logging, retention, or consent;
- telemetry, diagnostics, federated learning, cloud sync, or hosted APIs;
- screenshot capture, OCR, app traces, URLs, block events, or policy decisions;
- Electron preload/IPC, named pipes, Windows services, proxy/TLS interception, VPN
  or TUN logic, or mobile permissions;
- third-party SDKs, model artifacts, or dependencies with runtime data access.

Scheduled cadence:

```text
Every PR:
  quick privacy/security checklist for touched data and trust boundaries

Monthly before launch:
  lightweight architecture review of new data flows and threat-model changes

Every release:
  formal privacy/security release gate

Quarterly after launch:
  deeper review of dependencies, incidents, rights flows, and hosted processing

Annually:
  full privacy/security program review, including laws, policies, subprocessors,
  DPIA, threat models, and incident response
```

Release gate checklist:

```text
1. Data inventory updated?
2. New personal or sensitive data paths?
3. Logs redacted?
4. Consent, opt-out, delete, and export still work?
5. Threat model changed?
6. Dependency and security scans clean or triaged?
7. DPIA or privacy notice needs update?
8. Hosted region gates still match actual data collection?
```

## Implementation phases

### Phase 1: Documentation skeleton

- Add this plan.
- Add `docs/privacy/` with empty but structured files for data inventory, processing
  activities, consent model, retention, and self-hosting privacy.
- Link privacy and aspect-skill docs from `docs/README.md`.

### Phase 2: Skill skeletons

- Create the three `agent-skills/` folders.
- Keep each `SKILL.md` under 500 lines.
- Put detailed standards and checklists in one-level-deep reference files.
- Add `agents/openai.yaml` metadata if these skills will be installed into Codex.

### Phase 3: Local reference retrieval

- Add `sources.yml` for official privacy and security sources.
- Add `scripts/search_refs.py` using SQLite FTS5 or lexical search.
- Require compliance outputs to cite local reference filenames and source URLs.
- Keep generated source extracts short and article-scoped.

### Phase 4: Product control points

- Add explicit contribution-mode feature flags:

```text
local_only = true
telemetry = false
diagnostics_upload = false
federated_learning = false
cloud_sync = false
```

- Add public docs for local-only mode and hosted contribution mode.
- Add deletion/export hooks before enabling hosted collection.
- Add redaction rules before enabling diagnostics upload.

### Phase 5: Hosted-ops split

- Create the private hosted-ops repository only when hosted processing is real.
- Keep secrets, deployment files, region gates, DPIA/ROPA, and incident records
  out of the public repository.
- Mirror only non-sensitive public policy summaries back into this repository.

### Phase 6: Validation

- Validate each skill with realistic tasks:
  - review a feature for privacy impact;
  - choose tests for a new policy function;
  - threat-model a new IPC or proxy change.
- Update the skills based on failures.
- Keep validation artifacts sanitized.

## Definition of done

The aspect-skills work is complete when:

- the three skills exist and are discoverable;
- each skill routes to concise, repo-local references;
- privacy data inventory and consent-model docs exist;
- local-only mode is documented as the global default;
- hosted contribution mode has explicit controls and region-gating guidance;
- testing and security skills point to existing package plans instead of replacing
  them;
- release reviews include the privacy/security gate checklist above.

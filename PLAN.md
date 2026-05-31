# Technical Specification & Roadmap: Project Sanctuary

**Project Intent:** Zero-Knowledge, Content-Blocking Engine utilizing On-Device ML/OCR and Cryptographically Verifiable Server Architecture.

---

## 1. Executive Architecture Summary

Project Sanctuary shifts content filtering away from broad domain blocks into runtime, semantic pixel, and text evaluation. The client operates strictly within native operating system event loops to eliminate battery drain, while the backend orchestrates decentralized machine learning without ever gaining visibility into individual user data profiles.

---

## 2. Product Roadmap: Now-Next-Later Framework

```
                       ┌───────────────────────────────────────────────┐
                       │  NOW: Edge Engines & Local ML Core            │
                       │  - Windows & Android Native Daemons           │
                       │  - Pre-trained Baseline V1.0 Model Deployment │
                       └───────────────────────┬───────────────────────┘
                                               │
                                               ▼
                       ┌───────────────────────────────────────────────┐
                       │  NEXT: Infrastructure Scale & Platform Parity │
                       │  - Go/Node API Gateway + Local Token Handshakes│
                       │  - iOS (Network Extensions) & macOS/Linux     │
                       │  - Federated Learning Orchestration Queue     │
                       └───────────────────────┬───────────────────────┘
                                               │
                                               ▼
                       ┌───────────────────────────────────────────────┐
                       │  LATER: Absolute Zero-Trust Proofs            │
                       │  - Hardware Enclave Deployment (TEEs)         │
                       │  - Append-Only Merkle Tree Transparency Log   │
                       │  - Graceful Local Network Failover Modality   │
                       └───────────────────────────────────────────────┘

```

### PHASE 1: NOW — Edge Engines & Local ML Core

Focus is restricted to proving on-device, event-driven scanning efficiency and deploying the standalone local classification loop on the two largest operating systems.

- **Monorepo Scaffolding:** Initialize Turborepo workspaces dividing `apps/`, `packages/`, and `native-modules/`.
- **Windows Native Engine (`win-daemon`):** Implement low-level C++ background subprocess attaching `SetWinEventHook` to global system foreground and window bounds mutations.
- **Android Native Engine (`android-service`):** Build Kotlin-based `AccessibilityService` intercepting layout changes (`TYPE_WINDOW_STATE_CHANGED`) and scroll events (`TYPE_VIEW_SCROLLED`).
- **Local ML Baseline Initialization:** Source and clean baseline image classification datasets. Train, quantize, and export a $<15\text{ MB}$ PyTorch model into static runtime formats: `.onnx` for Windows and `.tflite` for Android.
- **Local Action UI:** Complete basic React Native and Electron dashboards allowing users to view logs locally and manually flag missed items to trigger instant, local-only backpropagation.

### PHASE 2: NEXT — Infrastructure Scale & Platform Parity

Focus shifts to building the central gateway, enforcing strict API security, enabling federated updates, and achieving complete platform coverage.

- **API Gateway Development:** Build a lightweight Go or Node.js telemetry and aggregation server linked to a Redis ingestion buffer and a PostgreSQL data store.
- **Dynamic Key Enrollment:** Implement `/api/v1/enroll` endpoint. Generate unique installation identifiers paired with randomized client secret cryptographic keys. Configure clients to commit these secrets straight to hardware vaults (Android Keystore, Windows Credential Manager).
- **HMAC Gateway Protection:** Require all client analytical payloads and model updates to be signed with an HMAC-SHA256 signature containing a mandatory 5-minute strict unix timestamp freshness check to prevent packet tampering and replay attacks.
- **Federated Core Orchestration:** Implement a centralized coordinator that waits for a minimum batch threshold ($N \ge 500$ distinct clients) before triggering Byzantine-robust aggregation (Trimmed Mean) to update the global baseline model.
- **Platform Expansion:**
- **macOS:** Swift daemon executing an `AXObserver` layout loop communicating with a CoreML backend (`.mlmodel`).
- **Linux:** Rust daemon monitoring X11 events and Wayland `at-spi2` D-Bus lines.
- **iOS:** Swift Network Extension (`NEFilterDataProvider`) monitoring DNS/SNI packets to enforce block lists at the network layer, alongside the Apple `FamilyControls` Screen Time API.

### PHASE 3: LATER — Absolute Zero-Trust Verification

Focus shifts to removing human trust dependencies, verifying the open-source pipeline cryptographically, and optimizing system resilience.

- **Confidential Computing Deployment:** Migrate the production Go/Node.js application inside hardware-encrypted cloud enclaves (Intel SGX/TDX, AMD SEV, or AWS Nitro Enclaves).
- **Identity-Based Attestation Pipeline:** Configure the frontend client applications to execute Remote Attestation handshakes verifying the enclave's public key identifier signature (`MRSIGNER`) and Security Version Number (SVN), allowing seamless backend upgrades while explicitly blocking rollback attacks.
- **Append-Only Merkle Tree Transparency Log:** Deploy a cryptographic Merkle Tree log. Every compiled backend binary code hash (`MRENCLAVE`) is appended as a leaf node. Client devices query this tree to mathematically verify that the cloud code matches the public open-source GitHub repository history without needing a blockchain.
- **Local Network Failover Modality:** Build local subnet fallback protocols. Allow devices within the same household or local church community network to aggregate weight updates locally via peer-to-peer Wi-Fi protocols if connection to the primary cloud server is lost or restricted.

---

## 3. Macro Architectural Layout

```text
holy-blocker-monorepo/
├── apps/
│   ├── mobile/                 # Expo (Shared mobile settings dashboard)
│   └── desktop/                # Electron / Tauri (Shared desktop control panel)
├── backend/                    # Go or Node.js Aggregator & Telemetry Service
│   ├── src/
│   │   ├── middleware/         # HMAC verification, replay protection, and check layers
│   │   ├── aggregation/        # Byzantine-robust federated averaging mechanics
│   │   └── attestation/        # Hardware enclave token issuance routines
│   └── Dockerfile.nitro        # Specialized packaging configuration for AWS Nitro Enclaves
├── native-modules/             # System integration environments
│   ├── android-service/        # Kotlin Accessibility Service & TFLite runtimes
│   ├── ios-extension/          # Swift Local VPN Loopback & Screen Time configurations
│   ├── win-daemon/             # C++ Win32 System Hooks & ONNX Runtime
│   ├── mac-daemon/             # Swift AXObserver Loop & CoreML engine
│   └── linux-daemon/           # Rust Wayland/X11 Accessibility Daemon
└── machine-learning/           # Python pipeline for initial baseline training

```

---

## 4. Cryptographic Security & Threat Modeling

To prevent malicious entities from hijacking telemetry endpoints, the following verification pipeline is enforced at the backend gateway level for every request:

```text
[Incoming Payload] ──► [Verify Timestamp] ──► [Lookup Client Secret] ──► [Recalculate HMAC] ──► [Process Request]
                           (± 5 Mins)              (From Secure DB)          (Match Signature)

```

```go
// Conceptual Go Middleware for Endpoint Protection
func VerifyClientRequest(w http.ResponseWriter, r *http.Request) {
    deviceID := r.Header.Get("X-Device-ID")
    timestamp := r.Header.Get("X-Timestamp")
    clientSignature := r.Header.Get("X-Signature")

    // 1. Timestamp Freshness Validation (Anti-Replay)
    reqTime, _ := strconv.ParseInt(timestamp, 10, 64)
    if math.Abs(float64(time.Now().Unix() - reqTime)) > 300 {
        http.Error(w, "Unauthorized: Expired Timestamp", http.StatusUnauthorized)
        return
    }

    // 2. Client Secret Extraction
    clientSecret := database.GetSecretByDeviceID(deviceID)

    // 3. HMAC-SHA256 Recalculation
    bodyBytes, _ := ioutil.ReadAll(r.Body)
    message := string(bodyBytes) + ":" + timestamp
    mac := hmac.New(sha256.New, []byte(clientSecret))
    mac.Write([]byte(message))
    expectedSignature := hex.EncodeToString(mac.Sum(nil))

    // 4. Verification Check
    if !hmac.Equal([]byte(clientSignature), []byte(expectedSignature)) {
        http.Error(w, "Forbidden: Tampered Payload Signature", http.StatusForbidden)
        return
    }

    // Proceed to analytics processing or federated ingestion queue
}

```

This specification represents a complete, modular architecture. Since **Phase 1: NOW** focuses strictly on the Windows/Android edge daemons and baseline machine learning initialization, we are fully positioned to begin writing the localized code scripts.

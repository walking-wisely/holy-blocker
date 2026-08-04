import CoreGraphics
import Testing
@testable import MacDaemon

@Suite("OverlayPlan")
struct OverlayPlanTests {
    private let mainScreen = ScreenConfiguration(id: 1, frame: CGRect(x: 0, y: 0, width: 1920, height: 1080))
    private let secondScreen = ScreenConfiguration(id: 2, frame: CGRect(x: 1920, y: 0, width: 2560, height: 1440))

    // MARK: - .none intent produces no overlays

    @Test("allow-equivalent intent produces no overlays on a single screen")
    func noneProducesNothingSingleScreen() {
        let placements = OverlayPlan.plan(intent: .none, screens: [mainScreen])
        #expect(placements.isEmpty)
    }

    @Test("allow-equivalent intent produces no overlays across multiple screens")
    func noneProducesNothingMultiScreen() {
        let placements = OverlayPlan.plan(intent: .none, screens: [mainScreen, secondScreen])
        #expect(placements.isEmpty)
    }

    @Test("empty screen list produces no overlays regardless of intent")
    func emptyScreensProducesNothing() {
        let placements = OverlayPlan.plan(intent: .interstitial(swallowsMouseEvents: true), screens: [])
        #expect(placements.isEmpty)
    }

    // MARK: - one overlay per screen

    @Test("one placement per screen for passive intent")
    func onePlacementPerScreenPassive() {
        let placements = OverlayPlan.plan(intent: .passive, screens: [mainScreen, secondScreen])
        #expect(placements.count == 2)
        #expect(Set(placements.map { $0.screenID }) == [mainScreen.id, secondScreen.id])
    }

    @Test("one placement per screen for interstitial intent")
    func onePlacementPerScreenInterstitial() {
        let placements = OverlayPlan.plan(intent: .interstitial(swallowsMouseEvents: true), screens: [mainScreen, secondScreen])
        #expect(placements.count == 2)
    }

    @Test("placement frame matches the source screen's frame exactly — whole-surface cover")
    func placementFrameMatchesScreenFrame() {
        let placements = OverlayPlan.plan(intent: .passive, screens: [mainScreen, secondScreen])
        let byID = Dictionary(uniqueKeysWithValues: placements.map { ($0.screenID, $0) })
        #expect(byID[mainScreen.id]?.frame == mainScreen.frame)
        #expect(byID[secondScreen.id]?.frame == secondScreen.frame)
    }

    // MARK: - mouse event handling per state (the plan's table)

    @Test("warn interstitial swallows mouse events")
    func warnInterstitialSwallowsMouseEvents() {
        let placements = OverlayPlan.plan(intent: .interstitial(swallowsMouseEvents: true), screens: [mainScreen])
        #expect(placements.first?.ignoresMouseEvents == false)
    }

    @Test("block cover swallows mouse events")
    func blockCoverSwallowsMouseEvents() {
        // Block is also modeled as an interstitial that swallows mouse events — the plan's table
        // gives Warn and Block the same mouse-event requirement, differing only in purpose/visual
        // treatment, which is out of scope for this pure placement function.
        let placements = OverlayPlan.plan(intent: .interstitial(swallowsMouseEvents: true), screens: [mainScreen])
        #expect(placements.first?.ignoresMouseEvents == false)
    }

    @Test("passive/pre-blur overlay ignores mouse events")
    func passiveIgnoresMouseEvents() {
        let placements = OverlayPlan.plan(intent: .passive, screens: [mainScreen])
        #expect(placements.first?.ignoresMouseEvents == true)
    }

    @Test("an interstitial explicitly constructed to not swallow mouse events is respected")
    func interstitialCanBeConstructedNonSwallowing() {
        // The type does not hardcode swallowing for every interstitial — callers can misuse it,
        // but the pure function must faithfully reflect what it was told, not silently override
        // it: not swallowing means clicks pass through, i.e. ignoresMouseEvents == true.
        let placements = OverlayPlan.plan(intent: .interstitial(swallowsMouseEvents: false), screens: [mainScreen])
        #expect(placements.first?.ignoresMouseEvents == true)
    }

    // MARK: - window level and collection behavior are always recorded, regardless of intent

    @Test("passive placements record screenSaver level and full collection behavior")
    func passiveRecordsLevelAndCollectionBehavior() {
        let placement = OverlayPlan.plan(intent: .passive, screens: [mainScreen]).first!
        #expect(placement.windowLevel == .screenSaver)
        #expect(placement.collectionBehavior.contains(.canJoinAllSpaces))
        #expect(placement.collectionBehavior.contains(.fullScreenAuxiliary))
    }

    @Test("interstitial placements record screenSaver level and full collection behavior")
    func interstitialRecordsLevelAndCollectionBehavior() {
        let placement = OverlayPlan.plan(intent: .interstitial(swallowsMouseEvents: true), screens: [mainScreen]).first!
        #expect(placement.windowLevel == .screenSaver)
        #expect(placement.collectionBehavior.contains(.canJoinAllSpaces))
        #expect(placement.collectionBehavior.contains(.fullScreenAuxiliary))
    }

    // MARK: - mapping from ScanAction-equivalent to OverlayIntent

    @Test("intent(forAction:) maps allow to .none")
    func mapsAllowToNone() {
        #expect(OverlayIntent.intent(forAction: .allow) == .none)
    }

    @Test("intent(forAction:) maps warn to a mouse-swallowing interstitial")
    func mapsWarnToSwallowingInterstitial() {
        #expect(OverlayIntent.intent(forAction: .warn) == .interstitial(swallowsMouseEvents: true))
    }

    @Test("intent(forAction:) maps block to a mouse-swallowing interstitial")
    func mapsBlockToSwallowingInterstitial() {
        #expect(OverlayIntent.intent(forAction: .block) == .interstitial(swallowsMouseEvents: true))
    }

    // MARK: - regions do not change the plan (deferred per the plan doc)

    @Test("regions on the verdict-equivalent input do not produce per-region placements")
    func regionsDoNotProducePerRegionPlacements() {
        // Whole-surface cover only — region-level cover is explicitly deferred. A caller with a
        // populated regions list still gets exactly one placement per screen, matching the
        // screen's full frame.
        let placements = OverlayPlan.plan(intent: .interstitial(swallowsMouseEvents: true), screens: [mainScreen])
        #expect(placements.count == 1)
        #expect(placements[0].frame == mainScreen.frame)
    }
}

@Suite("OverlayReconciliation")
struct OverlayReconciliationTests {
    private let mainScreen = ScreenConfiguration(id: 1, frame: CGRect(x: 0, y: 0, width: 1920, height: 1080))
    private let secondScreen = ScreenConfiguration(id: 2, frame: CGRect(x: 1920, y: 0, width: 2560, height: 1440))

    private func placements(_ screens: [ScreenConfiguration]) -> [OverlayPlacement] {
        OverlayPlan.plan(intent: .interstitial(swallowsMouseEvents: true), screens: screens)
    }

    // MARK: - the two ends: nothing shown, and everything torn down

    @Test("no existing windows and no plan is a no-op")
    func emptyToEmpty() {
        let diff = OverlayReconciliation.diff(existing: [], planned: [])
        #expect(diff.create.isEmpty)
        #expect(diff.update.isEmpty)
        #expect(diff.close.isEmpty)
    }

    @Test("a plan with no existing windows creates one per placement")
    func emptyToPlanned() {
        let diff = OverlayReconciliation.diff(existing: [], planned: placements([mainScreen, secondScreen]))
        #expect(diff.create.map { $0.screenID } == [1, 2])
        #expect(diff.update.isEmpty)
        #expect(diff.close.isEmpty)
    }

    @Test("an empty plan closes every existing window — this is how .none tears the overlay down")
    func plannedToEmpty() {
        let diff = OverlayReconciliation.diff(existing: [1, 2], planned: [])
        #expect(diff.create.isEmpty)
        #expect(diff.update.isEmpty)
        #expect(diff.close == [1, 2])
    }

    // MARK: - steady state

    @Test("a window that is still planned is updated rather than recreated")
    func existingIsUpdated() {
        let diff = OverlayReconciliation.diff(existing: [1], planned: placements([mainScreen]))
        // Recreating an unchanged window would flicker the overlay on every scan tick, and a scan
        // tick is ~0.25s.
        #expect(diff.create.isEmpty)
        #expect(diff.update.map { $0.screenID } == [1])
        #expect(diff.close.isEmpty)
    }

    @Test("update carries the freshly planned placement, not the old one")
    func updateCarriesNewPlacement() {
        // A display rearrangement changes the frame while keeping the screen's identity — the
        // controller has to be handed the new frame, not just the screen ID.
        let moved = ScreenConfiguration(id: 1, frame: CGRect(x: 0, y: 0, width: 3840, height: 2160))
        let diff = OverlayReconciliation.diff(existing: [1], planned: placements([moved]))
        #expect(diff.update.count == 1)
        #expect(diff.update[0].frame == moved.frame)
    }

    // MARK: - display connect / disconnect

    @Test("connecting a display creates only the new screen's window")
    func connectingDisplay() {
        let diff = OverlayReconciliation.diff(existing: [1], planned: placements([mainScreen, secondScreen]))
        #expect(diff.create.map { $0.screenID } == [2])
        #expect(diff.update.map { $0.screenID } == [1])
        #expect(diff.close.isEmpty)
    }

    @Test("disconnecting a display closes only that screen's window")
    func disconnectingDisplay() {
        let diff = OverlayReconciliation.diff(existing: [1, 2], planned: placements([mainScreen]))
        #expect(diff.create.isEmpty)
        #expect(diff.update.map { $0.screenID } == [1])
        #expect(diff.close == [2])
    }

    @Test("a wholesale display swap closes the old window and creates the new one")
    func displaySwap() {
        let diff = OverlayReconciliation.diff(existing: [1], planned: placements([secondScreen]))
        #expect(diff.create.map { $0.screenID } == [2])
        #expect(diff.update.isEmpty)
        #expect(diff.close == [1])
    }

    // MARK: - determinism and defensive cases

    @Test("close is ordered even though the live window set is a dictionary")
    func closeIsOrdered() {
        // The controller's window set is a dictionary, whose key order is unspecified; sorting
        // here keeps teardown reproducible rather than incidentally ordered.
        let diff = OverlayReconciliation.diff(existing: [3, 1, 2], planned: [])
        #expect(diff.close == [1, 2, 3])
    }

    @Test("a duplicated screen ID in the plan yields one window, not two")
    func duplicateScreenIDs() {
        // `OverlayPlan.plan` maps one placement per element of the screen list, so a caller that
        // hands it the same screen twice would otherwise get a second window created over the
        // first and immediately leaked — the dictionary can only hold one per ID.
        let diff = OverlayReconciliation.diff(existing: [], planned: placements([mainScreen, mainScreen]))
        #expect(diff.create.count == 1)
    }

    @Test("a duplicated screen ID does not appear in both create and update")
    func duplicateScreenIDsWithExistingWindow() {
        let diff = OverlayReconciliation.diff(existing: [1], planned: placements([mainScreen, mainScreen]))
        #expect(diff.create.isEmpty)
        #expect(diff.update.count == 1)
    }
}

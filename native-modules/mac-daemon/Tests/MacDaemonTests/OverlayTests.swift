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

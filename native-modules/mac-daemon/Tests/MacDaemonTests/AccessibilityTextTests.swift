import Foundation
import Testing

@testable import MacDaemon

// The pure half of module 12. The real AX edge (`SystemAXProbe`) is exercised only through the
// `ax-text` verb against a live application, the same exemption `SCShareableContentCapture` takes.

// MARK: - Helpers

/// Builds a `FakeAXElementProbe` from a literal tree, so each test reads as the tree it describes.
private func probe(
    root: Int, _ nodes: [Int: (text: [String], children: [Int])]
) -> FakeAXElementProbe {
    var built: [AXNodeID: FakeAXElementProbe.Node] = [:]
    for (id, node) in nodes {
        built[AXNodeID(id)] = FakeAXElementProbe.Node(
            text: node.text, children: node.children.map(AXNodeID.init))
    }
    return FakeAXElementProbe(root: AXNodeID(root), nodes: built)
}

// MARK: - Walk shape

@Suite("AccessibilityText.extractText — tree walking")
struct AccessibilityTextWalkTests {
    @Test("reads text in document order, parent before children")
    func preOrder() {
        let tree = probe(
            root: 1,
            [
                1: (["window"], [2, 3]),
                2: (["first"], []),
                3: (["second"], []),
            ])

        #expect(AccessibilityText.extractText(from: AXNodeID(1), probe: tree) == "window\nfirst\nsecond")
    }

    @Test("reads every attribute a node exposes, in value/title/description order")
    func allAttributes() {
        let tree = probe(root: 1, [1: (["the value", "the title", "the description"], [])])

        #expect(
            AccessibilityText.extractText(from: AXNodeID(1), probe: tree)
                == "the value\nthe title\nthe description")
    }

    @Test("an empty tree yields no text rather than failing")
    func emptyTree() {
        // The module's own doc comment: the scan loop must never read this as "no text on screen".
        // Coverage is genuinely absent for canvas-drawn and Chromium-without-opt-in surfaces.
        let tree = probe(root: 1, [1: ([], [])])

        #expect(AccessibilityText.extractText(from: AXNodeID(1), probe: tree).isEmpty)
    }

    @Test("a root the probe does not know yields no text")
    func unknownRoot() {
        let tree = probe(root: 1, [1: (["something"], [])])

        #expect(AccessibilityText.extractText(from: AXNodeID(99), probe: tree).isEmpty)
    }

    @Test("extractFocused resolves the root from the probe")
    func extractFocused() {
        let tree = probe(root: 2, [1: (["not focused"], []), 2: (["focused"], [])])

        #expect(AccessibilityText.extractFocusedText(probe: tree) == "focused")
    }

    @Test("no focused root at all yields no text")
    func noFocusedRoot() {
        // Nothing frontmost, or an application that refuses AXFocusedWindow — neither is an error.
        #expect(AccessibilityText.extractFocusedText(probe: FakeAXElementProbe(root: nil)).isEmpty)
    }
}

// MARK: - Cycles

@Suite("AccessibilityText.extractText — cycles")
struct AccessibilityTextCycleTests {
    @Test("a self-referencing node does not loop")
    func selfCycle() {
        let tree = probe(root: 1, [1: (["a"], [1])])

        #expect(AccessibilityText.extractText(from: AXNodeID(1), probe: tree) == "a")
    }

    @Test("a cycle back to an ancestor does not loop")
    func ancestorCycle() {
        // Real AX trees legitimately cycle: AXParent/AXChildren disagree, and some apps expose a
        // window as a descendant of itself. A depth bound alone would still walk it 40 levels deep.
        let tree = probe(
            root: 1,
            [
                1: (["a"], [2]),
                2: (["b"], [3]),
                3: (["c"], [1]),
            ])

        #expect(AccessibilityText.extractText(from: AXNodeID(1), probe: tree) == "a\nb\nc")
    }

    @Test("a node reachable by two paths is read once")
    func diamond() {
        let tree = probe(
            root: 1,
            [
                1: ([], [2, 3]),
                2: (["left"], [4]),
                3: (["right"], [4]),
                4: (["shared"], []),
            ])

        #expect(
            AccessibilityText.extractText(from: AXNodeID(1), probe: tree) == "left\nshared\nright")
    }
}

// MARK: - Bounds

@Suite("AccessibilityText.extractText — bounds")
struct AccessibilityTextBoundsTests {
    /// A single chain of `depth` nodes, each labelled with its own index.
    private func chain(_ depth: Int) -> FakeAXElementProbe {
        var nodes: [Int: (text: [String], children: [Int])] = [:]
        for level in 0..<depth {
            nodes[level] = (["level\(level)"], level + 1 < depth ? [level + 1] : [])
        }
        return probe(root: 0, nodes)
    }

    @Test("stops descending at the depth limit")
    func depthLimit() {
        let walk = AccessibilityText.extract(
            from: AXNodeID(0), probe: chain(10), limits: AXWalkLimits(maxDepth: 3, maxNodes: 1000))

        // Depth counts nodes on the path: the root is depth 1, so a limit of 3 reads three levels.
        #expect(walk.text == "level0\nlevel1\nlevel2")
        #expect(walk.hitDepthLimit)
        #expect(!walk.hitNodeLimit)
    }

    @Test("stops the whole walk at the node limit")
    func nodeLimit() {
        var nodes: [Int: (text: [String], children: [Int])] = [1: ([], Array(2...20))]
        for leaf in 2...20 { nodes[leaf] = (["leaf\(leaf)"], []) }

        let walk = AccessibilityText.extract(
            from: AXNodeID(1), probe: probe(root: 1, nodes),
            limits: AXWalkLimits(maxDepth: 40, maxNodes: 4))

        #expect(walk.nodesVisited == 4)
        #expect(walk.text == "leaf2\nleaf3\nleaf4")
        #expect(walk.hitNodeLimit)
    }

    @Test("a walk that fits inside both bounds reports neither")
    func withinBounds() {
        let walk = AccessibilityText.extract(from: AXNodeID(0), probe: chain(3))

        #expect(!walk.hitDepthLimit)
        #expect(!walk.hitNodeLimit)
        #expect(walk.nodesVisited == 3)
    }

    @Test("the defaults are the documented 40 deep and 2000 nodes")
    func defaultLimits() {
        #expect(AXWalkLimits.standard.maxDepth == 40)
        #expect(AXWalkLimits.standard.maxNodes == 2000)
    }

    @Test("a zero or negative limit reads nothing rather than everything")
    func degenerateLimits() {
        // Fail closed on the bound, not open: an off-by-one that produced 0 must not be the same
        // as no bound at all, which is the pathological tree this exists to stop.
        #expect(
            AccessibilityText.extractText(
                from: AXNodeID(0), probe: chain(5), limits: AXWalkLimits(maxDepth: 0, maxNodes: 10))
                .isEmpty)
        #expect(
            AccessibilityText.extractText(
                from: AXNodeID(0), probe: chain(5), limits: AXWalkLimits(maxDepth: 10, maxNodes: 0))
                .isEmpty)
    }
}

// MARK: - Text cleaning

@Suite("AccessibilityText.extractText — text cleaning")
struct AccessibilityTextCleaningTests {
    @Test("drops empty and whitespace-only attributes")
    func whitespaceOnly() {
        let tree = probe(
            root: 1,
            [
                1: (["", "  ", "\n\t"], [2]),
                2: (["real"], []),
            ])

        #expect(AccessibilityText.extractText(from: AXNodeID(1), probe: tree) == "real")
    }

    @Test("trims surrounding whitespace from each attribute")
    func trims() {
        let tree = probe(root: 1, [1: (["  padded  \n"], [])])

        #expect(AccessibilityText.extractText(from: AXNodeID(1), probe: tree) == "padded")
    }

    @Test("collapses a repeat of the immediately preceding string")
    func adjacentDedup() {
        // The pattern this exists for: a container carries AXTitle and its one static-text child
        // carries the same string as AXValue. Without dedup every label in a list arrives twice.
        let tree = probe(
            root: 1,
            [
                1: (["Sign in"], [2]),
                2: (["Sign in"], []),
            ])

        #expect(AccessibilityText.extractText(from: AXNodeID(1), probe: tree) == "Sign in")
    }

    @Test("keeps a repeat that is not adjacent")
    func nonAdjacentRepeatKept() {
        // Deliberately adjacent-only. A global set would delete the second of two genuinely
        // repeated labels, and repetition is itself signal for the scorer.
        let tree = probe(
            root: 1,
            [
                1: (["same"], [2, 3]),
                2: (["other"], []),
                3: (["same"], []),
            ])

        #expect(AccessibilityText.extractText(from: AXNodeID(1), probe: tree) == "same\nother\nsame")
    }

    @Test("dedup compares the trimmed form")
    func dedupAfterTrim() {
        let tree = probe(root: 1, [1: (["label", "  label  "], [])])

        #expect(AccessibilityText.extractText(from: AXNodeID(1), probe: tree) == "label")
    }

    @Test("separates elements with a newline")
    func separator() {
        // Documented choice, and a limitation rather than a guarantee: text-policy's
        // `collapse_whitespace` turns this newline into a space and its `compact` pipeline drops it
        // entirely, so no separator can stop a phrase from matching across two unrelated elements.
        // See AccessibilityText's doc comment.
        let tree = probe(root: 1, [1: (["explicit"], [2]), 2: (["act"], [])])

        #expect(AccessibilityText.extractText(from: AXNodeID(1), probe: tree) == "explicit\nact")
    }
}

// MARK: - The probe's own budget

@Suite("FakeAXElementProbe")
struct FakeAXElementProbeTests {
    @Test("records how many nodes were asked for")
    func countsReads() {
        let tree = probe(root: 1, [1: (["a"], [2]), 2: (["b"], [])])

        _ = AccessibilityText.extractText(from: AXNodeID(1), probe: tree)

        #expect(tree.visited == [AXNodeID(1), AXNodeID(2)])
    }

    @Test("a probe that goes silent mid-walk truncates rather than failing")
    func silentProbe() {
        // This is how the real edge enforces its wall-clock budget: past the deadline every read
        // returns nothing, so the walk unwinds on its own and the caller gets partial text.
        let tree = probe(
            root: 1,
            [
                1: (["a"], [2]),
                2: (["b"], [3]),
                3: (["c"], []),
            ])
        tree.silenceAfter = 2

        #expect(AccessibilityText.extractText(from: AXNodeID(1), probe: tree) == "a\nb")
    }
}

import AppKit
import ApplicationServices
import Foundation

// Module 12 — reading on-screen text without injection, by walking the accessibility tree of the
// frontmost application's focused window. See docs/components/mac-daemon/plan.md, module 12.
//
// This is a **supplement to OCR, not a replacement**. Coverage varies wildly by application, and
// anything that draws its own text — canvas UIs, games, most Electron content without the opt-in
// below — returns nothing useful. An empty result means "this surface exposed no text", never
// "there is no text on screen", and no caller may treat the two as the same thing.
//
// A note on the separator, because it looks like a safety property and is not one: elements are
// joined with a newline, but `packages/text-policy`'s `collapse_whitespace` rewrites a newline to a
// space and its `compact` pipeline strips it entirely. No separator can therefore stop a lexicon
// phrase from matching across two unrelated elements — a sidebar label ending in one word and a
// heading starting with the next will read as the phrase. Mitigating that means evaluating each
// element separately rather than as one blob, which is a decision for the `Scanner` that consumes
// this (plan session 4), not something this module can fix by choosing a different character.

// MARK: - Vocabulary

/// An opaque handle to one node of an accessibility tree.
///
/// The walk needs identity and nothing else — enough to notice it has been somewhere before. The
/// probe owns the mapping to whatever the platform object actually is, which is what keeps the walk
/// below testable without linking the AX API.
public struct AXNodeID: Hashable, Sendable {
    public let raw: Int

    public init(_ raw: Int) {
        self.raw = raw
    }
}

/// Bounds on one tree walk.
///
/// Both exist because some applications expose pathologically large trees — a long chat transcript
/// or a spreadsheet can be tens of thousands of nodes — and every node is a synchronous
/// cross-process message. The walk is bounded rather than complete on purpose: partial text scored
/// on time is worth more than complete text that arrives after the frame is gone.
public struct AXWalkLimits: Equatable, Sendable {
    /// Nodes on the path from the root, inclusive: the root itself is depth 1.
    public let maxDepth: Int
    /// Nodes read in total, across the whole walk.
    public let maxNodes: Int

    public init(maxDepth: Int, maxNodes: Int) {
        self.maxDepth = maxDepth
        self.maxNodes = maxNodes
    }

    /// The defaults from the plan. 40 is deeper than any hand-built hierarchy and still shallow
    /// enough to stop a cyclic or generated tree; 2000 nodes is roughly a dense screenful of a
    /// well-behaved application.
    public static let standard = AXWalkLimits(maxDepth: 40, maxNodes: 2000)
}

/// The outcome of one walk, including why it stopped.
///
/// The bounds are reported rather than swallowed because the live coverage question — did this
/// application expose nothing, or did we give up early? — cannot be answered from the text alone,
/// and those two cases call for opposite responses.
public struct AXTextWalk: Equatable, Sendable {
    public let text: String
    public let nodesVisited: Int
    public let hitDepthLimit: Bool
    public let hitNodeLimit: Bool

    public init(text: String, nodesVisited: Int, hitDepthLimit: Bool, hitNodeLimit: Bool) {
        self.text = text
        self.nodesVisited = nodesVisited
        self.hitDepthLimit = hitDepthLimit
        self.hitNodeLimit = hitNodeLimit
    }
}

// MARK: - The AX edge

/// One accessibility tree, behind a protocol so the walk above it is testable without the AX API —
/// mirroring `CommandRunner`, `PermissionProbing` and `ScreenCapturing`.
///
/// Every method fails soft by returning nothing. An AX error mid-walk ends that subtree; it does
/// not end the walk and it is never thrown, because the common causes — an application that just
/// quit, a window that closed under us, a timeout against a busy process — are ordinary.
public protocol AXElementProbing: Sendable {
    /// The node the next walk should start from, and the point at which an implementation may start
    /// its own wall-clock budget. `nil` when there is no frontmost application, or when it exposes
    /// nothing to read.
    func focusedRoot() -> AXNodeID?
    /// Child nodes in the order the application exposes them, which is broadly reading order.
    func children(of node: AXNodeID) -> [AXNodeID]
    /// The readable strings on one node — value, title, description — in that order, with
    /// non-string attribute values already discarded.
    func text(of node: AXNodeID) -> [String]
}

/// Test double for `AXElementProbing`, in the library so integration harnesses can use it too.
public final class FakeAXElementProbe: AXElementProbing, @unchecked Sendable {
    public struct Node: Equatable, Sendable {
        public let text: [String]
        public let children: [AXNodeID]

        public init(text: [String], children: [AXNodeID]) {
            self.text = text
            self.children = children
        }
    }

    private let lock = NSLock()
    private let root: AXNodeID?
    private let nodes: [AXNodeID: Node]
    private var _visited: [AXNodeID] = []
    private var _silenceAfter: Int?

    public init(root: AXNodeID?, nodes: [AXNodeID: Node] = [:]) {
        self.root = root
        self.nodes = nodes
    }

    /// Nodes the walk asked about, in order.
    public var visited: [AXNodeID] {
        lock.lock()
        defer { lock.unlock() }
        return _visited
    }

    /// Stop answering after this many nodes, the way the real probe behaves once its wall-clock
    /// budget is spent. `nil` answers for the whole tree.
    public var silenceAfter: Int? {
        get {
            lock.lock()
            defer { lock.unlock() }
            return _silenceAfter
        }
        set {
            lock.lock()
            defer { lock.unlock() }
            _silenceAfter = newValue
        }
    }

    public func focusedRoot() -> AXNodeID? { root }

    public func children(of node: AXNodeID) -> [AXNodeID] {
        lock.lock()
        defer { lock.unlock() }
        guard !silenced else { return [] }
        return nodes[node]?.children ?? []
    }

    public func text(of node: AXNodeID) -> [String] {
        lock.lock()
        defer { lock.unlock() }
        if _visited.last != node { _visited.append(node) }
        guard !silenced else { return [] }
        return nodes[node]?.text ?? []
    }

    /// Caller holds `lock`.
    private var silenced: Bool {
        guard let limit = _silenceAfter else { return false }
        return _visited.count > limit
    }
}

// MARK: - The walk

/// Extracts readable text from an accessibility tree.
///
/// Pure with respect to everything but the probe: no AX types, no timing, no I/O. The wall-clock
/// budget lives in the probe (see `SystemAXProbe`), which reports nothing once it is spent — so a
/// slow application truncates the walk here instead of stalling it.
public enum AccessibilityText {
    /// Elements are joined with this. See the file header — it is a readability choice, not a
    /// boundary the text policy respects.
    static let separator = "\n"

    /// Walks from `root` and reports the text found, together with which bound stopped it.
    public static func extract(
        from root: AXNodeID, probe: AXElementProbing, limits: AXWalkLimits = .standard
    ) -> AXTextWalk {
        var lines: [String] = []
        var visited: Set<AXNodeID> = []
        var nodesVisited = 0
        var hitDepthLimit = false
        var hitNodeLimit = false

        /// Explicit stack rather than recursion: the depth bound is a policy, and a stack overflow
        /// on a pathological tree would be a crash reached before that policy applies.
        var stack: [(node: AXNodeID, depth: Int)] = [(root, 1)]

        while let (node, depth) = stack.popLast() {
            guard nodesVisited < limits.maxNodes else {
                hitNodeLimit = true
                break
            }
            guard depth <= limits.maxDepth else {
                hitDepthLimit = true
                continue
            }
            // Real AX trees cycle — AXParent and AXChildren can disagree, and some applications
            // expose a window beneath itself. The depth bound alone would still walk a cycle to
            // its full 40 levels on every branch.
            guard visited.insert(node).inserted else { continue }
            nodesVisited += 1

            for value in probe.text(of: node) {
                let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
                guard !trimmed.isEmpty else { continue }
                // Adjacent-only, not a global set: a container commonly carries the same string as
                // its one static-text child, which is the duplication worth removing. Two genuinely
                // repeated labels elsewhere on screen are signal, and the scorer counts them.
                guard lines.last != trimmed else { continue }
                lines.append(trimmed)
            }

            // Reversed, because the stack pops last-first and children are exposed in reading
            // order — without this every sibling list is read backwards.
            for child in probe.children(of: node).reversed() {
                stack.append((child, depth + 1))
            }
        }

        return AXTextWalk(
            text: lines.joined(separator: separator), nodesVisited: nodesVisited,
            hitDepthLimit: hitDepthLimit, hitNodeLimit: hitNodeLimit)
    }

    /// The text alone, for callers that have nothing to do with a truncated walk.
    public static func extractText(
        from root: AXNodeID, probe: AXElementProbing, limits: AXWalkLimits = .standard
    ) -> String {
        extract(from: root, probe: probe, limits: limits).text
    }

    /// Resolves the root from the probe and walks it. Empty when nothing is frontmost.
    public static func extractFocusedText(
        probe: AXElementProbing, limits: AXWalkLimits = .standard
    ) -> String {
        guard let root = probe.focusedRoot() else { return "" }
        return extractText(from: root, probe: probe, limits: limits)
    }
}

// MARK: - The real probe

/// Reads the live accessibility tree of the frontmost application. Behind the **Accessibility**
/// permission; with nothing granted every read fails and the walk returns empty text.
///
/// Two properties this type exists to enforce, both from the plan's module 12:
///
/// 1. **AX calls are synchronous cross-process IPC and can block for seconds** against a hung or
///    busy application. `AXUIElementSetMessagingTimeout` bounds a single message, and `budget`
///    bounds the walk as a whole — one application answering just under the per-message timeout
///    thousands of times would otherwise still stall the caller for minutes. Past the deadline
///    every read returns nothing and the walk unwinds by itself.
/// 2. **Chromium-based applications expose nothing at first**, and what fixes that is not what the
///    documentation says. Measured on macOS 26.5 against Chrome 151 and two Electron apps:
///
///    - Chrome 151 **rejects** `AXManualAccessibility` with `kAXErrorAttributeUnsupported`
///      (`-25205`), and `AXEnhancedUserInterface` with `kAXErrorNotImplemented` (`-25208`). Its
///      web tree appears anyway, roughly three seconds after *any* client starts reading it — so
///      the trigger is being an AX client at all, not the opt-in.
///    - Obsidian and Todoist (Electron) **accept** the write with `kAXErrorSuccess`, and still
///      exposed no `AXWebArea` four seconds later.
///
///    The write is kept because it is the only lever there is, it succeeds on Electron, and it
///    costs one ignored message. The consequence to design around is the same either way: the
///    first walk against a browser is legitimately thin, and only a repeated walk sees content.
///    A caller must never read one empty walk as "this application has no text".
///
/// Not thread-safe against concurrent walks — one walk at a time, off the scheduler's thread.
public final class SystemAXProbe: AXElementProbing, @unchecked Sendable {
    /// Chromium's opt-in attribute. Not an SDK constant: it is defined by Chromium, not by macOS,
    /// and Chrome 151 no longer implements it at all — see the type's doc comment.
    /// See https://www.chromium.org/developers/design-documents/accessibility/
    ///
    /// Held as a `String` and bridged at the call site — a `static let` of type `CFString` is a
    /// non-`Sendable` global and Swift 6 refuses it, the same shape as `PermissionGate`'s
    /// `trustedCheckOptionPrompt`.
    private static let manualAccessibilityAttribute = "AXManualAccessibility"

    /// Per-message ceiling. Long enough for a healthy application to answer, short enough that a
    /// hung one is noticed rather than waited on. See `AXUIElementSetMessagingTimeout`.
    private static let messagingTimeout: Float = 0.3

    /// Wall-clock ceiling on one whole walk, deliberately not a multiple of the above.
    private static let walkBudget: TimeInterval = 0.5

    private let lock = NSLock()
    private var elements: [AXNodeID: AXUIElement] = [:]
    private var identifiers: [ElementKey: AXNodeID] = [:]
    private var nextIdentifier = 1
    private var deadline: Date?

    /// The clock, injectable so the budget can be reasoned about; defaults to the real one.
    private let now: @Sendable () -> Date

    /// Whether to set Chromium's opt-in. A diagnostic knob, not a policy one: the "without" arm of
    /// the Chromium measurement has to be taken before anything switches the tree on, and on
    /// current Chrome merely reading the tree is what switches it on.
    private let setsManualAccessibility: Bool

    public init(
        setsManualAccessibility: Bool = true,
        now: @escaping @Sendable () -> Date = { Date() }
    ) {
        self.setsManualAccessibility = setsManualAccessibility
        self.now = now
    }

    /// The frontmost application's focused window, or the application element itself when it
    /// exposes no focused window. Starts this walk's budget.
    ///
    /// Handles are not reused across walks: an `AXUIElement` outlives the window it refers to and
    /// keeping the table would let a stale node be read as a live one.
    public func focusedRoot() -> AXNodeID? {
        lock.lock()
        elements.removeAll()
        identifiers.removeAll()
        deadline = now().addingTimeInterval(Self.walkBudget)
        lock.unlock()

        guard let application = NSWorkspace.shared.frontmostApplication else { return nil }
        let applicationElement = AXUIElementCreateApplication(application.processIdentifier)

        // Applies to every subsequent message to this application, so it is set once here rather
        // than per element. See AXUIElementSetMessagingTimeout.
        _ = AXUIElementSetMessagingTimeout(applicationElement, Self.messagingTimeout)
        // Ignored by everything that is not Chromium, and Chromium takes a moment to build the
        // tree afterwards — the first walk against a browser can legitimately come back thin.
        if setsManualAccessibility {
            _ = AXUIElementSetAttributeValue(
                applicationElement, Self.manualAccessibilityAttribute as CFString, kCFBooleanTrue)
        }

        let root = copyElement(from: applicationElement, attribute: kAXFocusedWindowAttribute)
            ?? applicationElement
        return identifier(for: root)
    }

    public func children(of node: AXNodeID) -> [AXNodeID] {
        guard let element = self.element(for: node), !budgetSpent else { return [] }

        var value: CFTypeRef?
        guard
            AXUIElementCopyAttributeValue(element, kAXChildrenAttribute as CFString, &value)
                == .success,
            let children = value as? [AXUIElement]
        else { return [] }

        return children.map(identifier(for:))
    }

    public func text(of node: AXNodeID) -> [String] {
        guard let element = self.element(for: node), !budgetSpent else { return [] }

        // Value first: on a static-text element it is the text itself, and on a container it is
        // usually absent, so this order puts content ahead of chrome.
        return [kAXValueAttribute, kAXTitleAttribute, kAXDescriptionAttribute]
            .compactMap { copyString(from: element, attribute: $0) }
    }

    // MARK: Handles

    /// `AXUIElement` is a CoreFoundation type: it has no useful Swift identity, and two references
    /// to the same element are distinct objects. `CFEqual`/`CFHash` are the comparison the AX API
    /// documents, so the cycle detection above is keyed on them.
    private struct ElementKey: Hashable {
        let element: AXUIElement

        static func == (lhs: ElementKey, rhs: ElementKey) -> Bool {
            CFEqual(lhs.element, rhs.element)
        }

        func hash(into hasher: inout Hasher) {
            hasher.combine(CFHash(element))
        }
    }

    private func identifier(for element: AXUIElement) -> AXNodeID {
        lock.lock()
        defer { lock.unlock() }

        let key = ElementKey(element: element)
        if let existing = identifiers[key] { return existing }

        let identifier = AXNodeID(nextIdentifier)
        nextIdentifier += 1
        identifiers[key] = identifier
        elements[identifier] = element
        return identifier
    }

    private func element(for node: AXNodeID) -> AXUIElement? {
        lock.lock()
        defer { lock.unlock() }
        return elements[node]
    }

    private var budgetSpent: Bool {
        lock.lock()
        defer { lock.unlock() }
        guard let deadline else { return true }
        return now() >= deadline
    }

    // MARK: Attribute reads

    private func copyElement(from element: AXUIElement, attribute: String) -> AXUIElement? {
        var value: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, attribute as CFString, &value) == .success,
            let value, CFGetTypeID(value) == AXUIElementGetTypeID()
        else { return nil }
        return (value as! AXUIElement)
    }

    private func copyString(from element: AXUIElement, attribute: String) -> String? {
        var value: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, attribute as CFString, &value) == .success,
            let string = value as? String
        else { return nil }
        return string
    }
}

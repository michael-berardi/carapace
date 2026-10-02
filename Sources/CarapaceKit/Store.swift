import Foundation
import SwiftUI

/// Coalesces notifications from the core thread into one main-thread hop.
/// State keeps only the latest snapshot; events and faults are never dropped.
final class Mailbox: @unchecked Sendable {
    private let lock = NSLock()
    private var state: Data?
    private var events: [Data] = []
    private var faults: [String] = []
    private var scheduled = false

    /// Returns true when the caller must schedule a drain.
    func push(_ notice: CarapaceNotice) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        switch notice {
        case .state(let data): state = data
        case .event(let data): events.append(data)
        case .fault(let text): faults.append(text)
        }
        if scheduled { return false }
        scheduled = true
        return true
    }

    func take() -> (state: Data?, events: [Data], faults: [String]) {
        lock.lock()
        defer { lock.unlock() }
        defer { state = nil; events = []; faults = []; scheduled = false }
        return (state, events, faults)
    }
}

/// Stops the core when the store goes away.
final class CoreOwner: Sendable {
    let core: any CarapaceCore
    let subscription: Int
    init(core: any CarapaceCore, subscription: Int) {
        self.core = core
        self.subscription = subscription
    }
    deinit {
        core.unsubscribe(subscription)
        core.stop()
    }
}

/// The observable bridge between a Rust core and SwiftUI.
///
/// `state` is a value snapshot the views render; `send` forwards actions to the core.
/// Errors are never swallowed: they land in `faults` and in `onFault`.
@MainActor
public final class Store<App: CarapaceApp>: ObservableObject {
    @Published public private(set) var state: App.State
    /// Messages from the core: panics, undecodable actions, cycles. Newest last.
    @Published public private(set) var faults: [String] = []
    /// Called on the main actor for every platform request the core makes.
    /// Events that arrive before a handler is set are held and delivered when it is.
    public var onEvent: ((App.Event) -> Void)? {
        didSet { flushEvents() }
    }
    public var onFault: ((String) -> Void)?

    private let backend: any CarapaceBackend
    private let owner: CoreOwner
    private let mailbox: Mailbox
    private var heldEvents: [App.Event] = []
    private var flushing = false
    private let decoder = JSONDecoder()
    private let encoder = JSONEncoder()

    public init(backend: any CarapaceBackend, config: App.Config? = nil) throws {
        guard backend.abiVersion == CarapaceKitABI else {
            throw CarapaceError.abiMismatch(expected: CarapaceKitABI, found: backend.abiVersion)
        }
        guard backend.schemaHash == App.schemaHash else {
            throw CarapaceError.staleBindings(app: App.name, expected: App.schemaHash, found: backend.schemaHash)
        }
        let configData = try config.map { try JSONEncoder().encode($0) }
        let core = try backend.start(config: configData)
        let mailbox = Mailbox()
        // Subscribe before reading the snapshot so nothing falls between the two.
        let box = WeakBox<Store<App>>()
        let id = core.subscribe { notice in
            if mailbox.push(notice) {
                DispatchQueue.main.async {
                    MainActor.assumeIsolated { box.value?.drain() }
                }
            }
        }
        self.backend = backend
        self.owner = CoreOwner(core: core, subscription: id)
        self.mailbox = mailbox
        self.state = try decoder.decode(App.State.self, from: try core.snapshot())
        box.value = self
        // Events replayed to the subscription above are already queued: deliver them now. Queued
        // state notices may be older than the snapshot just read, so ignore them and read once more.
        drain(applyState: false)
        refreshFromSnapshot()
    }

    /// Send an action to the core. Never blocks; the new state arrives via `state`.
    public func send(_ action: App.Action) {
        do {
            try owner.core.dispatch(try encoder.encode(action))
        } catch {
            report("could not send \(action): \(error.localizedDescription)")
        }
    }

    /// Send an action and apply its result before returning. Use it where the new state must
    /// land inside the caller's context, for example inside `withAnimation`, so views animate
    /// from the old state to the new one. The round trip is microseconds; prefer `send` for
    /// anything that does not need it.
    public func sendSync(_ action: App.Action) {
        do {
            try owner.core.dispatchAndWait(try encoder.encode(action))
            drain()
        } catch {
            report("could not send \(action): \(error.localizedDescription)")
        }
    }

    /// Run one of the core's pure queries (colour math, parsing, formatting). Synchronous and
    /// stateless: it does not touch the core's state and is safe to call every frame.
    public func query(_ query: App.Query) throws -> App.Answer {
        do {
            return try decoder.decode(App.Answer.self, from: try backend.query(try encoder.encode(query)))
        } catch let error as CarapaceError {
            throw error
        } catch {
            throw CarapaceError.decoding("could not run query \(query): \(error)")
        }
    }

    /// A two-way binding for controls: reads `state`, writes by sending an action.
    public func binding<Value>(_ keyPath: KeyPath<App.State, Value>, send make: @escaping (Value) -> App.Action) -> Binding<Value> {
        Binding(get: { self.state[keyPath: keyPath] }, set: { self.send(make($0)) })
    }

    private func refreshFromSnapshot() {
        do {
            let next = try decoder.decode(App.State.self, from: try owner.core.snapshot())
            state = next
        } catch {
            report("could not read the core's state: \(error)")
        }
    }

    private func drain(applyState: Bool = true) {
        let batch = mailbox.take()
        if applyState, let data = batch.state {
            do { state = try decoder.decode(App.State.self, from: data) }
            catch { report("core sent a state this app cannot decode: \(error)") }
        }
        for data in batch.events {
            do { heldEvents.append(try decoder.decode(App.Event.self, from: data)) }
            catch { report("core sent an event this app cannot decode: \(error)") }
        }
        for fault in batch.faults { report(fault) }
        flushEvents()
    }

    /// Delivers held events in order. A handler that calls `sendSync` re-enters `drain`; the
    /// outer loop picks up what that adds, so events are never delivered out of order.
    private func flushEvents() {
        guard let handler = onEvent, !flushing else { return }
        flushing = true
        defer { flushing = false }
        while !heldEvents.isEmpty {
            handler(heldEvents.removeFirst())
        }
    }

    private func report(_ message: String) {
        faults.append(message)
        if faults.count > 50 { faults.removeFirst(faults.count - 50) }
        onFault?(message)
    }
}

/// The ABI version this CarapaceKit speaks.
public let CarapaceKitABI: UInt32 = 1

private final class WeakBox<T: AnyObject>: @unchecked Sendable {
    weak var value: T?
}

import AppKit
import CoreGraphics
import Darwin

let args = CommandLine.arguments
func argument(_ key: String) -> String? {
    guard let i = args.firstIndex(of: key), i + 1 < args.count else { return nil }
    return args[i + 1]
}
func canonical(_ value: String) -> String {
    // Resolve aliases such as /var and /private/var to the same path.
    return value.withCString { input in
        guard let resolved = realpath(input, nil) else { return value }
        defer { free(resolved) }
        return String(cString: resolved)
    }
}
guard let allow = argument("--allow"), allow.hasPrefix("/"),
      let parentText = argument("--parent-pid"), let parent = Int32(parentText),
      parent > 1, parent == getppid() else { exit(2) }
let parentPid = parent
let allowed = canonical(allow)
let allowedName = URL(fileURLWithPath: allowed).lastPathComponent
let allowedBundle = argument("--bundle").map(canonical)
signal(SIGPIPE, SIG_IGN)
let application = NSApplication.shared
application.setActivationPolicy(.prohibited)
let dateFormatter = ISO8601DateFormatter()
dateFormatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
var stopped = false
var processes: [pid_t: [String: Any]] = [:]
var windows: [String: [String: Any]] = [:]
var observedCGWindowBounds: [String: CGRect] = [:]
var metadataMovingUntil: TimeInterval = 0
var workspaceTokens: [NSObjectProtocol] = []
var sources: [DispatchSourceSignal] = []
var ticker: Timer?
var lastScan = Date.distantPast
var ready = false
var scanning = false
var ownedWindowIds = Set<UInt32>()
var controlBuffer = Data()

@MainActor func emit(_ object: [String: Any]) {
    guard !stopped, JSONSerialization.isValidJSONObject(object),
          var data = try? JSONSerialization.data(withJSONObject: object, options: [.sortedKeys]) else { return }
    data.append(10)
    do { try FileHandle.standardOutput.write(contentsOf: data) } catch { exit(0) }
}
@Sendable func macMetadataScanInterval(_ active: Bool, _ now: TimeInterval, _ movingUntil: TimeInterval) -> TimeInterval {
    return active && now < movingUntil ? 1.0 / 30.0 : (active ? 0.1 : 0.25)
}
@MainActor func metadata(_ app: NSRunningApplication) -> [String: Any]? {
    guard let executable = app.executableURL, executable.lastPathComponent == allowedName, canonical(executable.path) == allowed,
          allowedBundle == nil || app.bundleURL.map({ canonical($0.path) }) == allowedBundle,
          !app.isTerminated else { return nil }
    // launchDate may be nil for a legitimate app started directly from its
    // executable. Kernel creation time remains stable before Launch Services
    // registration and prevents PID reuse from replaying or attaching effects.
    var info = proc_bsdinfo()
    let length = MemoryLayout<proc_bsdinfo>.size
    let count = withUnsafeMutablePointer(to: &info) {
        proc_pidinfo(app.processIdentifier, PROC_PIDTBSDINFO, 0, $0, Int32(length))
    }
    guard count == Int32(length) else { return nil }
    let launched = Date(timeIntervalSince1970: Double(info.pbi_start_tvsec) + Double(info.pbi_start_tvusec) / 1_000_000)
    return ["pid": Int(app.processIdentifier), "executablePath": allowed,
            "processStartedAt": dateFormatter.string(from: launched)]
}
func dictionary(_ rect: CGRect) -> [String: Int] {
    return ["x": Int(rect.origin.x.rounded()), "y": Int(rect.origin.y.rounded()),
            "width": Int(rect.width.rounded()), "height": Int(rect.height.rounded())]
}
func subtract(_ source: CGRect, _ obstruction: CGRect) -> [CGRect] {
    let intersection = source.intersection(obstruction)
    if intersection.isNull || intersection.isEmpty { return [source] }
    var result: [CGRect] = []
    if intersection.minY > source.minY { result.append(CGRect(x: source.minX, y: source.minY, width: source.width, height: intersection.minY - source.minY)) }
    if intersection.maxY < source.maxY { result.append(CGRect(x: source.minX, y: intersection.maxY, width: source.width, height: source.maxY - intersection.maxY)) }
    if intersection.minX > source.minX { result.append(CGRect(x: source.minX, y: intersection.minY, width: intersection.minX - source.minX, height: intersection.height)) }
    if intersection.maxX < source.maxX { result.append(CGRect(x: intersection.maxX, y: intersection.minY, width: source.maxX - intersection.maxX, height: intersection.height)) }
    return result.filter { $0.width >= 1 && $0.height >= 1 }
}
@MainActor enum Observer {
    static func stop() {
        if stopped { return }
        ticker?.invalidate()
        for token in workspaceTokens { NSWorkspace.shared.notificationCenter.removeObserver(token) }
        FileHandle.standardInput.readabilityHandler = nil
        emit(["type": "STOP"])
        stopped = true
        exit(0)
    }
    static func scan() {
        if stopped || scanning { return }
        scanning = true
        defer { scanning = false }
        lastScan = Date()
        let runningApps = NSWorkspace.shared.runningApplications
        let candidates = runningApps.compactMap { app in metadata(app).map { (app, $0) } }
        let regularApplications = Set(runningApps.filter {
            $0.activationPolicy == .regular && !$0.isHidden && !$0.isTerminated
        }.map { $0.processIdentifier })
        var nextProcesses: [pid_t: [String: Any]] = [:]
        var targetApplications: [pid_t: NSRunningApplication] = [:]
        for (app, value) in candidates {
            var identity = value
            let pid = app.processIdentifier
            targetApplications[pid] = app
            nextProcesses[pid] = identity
            if processes[pid]?["processStartedAt"] as? String != identity["processStartedAt"] as? String {
                identity["type"] = "process"; identity["eventName"] = ready ? "CREATE" : "BASELINE"
                emit(identity)
            }
        }
        for (pid, identity) in processes where nextProcesses[pid]?["processStartedAt"] as? String != identity["processStartedAt"] as? String {
            var event = identity; event["type"] = "process"; event["eventName"] = "EXIT"; emit(event)
        }
        processes = nextProcesses
        if processes.isEmpty {
            emitWindowSnapshot(destroyedWindows(except: [:]))
            windows.removeAll()
            observedCGWindowBounds.removeAll()
            return
        }
        guard let allWindows = CGWindowListCopyWindowInfo([.optionAll, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]],
              let onScreenWindows = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] else {
            emit(["type": "error", "code": "mac_window_metadata_unavailable"]); return
        }
        // optionAll also includes other Spaces and ordered-out windows, and its
        // returned order need not match the current on-screen stacking order.
        // Use the on-screen snapshot for occlusion and append hidden records only
        // to retain their lifecycle/geometry until they become visible again.
        let onScreenWindowIds = Set(onScreenWindows.compactMap { $0[kCGWindowNumber as String] as? UInt32 })
        let list = onScreenWindows + allWindows.filter {
            guard let number = $0[kCGWindowNumber as String] as? UInt32 else { return false }
            return !onScreenWindowIds.contains(number)
        }
        let loginSession = CGSessionCopyCurrentDictionary() as? [String: Any] ?? [:]
        let sessionVisible = loginSession["CGSSessionScreenIsLocked"] as? Bool != true &&
            loginSession["kCGSSessionOnConsoleKey"] as? Bool != false
        let front = sessionVisible ? NSWorkspace.shared.frontmostApplication?.processIdentifier : nil
        var nextWindows: [String: [String: Any]] = [:]
        var nextCGWindowBounds: [String: CGRect] = [:]
        var foregroundChosen = Set<pid_t>()
        var changes: [[String: Any]] = []
        for (index, info) in list.enumerated() {
            guard let number = info[kCGWindowNumber as String] as? UInt32,
                  let owner = info[kCGWindowOwnerPID as String] as? Int32,
                  let identity = processes[owner], let app = targetApplications[owner],
                  let layer = info[kCGWindowLayer as String] as? Int, layer == 0,
                  let rawBounds = info[kCGWindowBounds as String] as? NSDictionary,
                  let bounds = CGRect(dictionaryRepresentation: rawBounds),
                  bounds.width >= 100, bounds.height >= 100,
                  [bounds.minX,bounds.minY,bounds.width,bounds.height].allSatisfy({ $0.isFinite && abs($0) < 100000 }) else { continue }
            let onScreen = onScreenWindowIds.contains(number)
            let key = "\(owner):\(identity["processStartedAt"]!):\(number)"
            let visible = sessionVisible && onScreen && !app.isHidden &&
                (info[kCGWindowAlpha as String] as? Double ?? 1) > 0
            nextCGWindowBounds[key] = bounds
            if visible, let previous = observedCGWindowBounds[key], previous != bounds {
                // Only sustained geometry changes keep the short high-rate phase
                // alive. An idle moved window does not stay at 30 Hz indefinitely.
                metadataMovingUntil = ProcessInfo.processInfo.systemUptime + 0.25
            }
            let focused = visible && front == owner && !foregroundChosen.contains(owner)
            if focused { foregroundChosen.insert(owner) }
            var parts = visible ? [bounds] : []
            if !parts.isEmpty {
                for higher in list.prefix(index) {
                    guard let pid = higher[kCGWindowOwnerPID as String] as? Int32, pid != getpid(),
                          let higherId = higher[kCGWindowNumber as String] as? UInt32,
                          !(pid == parentPid && ownedWindowIds.contains(higherId)),
                          pid == owner || regularApplications.contains(pid),
                          higher[kCGWindowIsOnscreen as String] as? Bool == true,
                          (higher[kCGWindowLayer as String] as? Int ?? -1) == 0,
                          (higher[kCGWindowAlpha as String] as? Double ?? 1) > 0,
                          let raw = higher[kCGWindowBounds as String] as? NSDictionary,
                          let obstruction = CGRect(dictionaryRepresentation: raw),
                          obstruction.width > 0, obstruction.height > 0,
                          [obstruction.minX,obstruction.minY,obstruction.width,obstruction.height].allSatisfy({ $0.isFinite && abs($0) < 100000 }) else { continue }
                    // Normal windows belonging to other regular applications can
                    // cover an unfocused target too. Exclude only the exact Prism
                    // overlay IDs; its manager window remains a real obstruction.
                    parts = parts.flatMap { subtract($0, obstruction) }
                    if parts.count > 64 { parts = []; break }
                }
            }
            var event = identity
            event["type"] = "window"; event["windowId"] = String(number)
            event["bounds"] = dictionary(bounds); event["coordinateSpace"] = "dip"
            event["visible"] = visible
            event["frontmost"] = front == owner; event["focused"] = focused
            event["zOrder"] = index
            event["overlayEligible"] = !parts.isEmpty
            event["clipRects"] = parts.map { dictionary($0.offsetBy(dx: -bounds.minX, dy: -bounds.minY)) }
            let previous = windows[key]
            nextWindows[key] = event
            if previous == nil || !NSDictionary(dictionary: previous!).isEqual(to: event) {
                var output = event
                output["eventName"] = previous == nil ? "SHOW" : "LOCATIONCHANGE"
                changes.append(output)
            }
        }
        changes.append(contentsOf: destroyedWindows(except: nextWindows))
        emitWindowSnapshot(changes)
        windows = nextWindows
        observedCGWindowBounds = nextCGWindowBounds
    }

    static func destroyedWindows(except present: [String: [String: Any]]) -> [[String: Any]] {
        var changes: [[String: Any]] = []
        for (key, old) in windows where present[key] == nil {
            var event = old
            event["eventName"] = "DESTROY"
            event["visible"] = false
            event["focused"] = false
            event["overlayEligible"] = false
            event["clipRects"] = []
            changes.append(event)
        }
        return changes
    }

    static func emitWindowSnapshot(_ changes: [[String: Any]]) {
        guard !changes.isEmpty else { return }
        emit(["type": "WINDOW_SNAPSHOT_BEGIN"])
        for event in changes { emit(event) }
        emit(["type": "WINDOW_SNAPSHOT_END"])
    }
}

let center = NSWorkspace.shared.notificationCenter
for name in [NSWorkspace.didLaunchApplicationNotification, NSWorkspace.didTerminateApplicationNotification,
             NSWorkspace.didActivateApplicationNotification, NSWorkspace.didDeactivateApplicationNotification,
             NSWorkspace.didHideApplicationNotification, NSWorkspace.didUnhideApplicationNotification,
             NSWorkspace.sessionDidResignActiveNotification, NSWorkspace.sessionDidBecomeActiveNotification,
             NSWorkspace.activeSpaceDidChangeNotification] {
    workspaceTokens.append(center.addObserver(forName: name, object: nil, queue: .main) { _ in
        MainActor.assumeIsolated { Observer.scan() }
    })
}
MainActor.assumeIsolated {
    Observer.scan()
    emit(["type": "READY", "protocolVersion": 1, "available": true, "baseline": Array(processes.values)])
    ready = true
}
FileHandle.standardInput.readabilityHandler = { handle in
    let data = handle.availableData
    DispatchQueue.main.async {
        if data.isEmpty { Observer.stop(); return }
        controlBuffer.append(data)
        if controlBuffer.count > 65536 { Observer.stop(); return }
        while let newline = controlBuffer.firstIndex(of: 10) {
            let line = controlBuffer.prefix(upTo: newline)
            controlBuffer.removeSubrange(...newline)
            if String(data: line, encoding: .utf8)?.trimmingCharacters(in: .whitespacesAndNewlines) == "stop" { Observer.stop(); return }
            guard let object = try? JSONSerialization.jsonObject(with: line) as? [String: Any] else { continue }
            guard object["type"] as? String == "OWNED_WINDOWS",
                  let values = object["windowIds"] as? [String], values.count <= 256,
                  values.allSatisfy({ !$0.isEmpty && $0.count <= 10 && $0.allSatisfy({ $0.isASCII && $0.isNumber }) && UInt32($0).map({ $0 > 0 }) == true }) else { continue }
            let next = Set(values.compactMap(UInt32.init))
            if next != ownedWindowIds {
                ownedWindowIds = next
                Observer.scan()
            }
        }
    }
}
for signalNumber in [SIGTERM, SIGINT] {
    signal(signalNumber, SIG_IGN)
    let source = DispatchSource.makeSignalSource(signal: signalNumber, queue: .main)
    source.setEventHandler { MainActor.assumeIsolated { Observer.stop() } }
    source.resume()
    sources.append(source)
}
ticker = Timer.scheduledTimer(withTimeInterval: 1.0 / 30.0, repeats: true) { _ in
    MainActor.assumeIsolated {
        if getppid() != parentPid { Observer.stop() }
        let active = windows.values.contains { $0["visible"] as? Bool == true }
        let elapsed = Date().timeIntervalSince(lastScan)
        let interval = macMetadataScanInterval(active, ProcessInfo.processInfo.systemUptime, metadataMovingUntil)
        if elapsed + 0.002 >= interval { Observer.scan() }
    }
}
// AppKit must service its application event queue as well as CFRunLoop sources;
// a bare Foundation run loop leaves NSWorkspace's running-app cache stale.
application.run()

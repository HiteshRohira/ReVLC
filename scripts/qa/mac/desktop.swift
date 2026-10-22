// Small CGEvent helper for QA runs; see docs/audit-playbook.md.
// Compile once:  swiftc -O scripts/qa/mac/desktop.swift -o "$QA_RUN/bin/desktop"
//
//   desktop windowid            ID of ReVLC's main window (for screencapture -l)
//   desktop windows             every on-screen ReVLC window, with bounds
//   desktop click X Y           left click at screen point
//   desktop move X Y            move the pointer
//   desktop key CODE [COUNT]    hold a key: COUNT key-downs, the repeats marked
//                               as autorepeat, then one key-up (49 Space,
//                               3 F, 46 M, 124 Right, 126 Up)
//   desktop drag X1 Y1 X2 Y2    press, drag in 30 steps, release
import AppKit
import CoreGraphics

func revlcWindows() -> [[String: Any]] {
    let all = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as! [[String: Any]]
    return all.filter { ($0[kCGWindowOwnerName as String] as? String ?? "").lowercased().contains("revlc") }
}

func post(_ type: CGEventType, _ p: CGPoint) {
    CGEvent(mouseEventSource: nil, mouseType: type, mouseCursorPosition: p, mouseButton: .left)?.post(tap: .cghidEventTap)
}

func point(_ x: String, _ y: String) -> CGPoint { CGPoint(x: Double(x)!, y: Double(y)!) }

let args = CommandLine.arguments
guard args.count >= 2 else { exit(1) }
switch args[1] {
case "windowid":
    if let w = revlcWindows().first(where: { ($0[kCGWindowLayer as String] as? Int ?? -1) == 0 }) {
        print(w[kCGWindowNumber as String]!)
    }
case "windows":
    for w in revlcWindows() { print(w) }
case "click":
    let p = point(args[2], args[3])
    post(.mouseMoved, p)
    post(.leftMouseDown, p)
    post(.leftMouseUp, p)
case "move":
    post(.mouseMoved, point(args[2], args[3]))
case "key":
    let code = CGKeyCode(args[2])!
    let count = args.count > 3 ? Int(args[3])! : 1
    for i in 0..<count {
        let e = CGEvent(keyboardEventSource: nil, virtualKey: code, keyDown: true)!
        if i > 0 { e.setIntegerValueField(.keyboardEventAutorepeat, value: 1) }
        e.post(tap: .cghidEventTap)
        Thread.sleep(forTimeInterval: 0.12)
    }
    CGEvent(keyboardEventSource: nil, virtualKey: code, keyDown: false)?.post(tap: .cghidEventTap)
case "drag":
    let p = point(args[2], args[3]), q = point(args[4], args[5])
    post(.leftMouseDown, p)
    for i in 1...30 {
        let f = Double(i) / 30
        post(.leftMouseDragged, CGPoint(x: p.x + (q.x - p.x) * f, y: p.y + (q.y - p.y) * f))
        Thread.sleep(forTimeInterval: 0.025)
    }
    post(.leftMouseUp, q)
default:
    exit(1)
}

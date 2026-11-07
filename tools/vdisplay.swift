// items.id=720 (THROWAWAY): creates ONE virtual display with a 2x backing scale for the Retina instrumentation pass,
// and keeps it alive until this process is stopped (Ctrl-C / kill). Uses the private CGVirtualDisplay classes, loaded
// dynamically. Installs nothing, changes no system setting; the display disappears when the process exits.
//   swiftc -O -o tools/out/vdisplay tools/vdisplay.swift && tools/out/vdisplay
// COMPROMISE: a virtual display is not a real Retina panel; results from it are instrumentation only.
import AppKit
import Foundation

func cls(_ n: String) -> NSObject.Type {
    guard let c = NSClassFromString(n) as? NSObject.Type else { fatalError("private class \(n) not available on this macOS") }
    return c
}

let desc = cls("CGVirtualDisplayDescriptor").init()
desc.setValue("Spike720 virtual 2x", forKey: "name")
desc.setValue(3840, forKey: "maxPixelsWide")
desc.setValue(2160, forKey: "maxPixelsHigh")
desc.setValue(NSValue(size: NSSize(width: 600, height: 340)), forKey: "sizeInMillimeters")
desc.setValue(0x720, forKey: "vendorID")
desc.setValue(0x720, forKey: "productID")
desc.setValue(1, forKey: "serialNum")
desc.setValue(DispatchQueue.main, forKey: "queue")

// -[CGVirtualDisplayMode initWithWidth:height:refreshRate:]
typealias InitMode = @convention(c) (AnyObject, Selector, UInt, UInt, Double) -> AnyObject
let modeCls = cls("CGVirtualDisplayMode")
let modeAlloc = modeCls.perform(Selector(("alloc")))!.takeUnretainedValue()
let initSel = Selector(("initWithWidth:height:refreshRate:"))
let initImp = unsafeBitCast(modeCls.instanceMethod(for: initSel), to: InitMode.self)
let mode = initImp(modeAlloc, initSel, 1920, 1080, 60.0)

let settings = cls("CGVirtualDisplaySettings").init()
settings.setValue(1, forKey: "hiDPI")
settings.setValue([mode], forKey: "modes")

let dispCls = cls("CGVirtualDisplay")
let dispAlloc = dispCls.perform(Selector(("alloc")))!.takeUnretainedValue()
let display = dispAlloc.perform(Selector(("initWithDescriptor:")), with: desc)!.takeUnretainedValue()
typealias Apply = @convention(c) (AnyObject, Selector, AnyObject) -> Bool
let applySel = Selector(("applySettings:"))
let ok = unsafeBitCast(display.method(for: applySel), to: Apply.self)(display, applySel, settings)
let id = (display.value(forKey: "displayID") as? UInt32) ?? 0
print("virtual display created: displayID=\(id) applySettings=\(ok)")

DispatchQueue.main.asyncAfter(deadline: .now() + 1.0) {
    for (i, s) in NSScreen.screens.enumerated() {
        print("screen \(i): \(s.localizedName) frame=\(s.frame) backingScaleFactor=\(s.backingScaleFactor)")
    }
    fflush(stdout)
}
// keep `display` alive and the run loop spinning
withExtendedLifetime(display) { RunLoop.main.run() }

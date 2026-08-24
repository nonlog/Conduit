package com.conduit.sync

import de.robv.android.xposed.IXposedHookLoadPackage
import de.robv.android.xposed.XC_MethodHook
import de.robv.android.xposed.XposedBridge
import de.robv.android.xposed.callbacks.XC_LoadPackage

/** Hardcoded rather than read from `BuildConfig`, which would mean generating one. */
private const val PACKAGE = "com.conduit.sync"

/**
 * Lets this app read the clipboard while it is in the background.
 *
 * On stock Android 10 and later `ClipboardService.clipboardAccessAllowed` refuses any
 * caller that is neither focused nor the current input method — and it refuses *before*
 * consulting app-ops, so no amount of `appops set` helps. The change listener is gated on
 * the same method, so without this the app is not merely unable to read a clip: it is
 * never told one happened. AOSP's own escape hatch,
 * `android.permission.READ_CLIPBOARD_IN_BACKGROUND`, is `signature|privileged` and would
 * mean shipping the app into `/system/priv-app`.
 *
 * Two deliberate narrowings, because this hook runs inside system_server:
 *  - it only forces the result when this package's own name is among the arguments, so
 *    every other app on the device stays subject to the normal check;
 *  - it matches on the *name* `clipboardAccessAllowed`, never a signature. That method
 *    gained a `userId` in 11, a `shouldNoteOp` in 12 and an `attributionTag` plus a
 *    `deviceId` in 13, so pinning a parameter list would break on the next upgrade.
 *
 * Scope in LSPosed: **System Framework** only.
 */
class ClipboardHook : IXposedHookLoadPackage {

    override fun handleLoadPackage(param: XC_LoadPackage.LoadPackageParam) {
        if (param.packageName != "android") return

        val service = runCatching {
            param.classLoader.loadClass("com.android.server.clipboard.ClipboardService")
        }.getOrElse {
            XposedBridge.log("conduit: no ClipboardService on this build: $it")
            return
        }

        val allow = object : XC_MethodHook() {
            override fun beforeHookedMethod(call: MethodHookParam) {
                if (call.args.any { it == PACKAGE }) {
                    // Short-circuits the method: the real check never runs for us.
                    call.result = true
                }
            }
        }

        var hooked = 0
        for (method in service.declaredMethods) {
            if (method.name == "clipboardAccessAllowed") {
                XposedBridge.hookMethod(method, allow)
                hooked++
            }
        }
        XposedBridge.log("conduit: hooked $hooked clipboardAccessAllowed overload(s)")
    }
}

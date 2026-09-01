package com.conduit.sync

import de.robv.android.xposed.IXposedHookLoadPackage
import de.robv.android.xposed.XC_MethodHook
import de.robv.android.xposed.XposedBridge
import de.robv.android.xposed.callbacks.XC_LoadPackage

/** Hardcoded rather than read from `BuildConfig`, which would mean generating one. */
private const val PACKAGE = "com.conduit.sync"

/**
 * Lets Conduit read the clipboard while it is in the background and exposes whether that root path
 * is actually active to Conduit's own process.
 *
 * On stock Android 10 and later `ClipboardService.clipboardAccessAllowed` refuses any caller that
 * is neither focused nor the current input method. The System Framework hook keeps the original
 * narrow behaviour: only Conduit's package is allowed and every other app still follows Android's
 * normal clipboard policy.
 *
 * The Conduit-app scope does not alter clipboard APIs. It only hooks the inert
 * [ClipboardAccess.isLsposedActive] marker to return true. That gives Settings and the
 * AccessibilityService a reliable way to prefer the low-overhead LSPosed path and leave the
 * accessibility compatibility path dormant on rooted devices.
 *
 * Scope in LSPosed: **System Framework + Conduit**.
 */
class ClipboardHook : IXposedHookLoadPackage {

    override fun handleLoadPackage(param: XC_LoadPackage.LoadPackageParam) {
        when (param.packageName) {
            PACKAGE -> exposeModuleState(param)
            "android" -> hookClipboardService(param)
        }
    }

    private fun exposeModuleState(param: XC_LoadPackage.LoadPackageParam) {
        val marker = runCatching {
            param.classLoader
                .loadClass("com.conduit.sync.ClipboardAccess")
                .getDeclaredMethod("isLsposedActive")
        }.getOrElse {
            XposedBridge.log("conduit: could not expose LSPosed state to app process: $it")
            return
        }
        XposedBridge.hookMethod(
            marker,
            object : XC_MethodHook() {
                override fun beforeHookedMethod(call: MethodHookParam) {
                    call.result = true
                }
            },
        )
        XposedBridge.log("conduit: LSPosed app-process marker active")
    }

    private fun hookClipboardService(param: XC_LoadPackage.LoadPackageParam) {
        val service = runCatching {
            param.classLoader.loadClass("com.android.server.clipboard.ClipboardService")
        }.getOrElse {
            XposedBridge.log("conduit: no ClipboardService on this build: $it")
            return
        }

        val allow = object : XC_MethodHook() {
            override fun beforeHookedMethod(call: MethodHookParam) {
                if (call.args.any { it == PACKAGE }) {
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

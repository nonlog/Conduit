package com.conduit.sync

import android.content.Context
import android.os.Binder
import android.os.IBinder
import android.os.IInterface
import android.os.Parcel
import android.util.Log
import de.robv.android.xposed.IXposedHookLoadPackage
import de.robv.android.xposed.XC_MethodHook
import de.robv.android.xposed.XposedBridge
import de.robv.android.xposed.XposedHelpers
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
        hookConnectedProtection(param)
    }

    /** SyncService registers this listener only while an authenticated desktop is connected. */
    private fun hookConnectedProtection(param: XC_LoadPackage.LoadPackageParam) {
        runCatching {
            val protection = OplusConnectionProtection(param.classLoader)
            val listeners = ConnectedProtectionListeners<IBinder>(protection::acquire, protection::release)
            val service = param.classLoader.loadClass("com.android.server.clipboard.ClipboardService\$ClipboardImpl")
            var hooked = 0
            for (method in service.declaredMethods) {
                if (method.name != "addPrimaryClipChangedListener" &&
                    method.name != "removePrimaryClipChangedListener") continue
                if (method.parameterTypes.size < 2 || method.parameterTypes[1] != String::class.java ||
                    !IInterface::class.java.isAssignableFrom(method.parameterTypes[0])) continue
                val adding = method.name == "addPrimaryClipChangedListener"
                XposedBridge.hookMethod(method, object : XC_MethodHook() {
                    override fun afterHookedMethod(call: MethodHookParam) {
                        if (call.hasThrowable() || call.args.getOrNull(1) != PACKAGE) return
                        runCatching {
                            val uid = Binder.getCallingUid()
                            val outer = XposedHelpers.getObjectField(call.thisObject, "this\$0")
                            val context = XposedHelpers.callMethod(outer, "getContext") as Context
                            // Do not trust a caller-supplied package name, including attribution tags.
                            val packageUid = context.packageManager.getPackageUid(PACKAGE, 0)
                            if (uid != packageUid) return
                            val listener = (call.args.firstOrNull() as? IInterface)?.asBinder() ?: return
                            synchronized(listeners) {
                                if (!adding) {
                                    listeners.remove(listener, uid)
                                    return
                                }
                                val death = IBinder.DeathRecipient {
                                    runCatching { listeners.remove(listener, uid) }
                                        .onFailure { Log.w("conduit.freeze", "client death cleanup failed", it) }
                                }
                                listener.linkToDeath(death, 0)
                                try {
                                    if (!listeners.add(listener, uid, packageUid) { listener.unlinkToDeath(death, 0) }) {
                                        listener.unlinkToDeath(death, 0)
                                    } else if (!listener.isBinderAlive) {
                                        listeners.remove(listener, uid)
                                    }
                                } catch (error: Throwable) {
                                    listener.unlinkToDeath(death, 0)
                                    throw error
                                }
                            }
                        }.onFailure { Log.w("conduit.freeze", "optional connected protection unavailable", it) }
                    }
                })
                hooked++
            }
            XposedBridge.log("conduit: hooked $hooked connected ColorOS protection method(s)")
        }.onFailure {
            XposedBridge.log("conduit: optional ColorOS protection not supported: $it")
        }
    }
}

/** Binder tokens share one lease per UID. Removal and Binder death use the same cleanup path. */
internal class ConnectedProtectionListeners<T>(
    private val acquire: (Int) -> Unit,
    private val release: (Int) -> Unit,
) {
    private class Listener(val uid: Int, val unlink: () -> Unit)
    private val listeners = mutableMapOf<T, Listener>()

    @Synchronized
    fun add(token: T, callingUid: Int, packageUid: Int, unlink: () -> Unit): Boolean {
        if (callingUid < 0 || callingUid != packageUid || listeners.containsKey(token)) return false
        if (listeners.values.none { it.uid == callingUid }) acquire(callingUid)
        listeners[token] = Listener(callingUid, unlink)
        return true
    }

    @Synchronized
    fun remove(token: T, callingUid: Int) {
        val listener = listeners[token] ?: return
        if (listener.uid != callingUid) return
        listeners.remove(token)
        try {
            listener.unlink()
        } finally {
            if (listeners.values.none { it.uid == callingUid }) release(callingUid)
        }
    }
}

/** Use the device's own AIDL classes; transaction numbers differ between firmware versions. */
private class OplusConnectionProtection(loader: ClassLoader) {
    private val serviceType = loader.loadClass("com.oplus.app.IOplusHansFreezeManager")
    private val serviceStub = loader.loadClass("com.oplus.app.IOplusHansFreezeManager\$Stub")
    private val serviceManager = loader.loadClass("android.os.ServiceManager")
    private val callbackType = loader.loadClass("com.oplus.app.IOplusProtectConnection")
    private val callbackStub = loader.loadClass("com.oplus.app.IOplusProtectConnection\$Stub")
    private val descriptor = callbackType.getField("DESCRIPTOR").get(null) as String
    private fun transaction(name: String): Int = callbackStub.getDeclaredField("TRANSACTION_$name")
        .apply { isAccessible = true }.getInt(null)
    private val success = transaction("onSuccess")
    private val error = transaction("onError")
    private val timeout = transaction("onTimeout")
    private val callback = callbackStub.getMethod("asInterface", IBinder::class.java).invoke(null,
        object : Binder() {
            override fun onTransact(code: Int, data: Parcel, reply: Parcel?, flags: Int): Boolean {
                if (code == IBinder.INTERFACE_TRANSACTION) {
                    reply?.writeString(descriptor)
                    return true
                }
                if (code != success && code != error && code != timeout) {
                    return super.onTransact(code, data, reply, flags)
                }
                data.enforceInterface(descriptor)
                when (code) {
                    error -> Log.w("conduit.freeze", "OEM protection rejected: ${data.readInt()}")
                    timeout -> Log.w("conduit.freeze", "OEM protection expired")
                }
                reply?.writeNoException()
                return true
            }
        })
    private val request = serviceType.getMethod("requestFrozenDelay", Int::class.javaPrimitiveType,
        String::class.java, Long::class.javaPrimitiveType, String::class.java, callbackType)
    private val cancel = serviceType.getMethod("cancelFrozenDelay", Int::class.javaPrimitiveType)
    private val query = serviceType.getMethod("getFrozenDelayTime", Int::class.javaPrimitiveType)

    private fun service(): Any {
        val binder = serviceManager.getMethod("getService", String::class.java).invoke(null, "oplus_freeze")
            ?: error("OEM freeze service absent")
        return requireNotNull(serviceStub.getMethod("asInterface", IBinder::class.java).invoke(null, binder))
    }

    fun acquire(uid: Int) = withSystemIdentity {
        val service = service()
        try {
            // 0 is the OEM's connection-owned lease, not a timer to renew or a wake lock.
            request.invoke(service, uid, PACKAGE, 0L, "conduit-authenticated-desktop", callback)
            check((query.invoke(service, uid) as Long) > 0) { "OEM protection was not granted" }
            Log.i("conduit.freeze", "connected protection acquired uid=$uid")
        } catch (failure: Throwable) {
            runCatching { cancel.invoke(service, uid) }
            throw failure
        }
    }

    fun release(uid: Int) = withSystemIdentity {
        cancel.invoke(service(), uid)
        Log.i("conduit.freeze", "connected protection released uid=$uid")
    }

    private inline fun withSystemIdentity(action: () -> Unit) {
        val identity = Binder.clearCallingIdentity()
        try {
            action()
        } finally {
            Binder.restoreCallingIdentity(identity)
        }
    }
}

//! Minimal JNI bridge exposing `conduit-iroh` to Android Kotlin (`com.conduit.sync.IrohBridge`).

use conduit_iroh::IrohNode;
use jni::objects::{JByteArray, JClass, JObjectArray, JString};
use jni::sys::{jboolean, jint, jlong, jstring};
use jni::JNIEnv;
use std::time::Duration;
use tokio::runtime::Runtime;

struct Handle {
    runtime: Runtime,
    node: IrohNode,
}

#[no_mangle]
pub extern "system" fn Java_com_conduit_sync_IrohBridge_nativeStart(
    mut env: JNIEnv,
    _class: JClass,
    noise_private_key: JByteArray,
    custom_relays: JObjectArray,
    use_default_relays: jboolean,
) -> jlong {
    let Ok(key_bytes) = env.convert_byte_array(noise_private_key) else {
        return 0;
    };
    let mut relays = Vec::new();
    if let Ok(len) = env.get_array_length(&custom_relays) {
        for idx in 0..len {
            if let Ok(obj) = env.get_object_array_element(&custom_relays, idx) {
                let jstr = JString::from(obj);
                let parsed = env
                    .get_string(&jstr)
                    .ok()
                    .map(|s| s.to_string_lossy().into_owned());
                if let Some(s) = parsed {
                    relays.push(s);
                }
            }
        }
    }
    let Ok(runtime) = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
    else {
        return 0;
    };
    let node = runtime.block_on(IrohNode::start(
        &key_bytes,
        &relays,
        use_default_relays != 0,
        None,
    ));
    match node {
        Ok(node) => Box::into_raw(Box::new(Handle { runtime, node })) as jlong,
        Err(_) => 0,
    }
}

#[no_mangle]
pub extern "system" fn Java_com_conduit_sync_IrohBridge_nativeEndpointId(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jstring {
    if handle == 0 {
        return std::ptr::null_mut();
    }
    let h = unsafe { &*(handle as *const Handle) };
    env.new_string(h.node.endpoint_id())
        .map(|s| s.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

#[no_mangle]
pub extern "system" fn Java_com_conduit_sync_IrohBridge_nativeDialLocalPort(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    peer_id_hex: JString,
    timeout_ms: jint,
) -> jint {
    if handle == 0 {
        return -1;
    }
    let Ok(peer) = env.get_string(&peer_id_hex) else {
        return -1;
    };
    let peer = peer.to_string_lossy().into_owned();
    let h = unsafe { &*(handle as *const Handle) };
    let timeout = Duration::from_millis(timeout_ms.clamp(1000, 60_000) as u64);
    match h.runtime.block_on(h.node.dial_local_port(&peer, timeout)) {
        Ok((port, _route)) => port as jint,
        Err(_) => -1,
    }
}

#[no_mangle]
pub extern "system" fn Java_com_conduit_sync_IrohBridge_nativeNetworkChanged(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) {
    if handle == 0 {
        return;
    }
    let h = unsafe { &*(handle as *const Handle) };
    h.runtime.block_on(h.node.network_change());
}

#[no_mangle]
pub extern "system" fn Java_com_conduit_sync_IrohBridge_nativeStop(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) {
    if handle == 0 {
        return;
    }
    let h = unsafe { Box::from_raw(handle as *mut Handle) };
    h.runtime.block_on(h.node.close());
    h.runtime.shutdown_timeout(Duration::from_secs(2));
}

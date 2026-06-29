//! Android JNI initialization for `ndk_context`.
//!
//! iroh's DNS resolver reads `LinkProperties.getDnsServers()` through
//! `ndk_context`, which must be initialized with the process's JavaVM and
//! `Application` context before any [`crate::IrohEndpoint`] is bound. The app
//! calls `IrohAndroid.installAndroidContext(applicationContext)` once at
//! startup; that lands here and stores the pointers for the process lifetime.
//!
//! Only the first call takes effect; later calls are no-ops.

use std::sync::Once;

use jni::objects::{JClass, JObject};
use jni::JNIEnv;

static INIT: Once = Once::new();

#[no_mangle]
pub extern "system" fn Java_app_azula_iroh_IrohAndroid_installAndroidContext(
    env: JNIEnv,
    _class: JClass,
    context: JObject,
) {
    INIT.call_once(|| {
        let Ok(java_vm) = env.get_java_vm() else { return };
        let Ok(global_ref) = env.new_global_ref(&context) else { return };
        unsafe {
            ndk_context::initialize_android_context(
                java_vm.get_java_vm_pointer() as *mut std::ffi::c_void,
                global_ref.as_obj().as_raw() as *mut std::ffi::c_void,
            );
        }
        // ndk_context keeps the raw pointer; the global ref must outlive it.
        std::mem::forget(global_ref);
    });
}

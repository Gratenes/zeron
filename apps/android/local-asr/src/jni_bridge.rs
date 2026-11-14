//! The JNI surface: `sh.zeron.android.car.LocalAsrNative`'s native methods.
//!
//! Handles are boxed pointers owned by Kotlin. Nothing here unwinds into the JVM
//! (panics become exceptions), and ONNX Runtime is never torn down at process
//! exit: the loaded library is kept for the life of the process by `ort`, and a
//! released [`Model`] only drops its sessions.

use crate::{Model, Session};
use jni::JNIEnv;
use jni::objects::{JByteArray, JClass, JString};
use jni::sys::{jint, jlong, jstring};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;

fn throw(env: &mut JNIEnv, message: impl AsRef<str>) {
    let _ = env.throw_new("java/lang/IllegalStateException", message.as_ref());
}

fn guarded<T>(env: &mut JNIEnv, default: T, f: impl FnOnce(&mut JNIEnv) -> Result<T, String>) -> T {
    match catch_unwind(AssertUnwindSafe(|| f(env))) {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => {
            throw(env, e);
            default
        }
        Err(_) => {
            throw(env, "native speech recognizer panicked");
            default
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_sh_zeron_android_car_LocalAsrNative_load(
    mut env: JNIEnv,
    _: JClass,
    dir: JString,
    threads: jint,
) -> jlong {
    guarded(&mut env, 0, |env| {
        let dir: String = env.get_string(&dir).map_err(|e| e.to_string())?.into();
        let model = Model::load(&PathBuf::from(dir), threads.max(1) as usize).map_err(|e| e.0)?;
        Ok(Box::into_raw(Box::new(model)) as jlong)
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_sh_zeron_android_car_LocalAsrNative_unload(_: JNIEnv, _: JClass, model: jlong) {
    if model != 0 {
        drop(unsafe { Box::from_raw(model as *mut Model) });
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_sh_zeron_android_car_LocalAsrNative_open(mut env: JNIEnv, _: JClass, model: jlong) -> jlong {
    guarded(&mut env, 0, |_| {
        let model = unsafe { (model as *const Model).as_ref() }.ok_or("model not loaded")?;
        Ok(Box::into_raw(Box::new(model.session().map_err(|e| e.0)?)) as jlong)
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_sh_zeron_android_car_LocalAsrNative_close(_: JNIEnv, _: JClass, session: jlong) {
    if session != 0 {
        drop(unsafe { Box::from_raw(session as *mut Session) });
    }
}

/// The partial text so far when [len] bytes of 16-bit PCM changed it, else null.
#[unsafe(no_mangle)]
pub extern "system" fn Java_sh_zeron_android_car_LocalAsrNative_feed(
    mut env: JNIEnv,
    _: JClass,
    session: jlong,
    pcm: JByteArray,
    len: jint,
) -> jstring {
    guarded(&mut env, std::ptr::null_mut(), |env| {
        let session = unsafe { (session as *mut Session).as_mut() }.ok_or("no session")?;
        let bytes = env.convert_byte_array(&pcm).map_err(|e| e.to_string())?;
        let bytes = &bytes[..(len.max(0) as usize).min(bytes.len())];
        match session.feed(bytes).map_err(|e| e.0)? {
            Some(text) => Ok(env.new_string(text).map_err(|e| e.to_string())?.into_raw()),
            None => Ok(std::ptr::null_mut()),
        }
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_sh_zeron_android_car_LocalAsrNative_finish(
    mut env: JNIEnv,
    _: JClass,
    session: jlong,
) -> jstring {
    guarded(&mut env, std::ptr::null_mut(), |env| {
        let session = unsafe { (session as *mut Session).as_mut() }.ok_or("no session")?;
        let text = session.finish().map_err(|e| e.0)?;
        Ok(env.new_string(text).map_err(|e| e.to_string())?.into_raw())
    })
}

//! Android-owned directories for persistent state and disposable cache.

use std::path::PathBuf;

use jni::JavaVM;
use jni::objects::{JObject, JString};
use slint::android::AndroidApp;

pub struct AndroidPaths {
    pub files: PathBuf,
    pub cache: PathBuf,
}

impl AndroidPaths {
    pub fn from_app(app: &AndroidApp) -> jni::errors::Result<Self> {
        // AndroidApp owns both references for the duration of this call. JObject
        // does not delete the unowned global activity reference on drop.
        let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast())? };
        let mut env = vm.attach_current_thread()?;
        let activity = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };
        fn path(
            env: &mut jni::JNIEnv<'_>,
            activity: &JObject<'_>,
            method: &str,
        ) -> jni::errors::Result<PathBuf> {
            let file = env
                .call_method(activity, method, "()Ljava/io/File;", &[])?
                .l()?;
            let value = env
                .call_method(file, "getAbsolutePath", "()Ljava/lang/String;", &[])?
                .l()?;
            let value = JString::from(value);
            Ok(PathBuf::from(String::from(env.get_string(&value)?)))
        }
        Ok(Self {
            files: path(&mut env, &activity, "getFilesDir")?,
            cache: path(&mut env, &activity, "getCacheDir")?,
        })
    }
}

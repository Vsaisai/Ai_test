//! JNI entry points for the Android app.
//!
//! The app is a thin WebView shell: `startServer(filesDir)` launches the same server as the
//! desktop `vndb-web serve` on 127.0.0.1 with a random port, and the WebView opens it.
//! All data (catalog, personal lists, image cache) lives in the app's private storage.

use jni::objects::{JClass, JString};
use jni::sys::jint;
use jni::JNIEnv;
use std::path::PathBuf;
use std::sync::OnceLock;

static PORT: OnceLock<u16> = OnceLock::new();

/// Start (once) and return the local port, or -1 on failure.
pub fn start(data_dir: PathBuf) -> i32 {
    if let Some(p) = PORT.get() {
        return *p as i32;
    }
    // Android has no /tmp: SQLite needs a writable place for sort/temp files during imports.
    let tmp = data_dir.join("tmp");
    let _ = std::fs::create_dir_all(&tmp);
    std::env::set_var("TMPDIR", &tmp);
    std::env::set_var("SQLITE_TMPDIR", &tmp);
    match vndb_web::spawn_local(data_dir, vndb_web::MOBILE_PORT) {
        Ok(port) => {
            let _ = PORT.set(port);
            port as i32
        }
        Err(e) => {
            eprintln!("vnshelf: failed to start server: {e:#}");
            -1
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_ru_vnshelf_app_MainActivity_startServer(
    mut env: JNIEnv,
    _class: JClass,
    dir: JString,
) -> jint {
    let dir: String = match env.get_string(&dir) {
        Ok(s) => s.into(),
        Err(_) => return -1,
    };
    start(PathBuf::from(dir).join("vnshelf"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn starts_once_and_serves() {
        let dir = std::env::temp_dir().join(format!("vnshelf-mobile-test-{}", std::process::id()));
        let p1 = super::start(dir.clone());
        let p2 = super::start(dir.clone());
        assert!(p1 > 0);
        assert_eq!(p1, p2);
        // no catalog yet: status must say so
        let body = std::process::Command::new("curl")
            .args(["-s", &format!("http://127.0.0.1:{p1}/api/status")])
            .output();
        if let Ok(out) = body {
            let text = String::from_utf8_lossy(&out.stdout);
            assert!(text.contains("\"ready\":false"), "{text}");
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}

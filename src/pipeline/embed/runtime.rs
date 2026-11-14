//! ONNX Runtime availability check (issue #54).
//!
//! The default build links the prebuilt static ONNX Runtime that `ort`
//! downloads, so there is nothing to check. With the `ort-load-dynamic`
//! feature, `ort` loads the shared library at run time from
//! `ORT_DYLIB_PATH` (or the platform default name), and a missing or too old
//! library would only surface as a panic inside the first embedding call.
//! [`ensure_loaded`] loads it up front and turns a failure into an error
//! that names `ORT_DYLIB_PATH` and the minimum supported version.

/// Environment variable `ort` reads the ONNX Runtime shared library path from.
pub const ORT_DYLIB_PATH_VAR: &str = "ORT_DYLIB_PATH";

/// Make sure ONNX Runtime can be used before the first model is loaded.
///
/// Built with `ort-load-dynamic`: loads the shared library (once per
/// process) and fails with an actionable message when it is missing or
/// older than the API level this build requires. Without the feature the
/// runtime is linked into the binary and this is a no-op.
///
/// The first outcome is final for the process: later calls return it
/// without asking `ort` again. `ort` 2.0.0-rc.13 marks its library slot as
/// initialized even when the load fails, so a second `ort::init_from` after
/// a failure would read an unloaded library instead of failing again.
///
/// Does not create the ONNX Runtime environment: applications keep the
/// option of configuring it themselves with `ort::init()`.
#[cfg(feature = "ort-load-dynamic")]
pub fn ensure_loaded() -> anyhow::Result<()> {
    static OUTCOME: std::sync::OnceLock<Result<(), String>> = std::sync::OnceLock::new();
    OUTCOME
        .get_or_init(load_dylib)
        .clone()
        .map_err(anyhow::Error::msg)
}

/// Load the shared library once through `ort`.
#[cfg(feature = "ort-load-dynamic")]
fn load_dylib() -> Result<(), String> {
    let path = dylib_path(std::env::var_os(ORT_DYLIB_PATH_VAR));
    // `init_from` keeps the library loaded in a process-wide slot that `ort`
    // uses from then on; the returned builder is dropped on purpose, without
    // committing an environment.
    let builder = ort::init_from(&path)
        .map_err(|e| load_error_message(&e.to_string(), ort::MINOR_VERSION))?;
    drop(builder);
    Ok(())
}

/// Make sure ONNX Runtime can be used before the first model is loaded.
///
/// This build links ONNX Runtime statically, so there is nothing to load.
#[cfg(not(feature = "ort-load-dynamic"))]
pub fn ensure_loaded() -> anyhow::Result<()> {
    Ok(())
}

/// Library path `ort` would load: `ORT_DYLIB_PATH` when set and non-empty,
/// else the platform's default library name (resolved next to the
/// executable first, then by the system loader).
#[cfg(feature = "ort-load-dynamic")]
fn dylib_path(var: Option<std::ffi::OsString>) -> std::path::PathBuf {
    match var {
        Some(path) if !path.is_empty() => path.into(),
        _ => default_dylib_name().into(),
    }
}

/// File name of the ONNX Runtime shared library on this platform.
#[cfg(feature = "ort-load-dynamic")]
const fn default_dylib_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "onnxruntime.dll"
    } else if cfg!(target_os = "macos") {
        "libonnxruntime.dylib"
    } else {
        "libonnxruntime.so"
    }
}

/// Error shown when the shared library cannot be loaded or is too old.
#[cfg(feature = "ort-load-dynamic")]
fn load_error_message(cause: &str, min_minor: u32) -> String {
    format!(
        "Could not load ONNX Runtime: {cause}\n\
         This build of knot uses the `ort-load-dynamic` feature: it loads ONNX Runtime \
         at run time instead of bundling it. Install ONNX Runtime 1.{min_minor} or newer \
         (https://github.com/microsoft/onnxruntime/releases) and point {ORT_DYLIB_PATH_VAR} \
         at its library, e.g. {ORT_DYLIB_PATH_VAR}=/opt/onnxruntime/lib/{}",
        default_dylib_name()
    )
}

#[cfg(all(test, feature = "ort-load-dynamic"))]
mod tests {
    use super::*;

    #[test]
    fn dylib_path_prefers_the_env_var() {
        let path = dylib_path(Some("/opt/ort/lib/libonnxruntime.so.1.28.0".into()));
        assert_eq!(
            path,
            std::path::PathBuf::from("/opt/ort/lib/libonnxruntime.so.1.28.0")
        );
    }

    #[test]
    fn dylib_path_falls_back_to_the_default_name() {
        assert_eq!(
            dylib_path(None),
            std::path::PathBuf::from(default_dylib_name())
        );
        assert_eq!(
            dylib_path(Some("".into())),
            std::path::PathBuf::from(default_dylib_name())
        );
    }

    #[test]
    fn load_error_message_names_the_env_var_and_version() {
        let msg = load_error_message("failed to load from `/x`: not found", 27);
        assert!(msg.contains("failed to load from `/x`"), "{msg}");
        assert!(msg.contains("ORT_DYLIB_PATH="), "{msg}");
        assert!(msg.contains("1.27 or newer"), "{msg}");
        assert!(msg.contains("ort-load-dynamic"), "{msg}");
    }

    #[test]
    fn ensure_loaded_reports_a_missing_library() {
        let missing = std::env::temp_dir().join("knot-no-such-dir/libonnxruntime.so");
        let err = temp_env::with_var(ORT_DYLIB_PATH_VAR, Some(&missing), ensure_loaded)
            .expect_err("a missing library must not load");
        let msg = err.to_string();
        assert!(msg.contains(ORT_DYLIB_PATH_VAR), "{msg}");
        assert!(msg.contains("knot-no-such-dir"), "{msg}");
        // A retry gets the same error instead of reaching `ort` again.
        let again = ensure_loaded().expect_err("a failed load stays failed");
        assert_eq!(again.to_string(), msg);
    }

    #[test]
    fn embedder_init_fails_before_downloading_the_model() {
        use crate::pipeline::embed::{DEFAULT_EMBED_MODEL, EmbedModelChoice, Embedder};

        let cache = tempfile::tempdir().expect("temp dir");
        let choice: EmbedModelChoice = DEFAULT_EMBED_MODEL.parse().expect("default model");
        let missing = std::env::temp_dir().join("knot-no-such-dir/libonnxruntime.so");
        let result = temp_env::with_var(ORT_DYLIB_PATH_VAR, Some(&missing), || {
            Embedder::init_with_model(cache.path().join("models"), choice)
        });
        let Err(err) = result else {
            panic!("a missing library must fail Embedder init");
        };
        assert!(err.to_string().contains(ORT_DYLIB_PATH_VAR), "{err}");
        // Nothing was downloaded: the cache dir was never even created.
        assert!(!cache.path().join("models").exists());
    }
}

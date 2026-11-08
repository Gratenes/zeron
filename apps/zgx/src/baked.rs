//! Defaults baked into a build made for one install: a development build to
//! double-click, with no launcher to set its variables (as crates/cli/src/env.rs
//! bakes the classic app's edge and client). Each is read only when its runtime
//! variable is unset, so the environment still overrides it, and a build made
//! without them (upstream's) has none: nothing here changes it.
//!
//!   ZERON_ZGX_DEFAULT_EDGE                 for ZERON_ZGX_EDGE
//!   ZERON_ZGX_DEFAULT_WORKOS_CLIENT        for ZERON_ZGX_WORKOS_CLIENT
//!   ZERON_ZGX_DEFAULT_ENGINE_PORT          for ZERON_ZGX_ENGINE_PORT
//!   ZERON_ZGX_DEFAULT_ALLOW_DEFAULT_PORT   for ZERON_ZGX_ALLOW_DEFAULT_PORT
//!   ZERON_ZGX_DEFAULT_ENGINE_DATA_NAME     for ZERON_ZGX_ENGINE_DATA: a folder
//!                                          name, resolved under %LOCALAPPDATA%
//!                                          as the app runs (no user's path baked)
//!   ZERON_ZGX_DEFAULT_ENGINE_BIN           for ZERON_ZGX_ENGINE_BIN (`%NAME%`
//!                                          expanded as the app runs)
//!   ZERON_ZGX_DEFAULT_ROUTE                the screen a launch with no
//!                                          arguments opens (`main`)
//!
//! A build with a route baked is one to double-click: on Windows it has no
//! console (build.rs's `zgx_gui`) and writes its output to its engine's
//! `logs\zgx.log` (src/self_log.rs). The beta bakes none of this.

use std::path::PathBuf;

fn raw(key: &str) -> Option<&'static str> {
    if cfg!(feature = "beta") {
        return None;
    }
    let value = match key {
        "ZERON_ZGX_EDGE" => option_env!("ZERON_ZGX_DEFAULT_EDGE"),
        "ZERON_ZGX_WORKOS_CLIENT" => option_env!("ZERON_ZGX_DEFAULT_WORKOS_CLIENT"),
        "ZERON_ZGX_ENGINE_PORT" => option_env!("ZERON_ZGX_DEFAULT_ENGINE_PORT"),
        "ZERON_ZGX_ALLOW_DEFAULT_PORT" => option_env!("ZERON_ZGX_DEFAULT_ALLOW_DEFAULT_PORT"),
        "ZERON_ZGX_ENGINE_DATA" => option_env!("ZERON_ZGX_DEFAULT_ENGINE_DATA_NAME"),
        "ZERON_ZGX_ENGINE_BIN" => option_env!("ZERON_ZGX_DEFAULT_ENGINE_BIN"),
        _ => None,
    };
    value.map(str::trim).filter(|v| !v.is_empty())
}

/// The baked default for the runtime variable `key`, resolved for this
/// machine, if this build has one.
pub fn var(key: &str) -> Option<String> {
    let value = raw(key)?;
    match key {
        "ZERON_ZGX_ENGINE_DATA" => local_app_data().map(|dir| dir.join(value).to_string_lossy().into_owned()),
        "ZERON_ZGX_ENGINE_BIN" => Some(expand_vars(value)),
        _ => Some(value.to_owned()),
    }
}

/// The runtime variable `key`, else its baked default.
pub fn env_or_baked(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.trim().is_empty()).or_else(|| var(key))
}

/// The screen a launch with no arguments opens, if baked.
pub fn route() -> Option<&'static str> {
    if cfg!(feature = "beta") {
        return None;
    }
    option_env!("ZERON_ZGX_DEFAULT_ROUTE").map(str::trim).filter(|v| !v.is_empty())
}

/// %LOCALAPPDATA% (Windows), else ~/.local/share.
fn local_app_data() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("LOCALAPPDATA").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    std::env::var_os("HOME").filter(|d| !d.is_empty()).map(|home| PathBuf::from(home).join(".local").join("share"))
}

/// `%NAME%` replaced by that variable's value (left as it is when unset).
pub fn expand_vars(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('%') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('%') else { break };
        let name = &after[..end];
        out.push_str(&rest[..start]);
        match std::env::var(name).ok().filter(|_| !name.is_empty()) {
            Some(value) => out.push_str(&value),
            None => {
                out.push('%');
                out.push_str(name);
                out.push('%');
            }
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_variables_expand_and_unknown_ones_stay() {
        // SAFETY: a test of its own variable; nothing else reads it.
        unsafe { std::env::set_var("ZGX_BAKED_TEST_DIR", r"C:\Users\someone") };
        assert_eq!(expand_vars(r"%ZGX_BAKED_TEST_DIR%\Downloads\a.exe"), r"C:\Users\someone\Downloads\a.exe");
        assert_eq!(expand_vars(r"%ZGX_NOT_SET_ANYWHERE%\a"), r"%ZGX_NOT_SET_ANYWHERE%\a");
        assert_eq!(expand_vars("plain"), "plain");
        assert_eq!(expand_vars("50%"), "50%");
    }
}

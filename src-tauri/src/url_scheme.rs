//! Claim default handlers for one-click subscribe schemes.
//!
//! Multiple proxy clients register the same schemes (`clash://`, `sing-box://`).
//! The OS picks one default; without claiming, clicks often open Verge / Sparkle / FlClash.

/// Bundle id from `tauri.conf.json` → `identifier`.
const BUNDLE_ID: &str = "com.satelite.proxy";

/// Schemes we handle (must match `plugins.deep-link.desktop.schemes`).
const SCHEMES: &[&str] = &["clash", "sing-box", "singbox"];

/// Best-effort: become the default open target for subscription deep links.
pub fn claim_subscription_schemes() {
    #[cfg(target_os = "macos")]
    macos::claim(SCHEMES, BUNDLE_ID);

    #[cfg(any(windows, target_os = "linux"))]
    {
        // Windows/Linux: deep-link `register_all` re-associates this executable.
        // Called from lib.rs after DeepLink is ready — no-op here.
        let _ = (SCHEMES, BUNDLE_ID);
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use core_foundation::base::TCFType;
    use core_foundation::string::CFString;
    use core_foundation_sys::string::CFStringRef;

    #[link(name = "CoreServices", kind = "framework")]
    extern "C" {
        fn LSSetDefaultHandlerForURLScheme(
            in_url_scheme: CFStringRef,
            in_handler_bundle_id: CFStringRef,
        ) -> i32;
    }

    pub fn claim(schemes: &[&str], bundle_id: &str) {
        let bid = CFString::new(bundle_id);
        for scheme in schemes {
            let s = CFString::new(scheme);
            // 0 = noErr
            let status = unsafe {
                LSSetDefaultHandlerForURLScheme(s.as_concrete_TypeRef(), bid.as_concrete_TypeRef())
            };
            if status != 0 {
                eprintln!("[satelite] LSSetDefaultHandlerForURLScheme({scheme}) status={status}");
            }
        }
    }
}

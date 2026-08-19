//! API keys stored in the OS keychain (macOS Keychain via the `keyring` crate),
//! so secrets never live in the webview or a plaintext file.
//!
//! **Every read is memoised for the life of the process.** macOS authorises a
//! keychain item per *read*, not per launch, so an uncached `get_key` turns one
//! run over 200 files into 200 dialogs, and the three status reads at startup
//! into six as soon as React's StrictMode mounts the app twice. Memoised, the
//! ceiling is one prompt per account per launch.
//!
//! The prompts themselves come from code identity, not from this module: see
//! `scripts/dev-run.sh`, which gives the dev binary the same signature the
//! installed app has so the keychain still recognises it after a rebuild.

use keyring::Entry;
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, OnceLock};

const SERVICE: &str = "ai.uniwise.toolkit";

type Cache = HashMap<String, Option<String>>;

fn cache() -> MutexGuard<'static, Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    let lock = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    // Poisoning is recovered rather than propagated: the critical section spans
    // a keychain call, and one panic in there must not fail every later read.
    lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Read `name` through the cache, calling `read` only on a miss.
///
/// The lock spans `read` on purpose: four jobs starting together would
/// otherwise race into four separate dialogs for the same account.
fn memoized(name: &str, read: impl FnOnce() -> Option<String>) -> Option<String> {
    let mut cache = cache();
    if let Some(hit) = cache.get(name) {
        return hit.clone();
    }
    let value = read();
    // A denied prompt is memoised as `None` too. "Deny" answered once should not
    // come back for every remaining file in the run; the app restarts to re-ask.
    cache.insert(name.to_owned(), value.clone());
    value
}

fn forget(name: &str) {
    cache().remove(name);
}

fn entry(name: &str) -> Result<Entry, String> {
    Entry::new(SERVICE, name).map_err(|e| e.to_string())
}

/// Store (or, when `value` is empty, clear) the key for a provider.
pub fn set_key(name: &str, value: &str) -> Result<(), String> {
    let entry = entry(name)?;
    let result = if value.is_empty() {
        let _ = entry.delete_credential();
        Ok(())
    } else {
        entry.set_password(value).map_err(|e| e.to_string())
    };
    // Drop the memo whatever happened. A failed write must not leave the old
    // value cached as though it were still what the keychain holds, and the
    // Settings panel re-reads the status the moment this returns.
    forget(name);
    result
}

pub fn get_key(name: &str) -> Option<String> {
    memoized(name, || {
        entry(name).ok().and_then(|e| e.get_password().ok())
    })
}

pub fn has_key(name: &str) -> bool {
    get_key(name).map(|v| !v.is_empty()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // The cache is process-wide and tests share it, so each test owns a name.

    #[test]
    fn a_key_is_read_from_the_keychain_once_per_process() {
        let reads = AtomicUsize::new(0);
        let read = || {
            reads.fetch_add(1, Ordering::SeqCst);
            Some("secret".to_string())
        };

        assert_eq!(memoized("test-read-once", read), Some("secret".into()));
        for _ in 0..10 {
            assert_eq!(memoized("test-read-once", read), Some("secret".into()));
        }
        assert_eq!(reads.load(Ordering::SeqCst), 1, "one prompt, not eleven");
    }

    #[test]
    fn a_missing_or_denied_key_is_memoised_too() {
        let reads = AtomicUsize::new(0);
        let read = || {
            reads.fetch_add(1, Ordering::SeqCst);
            None
        };

        assert_eq!(memoized("test-absent", read), None);
        assert_eq!(memoized("test-absent", read), None);
        assert_eq!(reads.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn writing_a_key_drops_the_memo_so_settings_sees_its_own_change() {
        assert_eq!(
            memoized("test-invalidate", || Some("old".into())),
            Some("old".into())
        );
        // Still the stale value while the memo stands.
        assert_eq!(
            memoized("test-invalidate", || Some("new".into())),
            Some("old".into())
        );

        forget("test-invalidate");
        assert_eq!(
            memoized("test-invalidate", || Some("new".into())),
            Some("new".into())
        );
    }
}

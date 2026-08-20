//! API keys in the macOS **data protection** keychain, so secrets never reach
//! the webview or a plaintext file.
//!
//! macOS has two keychains and the difference is the whole point of this
//! module. The legacy file-based store grants access through a per-item ACL
//! bound to the reader's designated requirement, so any change to the app's
//! signature voids every "Always Allow" and the dialogs come back. The data
//! protection store has no ACLs at all: access is the `keychain-access-groups`
//! entitlement, matched on team ID, so no dialog exists in that path.
//!
//! That entitlement is restricted and is only honoured when an embedded
//! provisioning profile authorises it. A bundled `.app` carries one at
//! `Contents/embedded.provisionprofile`; a bare `cargo run` binary has nowhere
//! to put it. So the dev loop falls back to the legacy store and keeps its own
//! separate copy of every key. Use `pnpm tauri build --debug` to exercise the
//! real path.
//!
//! Reads stay memoised for the life of the process. The old reason was prompt
//! count, which no longer applies: the reason now is latency, because a
//! 200-file run reads the same key once per file and each read is a round trip
//! to `securityd`.

use security_framework::base::Error as SecError;
use security_framework::os::macos::keychain::SecKeychain;
use security_framework::os::macos::passwords::find_generic_password;
use security_framework::passwords::{
    delete_generic_password_options, generic_password, set_generic_password_options,
};
use security_framework::passwords_options::PasswordOptions;
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, OnceLock};

const SERVICE: &str = "dev.esfandiari.toolkit";
const ACCESS_GROUP: &str = "92MA44797J.dev.esfandiari.toolkit";

/// `errSecMissingEntitlement`. `security-framework-sys` does not name it.
const MISSING_ENTITLEMENT: i32 = -34018;

/// An account no key is ever stored under, so the probe reads nothing real.
const PROBE_ACCOUNT: &str = "entitlement-probe";

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
/// otherwise make four separate round trips for the same account.
fn memoized(name: &str, read: impl FnOnce() -> Option<String>) -> Option<String> {
    let mut cache = cache();
    if let Some(hit) = cache.get(name) {
        return hit.clone();
    }
    let value = read();
    // A missing key is memoised as `None` too, so a run over 200 files does not
    // re-ask for a key that is not there. Settings drops the memo on write.
    cache.insert(name.to_owned(), value.clone());
    value
}

fn forget(name: &str) {
    cache().remove(name);
}

/// Which of the two keychains this process can reach.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Store {
    Protected,
    Legacy,
}

/// Only a missing entitlement rules the data protection keychain out. Every
/// other outcome, `errSecItemNotFound` included, proves it answered us.
fn classify(probe: Result<Vec<u8>, SecError>) -> Store {
    match probe {
        Err(error) if error.code() == MISSING_ENTITLEMENT => Store::Legacy,
        _ => Store::Protected,
    }
}

fn store() -> Store {
    static STORE: OnceLock<Store> = OnceLock::new();
    *STORE.get_or_init(|| {
        let selected = classify(generic_password(protected(PROBE_ACCOUNT)));
        if selected == Store::Legacy {
            // Expected under `cargo run`, which cannot carry the profile. In a
            // release bundle it means the entitlement or the profile is missing
            // and the keychain dialogs are about to come back.
            eprintln!(
                "[tool-kit] no keychain-access-groups entitlement, falling back \
                 to the legacy keychain"
            );
        }
        selected
    })
}

/// A data protection query for one account.
fn protected(account: &str) -> PasswordOptions {
    let mut options = PasswordOptions::new_generic_password(SERVICE, account);
    options.use_protected_keychain();
    options.set_access_group(ACCESS_GROUP);
    // These are machine-local credentials. Roaming them through iCloud Keychain
    // would copy them to every Mac signed into the account.
    options.set_access_synchronized(Some(false));
    options
}

/// Keychain Access lists items by label, and the service name alone reads as a
/// bare bundle id there.
fn label(account: &str) -> String {
    format!("Tool-Kit ({account})")
}

fn read(account: &str) -> Option<String> {
    let bytes = match store() {
        Store::Protected => generic_password(protected(account)).ok()?,
        Store::Legacy => {
            let keychain = SecKeychain::default().ok()?;
            let (password, _) = find_generic_password(Some(&[keychain]), SERVICE, account).ok()?;
            password.to_vec()
        }
    };
    String::from_utf8(bytes).ok()
}

fn write(account: &str, value: &str) -> Result<(), String> {
    match store() {
        Store::Protected => {
            let mut options = protected(account);
            options.set_label(&label(account));
            set_generic_password_options(value.as_bytes(), options).map_err(|e| e.to_string())
        }
        Store::Legacy => SecKeychain::default()
            .and_then(|keychain| keychain.set_generic_password(SERVICE, account, value.as_bytes()))
            .map_err(|e| e.to_string()),
    }
}

fn erase(account: &str) {
    match store() {
        Store::Protected => {
            let _ = delete_generic_password_options(protected(account));
        }
        Store::Legacy => {
            if let Ok(keychain) = SecKeychain::default() {
                if let Ok((_, item)) = find_generic_password(Some(&[keychain]), SERVICE, account) {
                    item.delete();
                }
            }
        }
    }
}

/// Store (or, when `value` is empty, clear) the key for a provider.
pub fn set_key(name: &str, value: &str) -> Result<(), String> {
    let result = if value.is_empty() {
        erase(name);
        Ok(())
    } else {
        write(name, value)
    };
    // Drop the memo whatever happened. A failed write must not leave the old
    // value cached as though it were still what the keychain holds, and the
    // Settings panel re-reads the status the moment this returns.
    forget(name);
    result
}

pub fn get_key(name: &str) -> Option<String> {
    memoized(name, || read(name))
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
        assert_eq!(reads.load(Ordering::SeqCst), 1, "one round trip, not eleven");
    }

    #[test]
    fn a_missing_key_is_memoised_too() {
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

    /// The sidecar mints its token during the launch, after something has
    /// already asked for it and been told there is none. `set_key` is the only
    /// write that drops that memo, which is why `backend_host` goes through it
    /// rather than writing the keychain directly: skip it and every backend job
    /// fails for the rest of the session against a token that is really there.
    #[test]
    fn minting_the_backend_token_replaces_a_memoised_absence() {
        assert_eq!(memoized("test-backend-mint", || None), None);
        assert_eq!(memoized("test-backend-mint", || Some("minted".into())), None);

        forget("test-backend-mint");

        assert_eq!(
            memoized("test-backend-mint", || Some("minted".into())),
            Some("minted".into())
        );
    }

    #[test]
    fn only_a_missing_entitlement_falls_back_to_the_legacy_keychain() {
        assert_eq!(
            classify(Err(SecError::from_code(MISSING_ENTITLEMENT))),
            Store::Legacy
        );
        // errSecItemNotFound. The probe account holds nothing, which is the
        // expected answer and still proves the entitlement is in force.
        assert_eq!(classify(Err(SecError::from_code(-25300))), Store::Protected);
        assert_eq!(classify(Ok(b"key".to_vec())), Store::Protected);
    }

    /// Talks to the real keychain, so it is ignored by default. Run it after
    /// changing the store selection:
    ///
    ///   cargo test --lib secrets::tests::an_unsigned -- --ignored --nocapture
    ///
    /// A test binary is ad-hoc signed and carries no profile, so this asserts
    /// the exact condition the dev-loop fallback depends on. If it ever stops
    /// reporting -34018, the fallback is dead code and dev reads are silently
    /// hitting a different store than they used to.
    #[test]
    #[ignore]
    fn an_unsigned_binary_is_refused_the_data_protection_keychain() {
        let error = generic_password(protected(PROBE_ACCOUNT))
            .expect_err("an unentitled binary must not read the protected store");
        println!("code {}: {:?}", error.code(), error.message());
        assert_eq!(error.code(), MISSING_ENTITLEMENT);
        assert_eq!(classify(Err(error)), Store::Legacy);
    }

    #[test]
    fn the_access_group_is_prefixed_with_the_team_id() {
        // codesign rejects a group that does not start with the team that signs
        // the app, and the failure surfaces as a runtime -34018, not a build
        // error, so assert the shape here.
        assert!(ACCESS_GROUP.starts_with("92MA44797J."));
        assert!(ACCESS_GROUP.ends_with(SERVICE));
    }
}

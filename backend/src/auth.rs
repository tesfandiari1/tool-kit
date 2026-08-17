use std::{
    fmt,
    fs::File,
    io::{Read, Take},
    path::{Path, PathBuf},
    sync::Arc,
};

use axum::http::{header::AUTHORIZATION, HeaderMap};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use thiserror::Error;
use zeroize::Zeroize;

const MIN_TOKEN_BYTES: usize = 32;
const MAX_TOKEN_BYTES: usize = 512;
const MAX_TOKEN_FILE_BYTES: u64 = 4096;

#[derive(Clone)]
pub struct BootstrapAuth {
    digest: Arc<[u8; 32]>,
}

impl BootstrapAuth {
    pub fn load(path: &Path) -> Result<Self, AuthLoadError> {
        let metadata = std::fs::metadata(path).map_err(|source| AuthLoadError::Read {
            path: path.to_owned(),
            source,
        })?;
        if !metadata.is_file() {
            return Err(AuthLoadError::NotAFile(path.to_owned()));
        }

        let file = File::open(path).map_err(|source| AuthLoadError::Read {
            path: path.to_owned(),
            source,
        })?;
        let mut raw = Vec::new();
        read_bounded(file.take(MAX_TOKEN_FILE_BYTES + 1), &mut raw).map_err(|source| {
            AuthLoadError::Read {
                path: path.to_owned(),
                source,
            }
        })?;
        if raw.len() as u64 > MAX_TOKEN_FILE_BYTES {
            raw.zeroize();
            return Err(AuthLoadError::TooLarge(path.to_owned()));
        }

        strip_one_line_ending(&mut raw);
        validate_token(&raw).map_err(|reason| {
            raw.zeroize();
            AuthLoadError::Invalid {
                path: path.to_owned(),
                reason,
            }
        })?;

        let digest: [u8; 32] = Sha256::digest(&raw).into();
        raw.zeroize();

        Ok(Self {
            digest: Arc::new(digest),
        })
    }

    pub fn authorizes(&self, headers: &HeaderMap) -> bool {
        let mut values = headers.get_all(AUTHORIZATION).iter();
        let Some(value) = values.next() else {
            return false;
        };
        if values.next().is_some() {
            return false;
        }
        let Ok(value) = value.to_str() else {
            return false;
        };
        let Some((scheme, token)) = value.split_once(' ') else {
            return false;
        };
        if !scheme.eq_ignore_ascii_case("Bearer") || token.contains(' ') {
            return false;
        }
        let bytes = token.as_bytes();
        if validate_token(bytes).is_err() {
            return false;
        }

        let candidate: [u8; 32] = Sha256::digest(bytes).into();
        bool::from(self.digest.as_ref().ct_eq(&candidate))
    }
}

impl fmt::Debug for BootstrapAuth {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BootstrapAuth")
            .field("digest", &"[REDACTED]")
            .finish()
    }
}

fn read_bounded(mut reader: Take<File>, output: &mut Vec<u8>) -> std::io::Result<()> {
    reader.read_to_end(output).map(|_| ())
}

fn strip_one_line_ending(raw: &mut Vec<u8>) {
    if raw.last() == Some(&b'\n') {
        raw.pop();
        if raw.last() == Some(&b'\r') {
            raw.pop();
        }
    }
}

fn validate_token(raw: &[u8]) -> Result<(), &'static str> {
    if !(MIN_TOKEN_BYTES..=MAX_TOKEN_BYTES).contains(&raw.len()) {
        return Err("token length must be between 32 and 512 bytes");
    }
    if !raw.iter().all(|byte| byte.is_ascii_graphic()) {
        return Err("token must contain only visible ASCII characters and no whitespace");
    }
    Ok(())
}

#[derive(Debug, Error)]
pub enum AuthLoadError {
    #[error("bootstrap token path is not a regular file: {0:?}")]
    NotAFile(PathBuf),
    #[error("cannot read bootstrap token file {path:?}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("bootstrap token file exceeds {MAX_TOKEN_FILE_BYTES} bytes: {0:?}")]
    TooLarge(PathBuf),
    #[error("invalid bootstrap token file {path:?}: {reason}")]
    Invalid { path: PathBuf, reason: &'static str },
}

#[cfg(test)]
mod tests {
    use std::fs;

    use axum::http::{header::AUTHORIZATION, HeaderMap, HeaderValue};
    use tempfile::tempdir;

    use super::BootstrapAuth;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn token_is_loaded_with_one_terminal_newline() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("token");
        fs::write(&path, format!("{TOKEN}\n")).unwrap();
        let auth = BootstrapAuth::load(&path).unwrap();

        assert!(auth.authorizes(&headers(&format!("Bearer {TOKEN}"))));
        assert!(!auth.authorizes(&headers("Bearer wrong-wrong-wrong-wrong-wrong-wrong")));
    }

    #[test]
    fn whitespace_and_short_tokens_are_rejected() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("token");

        fs::write(&path, "short").unwrap();
        assert!(BootstrapAuth::load(&path).is_err());

        fs::write(&path, format!("{TOKEN} \n")).unwrap();
        assert!(BootstrapAuth::load(&path).is_err());
    }

    #[test]
    fn missing_directory_and_oversized_token_sources_are_rejected() {
        let directory = tempdir().unwrap();
        assert!(BootstrapAuth::load(&directory.path().join("missing")).is_err());
        assert!(BootstrapAuth::load(directory.path()).is_err());

        let path = directory.path().join("oversized");
        fs::write(&path, vec![b'x'; 4097]).unwrap();
        assert!(BootstrapAuth::load(&path).is_err());
    }

    #[test]
    fn duplicate_authorization_headers_are_rejected() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("token");
        fs::write(&path, TOKEN).unwrap();
        let auth = BootstrapAuth::load(&path).unwrap();
        let mut headers = headers(&format!("Bearer {TOKEN}"));
        headers.append(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {TOKEN}")).unwrap(),
        );

        assert!(!auth.authorizes(&headers));
    }

    fn headers(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_str(value).unwrap());
        headers
    }
}

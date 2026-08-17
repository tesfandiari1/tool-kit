use std::{env, net::SocketAddr, path::PathBuf, time::Duration};

use thiserror::Error;
use tracing_subscriber::EnvFilter;

const DEFAULT_BIND_ADDRESS: &str = "127.0.0.1:8080";
const DEFAULT_LOG_FILTER: &str = "tool_kit_converter=info";
const DEFAULT_TOKEN_FILE: &str = "/run/secrets/bootstrap_token";
const DEFAULT_DATA_DIR: &str = "/data";
const DEFAULT_SCRATCH_PARENT: &str = "/tmp";
const DEFAULT_MAX_UPLOAD_BYTES: u64 = 25 * 1024 * 1024;
const DEFAULT_MAX_OUTPUT_BYTES: u64 = 50 * 1024 * 1024;
const DEFAULT_MAX_JOBS: usize = 32;
const DEFAULT_MAX_CONCURRENT_UPLOADS: usize = 2;
const DEFAULT_UPLOAD_TIMEOUT_SECS: u64 = 120;
const DEFAULT_PDF_TIMEOUT_SECS: u64 = 60;
const DEFAULT_PDF_THREADS: usize = 2;
const DEFAULT_DATABASE_BUSY_TIMEOUT_SECS: u64 = 5;
const DEFAULT_WORKER_POLL_INTERVAL_SECS: u64 = 1;
const DEFAULT_RECOVERY_LIMIT: usize = 3;
const DEFAULT_SHUTDOWN_GRACE_SECS: u64 = 30;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Settings {
    pub bind_address: SocketAddr,
    pub log_filter: String,
    pub token_file: PathBuf,
    pub data_dir: PathBuf,
    pub scratch_parent: PathBuf,
    pub pdf_worker_path: PathBuf,
    pub pdf_bcmaps_dir: Option<PathBuf>,
    pub limits: Limits,
    pub pdf_threads: usize,
    pub database_busy_timeout: Duration,
    pub worker_poll_interval: Duration,
    pub recovery_limit: usize,
    pub shutdown_grace: Duration,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Limits {
    pub max_upload_bytes: u64,
    pub max_output_bytes: u64,
    pub max_jobs: usize,
    pub max_concurrent_uploads: usize,
    pub upload_timeout: Duration,
    pub pdf_timeout: Duration,
}

impl Settings {
    pub fn from_env() -> Result<Self, ConfigError> {
        let raw_bind_address =
            read_env_or_default("TOOLKIT_CONVERTER_BIND_ADDR", DEFAULT_BIND_ADDRESS)?;
        let bind_address =
            raw_bind_address
                .parse()
                .map_err(|source| ConfigError::InvalidBindAddress {
                    value: raw_bind_address,
                    source,
                })?;
        let log_filter = read_env_or_default("RUST_LOG", DEFAULT_LOG_FILTER)?;
        EnvFilter::try_new(&log_filter).map_err(|source| ConfigError::InvalidLogFilter {
            value: log_filter.clone(),
            reason: source.to_string(),
        })?;

        let token_file = absolute_path(
            "TOOLKIT_CONVERTER_TOKEN_FILE",
            read_env_or_default("TOOLKIT_CONVERTER_TOKEN_FILE", DEFAULT_TOKEN_FILE)?,
        )?;
        let data_dir = absolute_path(
            "TOOLKIT_CONVERTER_DATA_DIR",
            read_env_or_default("TOOLKIT_CONVERTER_DATA_DIR", DEFAULT_DATA_DIR)?,
        )?;
        let scratch_parent = absolute_path(
            "TOOLKIT_CONVERTER_SCRATCH_PARENT",
            read_env_or_default("TOOLKIT_CONVERTER_SCRATCH_PARENT", DEFAULT_SCRATCH_PARENT)?,
        )?;
        let pdf_worker_path = match read_optional_env("TOOLKIT_CONVERTER_PDF_WORKER_PATH")? {
            Some(path) => absolute_path("TOOLKIT_CONVERTER_PDF_WORKER_PATH", path)?,
            None => sibling_worker_path()?,
        };
        let pdf_bcmaps_dir = read_optional_env("TOOLKIT_CONVERTER_PDF_BCMAPS_DIR")?
            .map(|path| absolute_path("TOOLKIT_CONVERTER_PDF_BCMAPS_DIR", path))
            .transpose()?;

        Ok(Self {
            bind_address,
            log_filter,
            token_file,
            data_dir,
            scratch_parent,
            pdf_worker_path,
            pdf_bcmaps_dir,
            limits: Limits {
                max_upload_bytes: read_bounded_u64(
                    "TOOLKIT_CONVERTER_MAX_UPLOAD_BYTES",
                    DEFAULT_MAX_UPLOAD_BYTES,
                    1024,
                    1024 * 1024 * 1024,
                )?,
                max_output_bytes: read_bounded_u64(
                    "TOOLKIT_CONVERTER_MAX_OUTPUT_BYTES",
                    DEFAULT_MAX_OUTPUT_BYTES,
                    1024,
                    2 * 1024 * 1024 * 1024,
                )?,
                max_jobs: read_bounded_usize(
                    "TOOLKIT_CONVERTER_MAX_JOBS",
                    DEFAULT_MAX_JOBS,
                    1,
                    1024,
                )?,
                max_concurrent_uploads: read_bounded_usize(
                    "TOOLKIT_CONVERTER_MAX_CONCURRENT_UPLOADS",
                    DEFAULT_MAX_CONCURRENT_UPLOADS,
                    1,
                    16,
                )?,
                upload_timeout: Duration::from_secs(read_bounded_u64(
                    "TOOLKIT_CONVERTER_UPLOAD_TIMEOUT_SECS",
                    DEFAULT_UPLOAD_TIMEOUT_SECS,
                    1,
                    3600,
                )?),
                pdf_timeout: Duration::from_secs(read_bounded_u64(
                    "TOOLKIT_CONVERTER_PDF_TIMEOUT_SECS",
                    DEFAULT_PDF_TIMEOUT_SECS,
                    1,
                    3600,
                )?),
            },
            pdf_threads: read_bounded_usize(
                "TOOLKIT_CONVERTER_PDF_THREADS",
                DEFAULT_PDF_THREADS,
                1,
                64,
            )?,
            database_busy_timeout: Duration::from_secs(read_bounded_u64(
                "TOOLKIT_CONVERTER_DATABASE_BUSY_TIMEOUT_SECS",
                DEFAULT_DATABASE_BUSY_TIMEOUT_SECS,
                1,
                60,
            )?),
            worker_poll_interval: Duration::from_secs(read_bounded_u64(
                "TOOLKIT_CONVERTER_WORKER_POLL_INTERVAL_SECS",
                DEFAULT_WORKER_POLL_INTERVAL_SECS,
                1,
                60,
            )?),
            recovery_limit: read_bounded_usize(
                "TOOLKIT_CONVERTER_RECOVERY_LIMIT",
                DEFAULT_RECOVERY_LIMIT,
                0,
                16,
            )?,
            shutdown_grace: Duration::from_secs(read_bounded_u64(
                "TOOLKIT_CONVERTER_SHUTDOWN_GRACE_SECS",
                DEFAULT_SHUTDOWN_GRACE_SECS,
                1,
                600,
            )?),
        })
    }
}

fn sibling_worker_path() -> Result<PathBuf, ConfigError> {
    let executable = env::current_exe().map_err(ConfigError::CurrentExecutable)?;
    let worker_name = if cfg!(windows) {
        "tool-kit-pdf-worker.exe"
    } else {
        "tool-kit-pdf-worker"
    };

    Ok(executable.with_file_name(worker_name))
}

fn absolute_path(variable: &'static str, raw: String) -> Result<PathBuf, ConfigError> {
    let path = PathBuf::from(raw);
    if !path.is_absolute() {
        return Err(ConfigError::PathMustBeAbsolute { variable, path });
    }
    Ok(path)
}

fn read_env_or_default(name: &'static str, default: &str) -> Result<String, ConfigError> {
    Ok(read_optional_env(name)?.unwrap_or_else(|| default.to_owned()))
}

fn read_optional_env(name: &'static str) -> Result<Option<String>, ConfigError> {
    match env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(env::VarError::NotUnicode(_)) => Err(ConfigError::NonUnicodeValue { variable: name }),
    }
}

fn read_bounded_u64(
    name: &'static str,
    default: u64,
    minimum: u64,
    maximum: u64,
) -> Result<u64, ConfigError> {
    let raw = read_optional_env(name)?;
    let value = match raw.as_deref() {
        Some(value) => value
            .parse::<u64>()
            .map_err(|_| ConfigError::InvalidNumber {
                variable: name,
                value: value.to_owned(),
            })?,
        None => default,
    };
    if !(minimum..=maximum).contains(&value) {
        return Err(ConfigError::NumberOutOfRange {
            variable: name,
            value,
            minimum,
            maximum,
        });
    }
    Ok(value)
}

fn read_bounded_usize(
    name: &'static str,
    default: usize,
    minimum: usize,
    maximum: usize,
) -> Result<usize, ConfigError> {
    let value = read_bounded_u64(name, default as u64, minimum as u64, maximum as u64)?;
    usize::try_from(value).map_err(|_| ConfigError::NumberOutOfRange {
        variable: name,
        value,
        minimum: minimum as u64,
        maximum: maximum as u64,
    })
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("TOOLKIT_CONVERTER_BIND_ADDR is not a valid socket address: {value}")]
    InvalidBindAddress {
        value: String,
        #[source]
        source: std::net::AddrParseError,
    },
    #[error("{variable} contains non-Unicode data")]
    NonUnicodeValue { variable: &'static str },
    #[error("RUST_LOG is not a valid tracing filter ({value}): {reason}")]
    InvalidLogFilter { value: String, reason: String },
    #[error("{variable} must be an absolute path, got {path:?}")]
    PathMustBeAbsolute {
        variable: &'static str,
        path: PathBuf,
    },
    #[error("{variable} is not a valid unsigned integer: {value}")]
    InvalidNumber {
        variable: &'static str,
        value: String,
    },
    #[error("{variable}={value} is outside the allowed range {minimum}..={maximum}")]
    NumberOutOfRange {
        variable: &'static str,
        value: u64,
        minimum: u64,
        maximum: u64,
    },
    #[error("cannot determine the current executable path")]
    CurrentExecutable(#[source] std::io::Error),
}

#[cfg(test)]
mod tests {
    use tracing_subscriber::EnvFilter;

    use super::{
        DEFAULT_BIND_ADDRESS, DEFAULT_DATABASE_BUSY_TIMEOUT_SECS, DEFAULT_LOG_FILTER,
        DEFAULT_MAX_OUTPUT_BYTES, DEFAULT_MAX_UPLOAD_BYTES, DEFAULT_RECOVERY_LIMIT,
        DEFAULT_SHUTDOWN_GRACE_SECS, DEFAULT_WORKER_POLL_INTERVAL_SECS,
    };

    #[test]
    fn default_bind_address_is_loopback_only() {
        let address: std::net::SocketAddr = DEFAULT_BIND_ADDRESS.parse().unwrap();

        assert!(address.ip().is_loopback());
        assert_eq!(address.port(), 8080);
    }

    #[test]
    fn default_log_filter_is_valid() {
        assert!(EnvFilter::try_new(DEFAULT_LOG_FILTER).is_ok());
    }

    #[test]
    fn default_limits_fit_the_development_tmpfs() {
        assert_eq!(DEFAULT_MAX_UPLOAD_BYTES, 25 * 1024 * 1024);
        assert_eq!(DEFAULT_MAX_OUTPUT_BYTES, 50 * 1024 * 1024);
    }

    #[test]
    fn default_paths_are_absolute() {
        assert!(std::path::Path::new(super::DEFAULT_TOKEN_FILE).is_absolute());
        assert!(std::path::Path::new(super::DEFAULT_DATA_DIR).is_absolute());
        assert!(std::path::Path::new(super::DEFAULT_SCRATCH_PARENT).is_absolute());
    }

    #[test]
    fn durability_defaults_are_deliberately_bounded() {
        assert_eq!(DEFAULT_DATABASE_BUSY_TIMEOUT_SECS, 5);
        assert_eq!(DEFAULT_WORKER_POLL_INTERVAL_SECS, 1);
        assert_eq!(DEFAULT_RECOVERY_LIMIT, 3);
        assert_eq!(DEFAULT_SHUTDOWN_GRACE_SECS, 30);
    }
}

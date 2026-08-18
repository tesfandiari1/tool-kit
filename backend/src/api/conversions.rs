use std::path::Path as FilePath;

use axum::{
    body::Body,
    extract::{multipart::MultipartRejection, Extension, Multipart, Path, State},
    http::{
        header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_TYPE, ETAG},
        HeaderMap, HeaderName, HeaderValue, StatusCode,
    },
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::{fs::OpenOptions, io::AsyncWriteExt, time::timeout};
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use crate::{
    conversion::{
        source_format_by_extension, ArtifactKind, ArtifactLookup, ArtifactView, ContainerMagic,
        ConversionProfile, JobView, SourceMetadata, Submission, SubmissionDecision,
    },
    error::{ApiError, RequestId},
    AppState,
};

const IDEMPOTENCY_KEY: HeaderName = HeaderName::from_static("idempotency-key");
const IDEMPOTENCY_REPLAYED: HeaderName = HeaderName::from_static("idempotency-replayed");
const X_CONTENT_TYPE_OPTIONS: HeaderName = HeaderName::from_static("x-content-type-options");
const MAX_METADATA_BYTES: usize = 256;
const MAX_FILENAME_BYTES: usize = 255;

pub async fn create(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    headers: HeaderMap,
    multipart: Result<Multipart, MultipartRejection>,
) -> Result<Response, ApiError> {
    let idempotency_key = parse_idempotency_key(&headers, &request_id)?;
    let _upload_permit = state.try_acquire_upload().map_err(|_| {
        error(
            StatusCode::TOO_MANY_REQUESTS,
            "upload_capacity_reached",
            "The service is already staging the maximum number of uploads.",
            &request_id,
        )
    })?;
    let multipart = multipart.map_err(|rejection| {
        let status = rejection.into_response().status();
        if status == StatusCode::PAYLOAD_TOO_LARGE {
            error(
                StatusCode::PAYLOAD_TOO_LARGE,
                "upload_too_large",
                "The upload exceeds the configured limit.",
                &request_id,
            )
        } else {
            error(
                StatusCode::BAD_REQUEST,
                "invalid_multipart",
                "The multipart request is malformed.",
                &request_id,
            )
        }
    })?;

    let job_id = Uuid::new_v4();
    let attempt_id = Uuid::new_v4();
    let prepared = state
        .service()
        .prepare_submission(job_id)
        .await
        .map_err(|_| {
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "scratch_unavailable",
                "The service cannot prepare upload storage.",
                &request_id,
            )
        })?;
    let staged = match timeout(
        state.limits().upload_timeout,
        stage_multipart(
            multipart,
            &prepared.paths.source_staging,
            state.limits().max_upload_bytes,
            &request_id,
        ),
    )
    .await
    {
        Ok(Ok(staged)) => staged,
        Ok(Err(api_error)) => {
            state.service().discard_unaccepted_job(job_id).await;
            return Err(api_error);
        }
        Err(_) => {
            state.service().discard_unaccepted_job(job_id).await;
            return Err(error(
                StatusCode::REQUEST_TIMEOUT,
                "upload_timeout",
                "The upload exceeded its time limit.",
                &request_id,
            ));
        }
    };
    if staged.profile == ConversionProfile::BestQuality {
        state.service().discard_unaccepted_job(job_id).await;
        return Err(error(
            StatusCode::CONFLICT,
            "profile_unavailable",
            "The best_quality profile is not available in this service version.",
            &request_id,
        ));
    }

    let decision = state
        .service()
        .submit(Submission {
            prepared,
            attempt_id,
            client_run_id: staged.client_run_id,
            profile: staged.profile,
            source: staged.source,
            idempotency_key,
            origin_request_id: request_id.as_str().to_owned(),
        })
        .await
        .map_err(|_| service_unavailable(&request_id))?;
    match decision {
        SubmissionDecision::Created(job) => Ok(accepted(job, false)),
        SubmissionDecision::Replay(job) => Ok(accepted(job, true)),
        SubmissionDecision::Conflict => Err(error(
            StatusCode::CONFLICT,
            "idempotency_conflict",
            "The idempotency key was already used for a different submission.",
            &request_id,
        )),
        SubmissionDecision::Capacity => Err(error(
            StatusCode::TOO_MANY_REQUESTS,
            "job_capacity_reached",
            "The service is at its active job capacity; retry later.",
            &request_id,
        )),
    }
}

pub async fn get(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Path(raw_job_id): Path<String>,
) -> Result<Json<JobEnvelope>, ApiError> {
    let job_id = parse_job_id(&raw_job_id, &request_id)?;
    let job = state
        .service()
        .get(job_id)
        .await
        .map_err(|_| service_unavailable(&request_id))?
        .ok_or_else(|| {
            error(
                StatusCode::NOT_FOUND,
                "conversion_not_found",
                "The conversion does not exist.",
                &request_id,
            )
        })?;
    Ok(Json(JobEnvelope { data: job }))
}

pub async fn list_artifacts(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Path(raw_job_id): Path<String>,
) -> Result<Json<ArtifactListEnvelope>, ApiError> {
    let job_id = parse_job_id(&raw_job_id, &request_id)?;
    let artifacts = state
        .service()
        .artifact_views(job_id)
        .await
        .map_err(|_| service_unavailable(&request_id))?
        .ok_or_else(|| {
            error(
                StatusCode::NOT_FOUND,
                "conversion_not_found",
                "The conversion does not exist.",
                &request_id,
            )
        })?;
    Ok(Json(ArtifactListEnvelope { data: artifacts }))
}

pub async fn download_markdown(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Path(raw_job_id): Path<String>,
) -> Result<Response, ApiError> {
    download(state, request_id, raw_job_id, ArtifactKind::Markdown).await
}

pub async fn download_manifest(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Path(raw_job_id): Path<String>,
) -> Result<Response, ApiError> {
    download(state, request_id, raw_job_id, ArtifactKind::Manifest).await
}

async fn download(
    state: AppState,
    request_id: RequestId,
    raw_job_id: String,
    kind: ArtifactKind,
) -> Result<Response, ApiError> {
    let job_id = parse_job_id(&raw_job_id, &request_id)?;
    let lookup = state
        .service()
        .artifact(job_id, kind)
        .await
        .map_err(|_| service_unavailable(&request_id))?
        .ok_or_else(|| {
            error(
                StatusCode::NOT_FOUND,
                "conversion_not_found",
                "The conversion does not exist.",
                &request_id,
            )
        })?;
    let artifact = match lookup {
        ArtifactLookup::Ready(artifact) => artifact,
        ArtifactLookup::NotReady => {
            return Err(error(
                StatusCode::CONFLICT,
                "artifact_not_ready",
                "The artifact is not ready.",
                &request_id,
            ));
        }
        ArtifactLookup::NotFound => {
            return Err(error(
                StatusCode::NOT_FOUND,
                "artifact_not_found",
                "The artifact does not exist for this conversion.",
                &request_id,
            ));
        }
    };
    let extension = match kind {
        ArtifactKind::Markdown => "md",
        ArtifactKind::Manifest => "json",
    };
    let disposition = format!("attachment; filename=\"conversion-{job_id}.{extension}\"");
    let etag = format!("\"sha256-{}\"", artifact.sha256);
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, artifact.media_type)
        .header(CONTENT_LENGTH, artifact.byte_length)
        .header(CONTENT_DISPOSITION, disposition)
        .header(CACHE_CONTROL, "private, no-store")
        .header(X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(ETAG, etag)
        .body(Body::from_stream(ReaderStream::new(artifact.file)))
        .map_err(|_| {
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "artifact_response_failed",
                "The service could not create the artifact response.",
                &request_id,
            )
        })
}

async fn stage_multipart(
    mut multipart: Multipart,
    source_path: &std::path::Path,
    max_upload_bytes: u64,
    request_id: &RequestId,
) -> Result<StagedSubmission, ApiError> {
    let mut client_run_id = None;
    let mut profile = None;
    let mut source = None;
    while let Some(field) = multipart.next_field().await.map_err(|_| {
        error(
            StatusCode::BAD_REQUEST,
            "invalid_multipart",
            "The multipart request is malformed.",
            request_id,
        )
    })? {
        match field.name() {
            Some("clientRunId") if client_run_id.is_none() => {
                let value = read_text(field, request_id).await?;
                client_run_id = Some(Uuid::parse_str(&value).map_err(|_| {
                    error(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        "invalid_client_run_id",
                        "clientRunId must be a UUID.",
                        request_id,
                    )
                })?);
            }
            Some("profile") if profile.is_none() => {
                let value = read_text(field, request_id).await?;
                profile = Some(value.parse::<ConversionProfile>().map_err(|_| {
                    error(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        "invalid_profile",
                        "profile must be standard, local_only, or best_quality.",
                        request_id,
                    )
                })?);
            }
            Some("source") if source.is_none() => {
                source =
                    Some(stream_source(field, source_path, max_upload_bytes, request_id).await?);
            }
            Some("clientRunId" | "profile" | "source") => {
                return Err(error(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "duplicate_multipart_field",
                    "Multipart fields may appear exactly once.",
                    request_id,
                ));
            }
            _ => {
                return Err(error(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "unexpected_multipart_field",
                    "Exactly source, clientRunId, and profile fields are required.",
                    request_id,
                ));
            }
        }
    }

    Ok(StagedSubmission {
        client_run_id: client_run_id.ok_or_else(|| missing_field("clientRunId", request_id))?,
        profile: profile.ok_or_else(|| missing_field("profile", request_id))?,
        source: source.ok_or_else(|| missing_field("source", request_id))?,
    })
}

async fn read_text(
    mut field: axum::extract::multipart::Field<'_>,
    request_id: &RequestId,
) -> Result<String, ApiError> {
    let mut bytes = Vec::new();
    while let Some(chunk) = field.chunk().await.map_err(|_| {
        error(
            StatusCode::BAD_REQUEST,
            "invalid_multipart",
            "The multipart request is malformed.",
            request_id,
        )
    })? {
        if bytes.len().saturating_add(chunk.len()) > MAX_METADATA_BYTES {
            return Err(error(
                StatusCode::UNPROCESSABLE_ENTITY,
                "metadata_too_large",
                "Multipart metadata fields are too large.",
                request_id,
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    String::from_utf8(bytes).map_err(|_| {
        error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_metadata_encoding",
            "Multipart metadata must be UTF-8.",
            request_id,
        )
    })
}

async fn stream_source(
    mut field: axum::extract::multipart::Field<'_>,
    source_path: &std::path::Path,
    max_upload_bytes: u64,
    request_id: &RequestId,
) -> Result<SourceMetadata, ApiError> {
    let filename = field.file_name().ok_or_else(|| {
        error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "missing_filename",
            "The source field must include a filename.",
            request_id,
        )
    })?;
    let format = if filename.len() > MAX_FILENAME_BYTES || filename.chars().any(char::is_control) {
        None
    } else {
        FilePath::new(filename)
            .extension()
            .and_then(|extension| extension.to_str())
            .and_then(source_format_by_extension)
    };
    let Some(format) = format else {
        return Err(error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported_source_extension",
            "The source filename must use a supported extension.",
            request_id,
        ));
    };
    if field
        .content_type()
        .is_none_or(|content_type| !content_type.eq_ignore_ascii_case(format.media_type))
    {
        return Err(error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "invalid_source_media_type",
            "The source media type does not match its extension.",
            request_id,
        ));
    }

    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        options.mode(0o600);
    }
    let mut file = options.open(source_path).await.map_err(|_| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "scratch_unavailable",
            "The service cannot store the upload.",
            request_id,
        )
    })?;
    let mut digest = Sha256::new();
    let mut byte_length = 0_u64;
    let mut signature = Vec::with_capacity(1024);
    while let Some(chunk) = field.chunk().await.map_err(|_| {
        error(
            StatusCode::BAD_REQUEST,
            "upload_interrupted",
            "The source upload was interrupted.",
            request_id,
        )
    })? {
        byte_length = byte_length.checked_add(chunk.len() as u64).ok_or_else(|| {
            error(
                StatusCode::PAYLOAD_TOO_LARGE,
                "upload_too_large",
                "The upload exceeds the configured limit.",
                request_id,
            )
        })?;
        if byte_length > max_upload_bytes {
            return Err(error(
                StatusCode::PAYLOAD_TOO_LARGE,
                "upload_too_large",
                "The upload exceeds the configured limit.",
                request_id,
            ));
        }
        let remaining = 1024_usize.saturating_sub(signature.len());
        signature.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
        digest.update(&chunk);
        file.write_all(&chunk).await.map_err(|_| {
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "scratch_write_failed",
                "The service could not store the upload.",
                request_id,
            )
        })?;
    }
    if byte_length == 0 || !has_container_magic(format.magic, &signature) {
        return Err(error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "invalid_source_signature",
            "The source content does not match its extension.",
            request_id,
        ));
    }
    file.sync_all().await.map_err(|_| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "scratch_write_failed",
            "The service could not store the upload.",
            request_id,
        )
    })?;
    Ok(SourceMetadata {
        media_type: format.media_type.to_owned(),
        byte_length,
        sha256: hex::encode(digest.finalize()),
    })
}

/// The cheap admission check: the container signature each format family
/// carries in its first bytes. Authoritative format detection happens from
/// the full content inside the engine.
fn has_container_magic(magic: ContainerMagic, prefix: &[u8]) -> bool {
    match magic {
        ContainerMagic::Pdf => has_pdf_signature(prefix),
        ContainerMagic::Zip => prefix.starts_with(b"PK\x03\x04"),
        ContainerMagic::Ole => {
            prefix.starts_with(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1])
        }
        ContainerMagic::Rtf => prefix.starts_with(b"{\\rtf"),
        // CSV has no signature to check. Admission rests on the extension and
        // the declared media type; the engine still has the final say.
        ContainerMagic::None => true,
    }
}

fn has_pdf_signature(prefix: &[u8]) -> bool {
    let without_bom = prefix.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(prefix);
    let trimmed = without_bom
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .map(|index| &without_bom[index..])
        .unwrap_or_default();
    trimmed.starts_with(b"%PDF-")
}

fn parse_idempotency_key(headers: &HeaderMap, request_id: &RequestId) -> Result<String, ApiError> {
    let mut values = headers.get_all(&IDEMPOTENCY_KEY).iter();
    let value = values.next().ok_or_else(|| {
        error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "missing_idempotency_key",
            "Idempotency-Key is required.",
            request_id,
        )
    })?;
    if values.next().is_some() {
        return Err(error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_idempotency_key",
            "Idempotency-Key must appear exactly once.",
            request_id,
        ));
    }
    let value = value.to_str().map_err(|_| {
        error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_idempotency_key",
            "Idempotency-Key contains invalid characters.",
            request_id,
        )
    })?;
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._~-".contains(&byte))
    {
        return Err(error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_idempotency_key",
            "Idempotency-Key must be 1-128 URL-safe characters.",
            request_id,
        ));
    }
    Ok(value.to_owned())
}

fn parse_job_id(raw: &str, request_id: &RequestId) -> Result<Uuid, ApiError> {
    Uuid::parse_str(raw).map_err(|_| {
        error(
            StatusCode::NOT_FOUND,
            "conversion_not_found",
            "The conversion does not exist.",
            request_id,
        )
    })
}

fn accepted(job: JobView, replayed: bool) -> Response {
    let location = format!("/api/v1/conversions/{}", job.id);
    let mut response = (StatusCode::ACCEPTED, Json(JobEnvelope { data: job })).into_response();
    response.headers_mut().insert(
        "location",
        HeaderValue::from_str(&location).expect("UUID locations are valid"),
    );
    response
        .headers_mut()
        .insert("retry-after", HeaderValue::from_static("1"));
    response.headers_mut().insert(
        IDEMPOTENCY_REPLAYED,
        HeaderValue::from_static(if replayed { "true" } else { "false" }),
    );
    response
}

fn missing_field(field: &'static str, request_id: &RequestId) -> ApiError {
    let (code, message) = match field {
        "clientRunId" => ("missing_client_run_id", "clientRunId is required."),
        "profile" => ("missing_profile", "profile is required."),
        _ => ("missing_source", "source is required."),
    };
    error(StatusCode::UNPROCESSABLE_ENTITY, code, message, request_id)
}

fn error(
    status: StatusCode,
    code: &'static str,
    message: &'static str,
    request_id: &RequestId,
) -> ApiError {
    ApiError::new(status, code, message, request_id.clone())
}

fn service_unavailable(request_id: &RequestId) -> ApiError {
    error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "conversion_unavailable",
        "The conversion service is temporarily unavailable.",
        request_id,
    )
}

struct StagedSubmission {
    client_run_id: Uuid,
    profile: ConversionProfile,
    source: SourceMetadata,
}

#[derive(Debug, Serialize)]
pub(super) struct JobEnvelope {
    data: JobView,
}

#[derive(Debug, Serialize)]
pub(super) struct ArtifactListEnvelope {
    data: Vec<ArtifactView>,
}

#[cfg(test)]
mod tests {
    use super::has_pdf_signature;

    #[test]
    fn pdf_signature_allows_only_bom_or_whitespace_prefixes() {
        assert!(has_pdf_signature(b"%PDF-1.7\n"));
        assert!(has_pdf_signature(b"\xEF\xBB\xBF \n%PDF-1.4"));
        assert!(!has_pdf_signature(b"PK\x03\x04%PDF-1.7"));
        assert!(!has_pdf_signature(b"not a pdf"));
    }
}

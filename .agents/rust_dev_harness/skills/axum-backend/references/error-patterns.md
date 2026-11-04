# Axum Error Patterns

Non-normative implementation reference. Governing Error and Response Rules remain authority.

## 1. Keep semantic errors transport-neutral

Application/domain example:

```rust
#[derive(Debug, thiserror::Error)]
pub enum CreateUserError {
    #[error("email already exists")]
    EmailAlreadyExists,

    #[error("user store unavailable")]
    StoreUnavailable,
}
```

Do not add `StatusCode::CONFLICT` to the semantic error merely because the first adapter is HTTP.

## 2. HTTP mapping owner

A transport error can preserve stable client semantics while logging internal causes separately.

Conceptual shape:

```rust
pub enum HttpError {
    BadRequest { code: &'static str, message: String },
    Unauthorized,
    Forbidden,
    NotFound,
    Conflict { code: &'static str },
    TooManyRequests,
    Internal { error: anyhow::Error },
}
```

The exact type is repository-specific. A giant enum is not mandatory.

## 3. `IntoResponse` pattern

```rust
#[derive(Serialize)]
struct ErrorBody {
    code: &'static str,
    message: String,
}

impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        let (status, body) = match self {
            HttpError::BadRequest { code, message } => (
                StatusCode::BAD_REQUEST,
                ErrorBody { code, message },
            ),
            HttpError::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                ErrorBody {
                    code: "unauthorized",
                    message: "authentication required".to_owned(),
                },
            ),
            HttpError::Forbidden => (
                StatusCode::FORBIDDEN,
                ErrorBody {
                    code: "forbidden",
                    message: "operation not permitted".to_owned(),
                },
            ),
            HttpError::NotFound => (
                StatusCode::NOT_FOUND,
                ErrorBody {
                    code: "not_found",
                    message: "resource not found".to_owned(),
                },
            ),
            HttpError::Conflict { code } => (
                StatusCode::CONFLICT,
                ErrorBody {
                    code,
                    message: "request conflicts with current state".to_owned(),
                },
            ),
            HttpError::TooManyRequests => (
                StatusCode::TOO_MANY_REQUESTS,
                ErrorBody {
                    code: "overloaded",
                    message: "request cannot be accepted now".to_owned(),
                },
            ),
            HttpError::Internal { error } => {
                tracing::error!(error = ?error, "request failed");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ErrorBody {
                        code: "internal",
                        message: "internal server error".to_owned(),
                    },
                )
            }
        };

        (status, Json(body)).into_response()
    }
}
```

This demonstrates the separation:

```text
trusted telemetry = detailed cause
client response    = stable safe category/message
```

Do not log secrets or entire sensitive inputs with the cause chain.

## 4. Mapping semantic errors

Prefer explicit conversion at the HTTP adapter:

```rust
impl From<CreateUserError> for HttpError {
    fn from(error: CreateUserError) -> Self {
        match error {
            CreateUserError::EmailAlreadyExists => HttpError::Conflict {
                code: "email_exists",
            },
            CreateUserError::StoreUnavailable => HttpError::Internal {
                error: error.into(),
            },
        }
    }
}
```

If the semantic error has multiple transport interpretations, keep the mapping adapter-specific instead of giving the domain one universal HTTP conversion.

## 5. Extractor rejection normalization

Built-in Axum extractors can reject before the handler runs. If clients require one error envelope, normalize these at the transport layer.

Possible approaches vary by extractor/version:

- custom extractor wrapping built-in behavior;
- route/middleware-level rejection handling where appropriate;
- custom body/request DTO extraction;
- explicitly accepting Axum defaults for endpoints where stability is unimportant.

Do not assume handler `Result<_, HttpError>` catches extraction failures that occur before the handler is called.

## 6. Middleware errors

Custom Tower services/layers can have a fallible `Error` channel. Axum routing expects response-producing behavior, so convert the error before it reaches the app boundary:

```text
fallible Tower service
→ HandleErrorLayer / equivalent mapping
→ HTTP Response
→ Axum
```

Some tower-http layers already produce responses and do not need this pattern.

## 7. Timeout and overload errors

Distinguish:

```text
timeout while waiting
concurrency admission rejected
temporary downstream unavailable
rate limit exceeded
permanent validation failure
```

Whether these map to 408/429/503/504 or another contract is API policy. The important property is deliberate stable semantics.

For automated retries, status alone is insufficient: consider idempotency and whether a side effect may already have committed.

## 8. Correlation IDs

For operationally important APIs, include a request/trace identifier in the response header and/or error envelope when useful:

```json
{
  "code": "internal",
  "message": "internal server error",
  "request_id": "..."
}
```

The identifier must be bounded/safe and must not itself reveal secret/user data.

## 9. Error tests

Test behavior, not only variants:

```text
semantic conflict → correct status + code
malformed JSON → canonical client error
unauthorized → no sensitive auth detail
internal DB error → sanitized 5xx + observable internal cause
fallible middleware → mapped response, not service crash
```

## 10. Smells

- `Json(json!({"error": format!("{err:?}")}))` on public 500s;
- handlers repeat their own status mapping for the same semantic error;
- domain enums contain `StatusCode` without domain justification;
- all errors become `500`;
- all client errors become `400`;
- retries are triggered by `is_server_error()` with no idempotency model;
- extractor rejections use one body shape while handler errors use another despite a promised stable API.

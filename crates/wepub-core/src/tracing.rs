use tracing::{Instrument, Level, Span, debug, error, info, trace, warn};

use crate::{Result, WepubError};

pub(crate) async fn instrument_step<T>(
    span: Span,
    error_level: Level,
    fut: impl std::future::Future<Output = Result<T>>,
) -> Result<T> {
    async move { fut.await.inspect_err(|err| record_error(error_level, err)) }
        .instrument(span)
        .await
}

fn record_error(level: Level, err: &WepubError) {
    macro_rules! record {
        ($($args:tt)*) => {
            match level {
                Level::TRACE => trace!($($args)*),
                Level::DEBUG => debug!($($args)*),
                Level::INFO => info!($($args)*),
                Level::WARN => warn!($($args)*),
                Level::ERROR => error!($($args)*),
            }
        };
    }
    match err {
        WepubError::Http { source } => {
            let source = source.to_string();
            record!(source = source.as_str(), "{err}");
        }
        WepubError::HttpStatus { status, body } => {
            record!(status = *status, body = body.as_str(), "{err}");
        }
        WepubError::OAuthToken {
            error,
            error_description,
            error_uri,
        } => {
            record!(
                error = error.as_str(),
                error_description = error_description.as_deref(),
                error_uri = error_uri.as_deref(),
                "{err}",
            );
        }
        WepubError::PollTimeout { elapsed } => {
            record!(elapsed_secs = elapsed.as_secs_f64(), "{err}");
        }
        WepubError::UnexpectedResponse { reason } => {
            record!(reason = reason.as_str(), "{err}");
        }
        WepubError::ChromeUpload { upload_state } => {
            record!(upload_state = upload_state.as_str(), "{err}");
        }
        WepubError::ChromePublish { item_state } => {
            record!(item_state = item_state.as_str(), "{err}");
        }
        WepubError::FirefoxUpload { validation } => {
            let validation = serde_json::to_string(validation).unwrap_or_default();
            record!(validation = validation.as_str(), "{err}");
        }
        WepubError::EdgeApi {
            message,
            error_code,
            errors,
        } => {
            let errors = errors
                .as_ref()
                .map(|errors| serde_json::to_string(errors).unwrap_or_default());
            record!(
                message = message.as_deref(),
                error_code = error_code.as_deref(),
                errors = errors.as_deref(),
                "{err}",
            );
        }
    }
}

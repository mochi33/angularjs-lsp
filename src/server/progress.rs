use tower_lsp::lsp_types::*;
use tower_lsp::lsp_types::notification;
use tower_lsp::Client;

/// Send progress notification helper
pub async fn send_progress(client: &Client, token: &NumberOrString, value: WorkDoneProgress) {
    let params = ProgressParams {
        token: token.clone(),
        value: ProgressParamsValue::WorkDone(value),
    };
    client
        .send_notification::<notification::Progress>(params)
        .await;
}

/// Create a progress token and send begin notification
pub async fn begin_progress(
    client: &Client,
    token_name: &str,
    title: &str,
    message: Option<String>,
) -> NumberOrString {
    let token = NumberOrString::String(token_name.to_string());
    let _ = client
        .send_request::<request::WorkDoneProgressCreate>(WorkDoneProgressCreateParams {
            token: token.clone(),
        })
        .await;

    send_progress(
        client,
        &token,
        WorkDoneProgress::Begin(WorkDoneProgressBegin {
            title: title.to_string(),
            cancellable: Some(false),
            message,
            percentage: Some(0),
        }),
    )
    .await;

    token
}

/// Send progress report
pub async fn report_progress(
    client: &Client,
    token: &NumberOrString,
    message: String,
    percentage: u32,
) {
    send_progress(
        client,
        token,
        WorkDoneProgress::Report(WorkDoneProgressReport {
            cancellable: Some(false),
            message: Some(message),
            percentage: Some(percentage),
        }),
    )
    .await;
}

/// Send progress end notification
pub async fn end_progress(client: &Client, token: &NumberOrString, message: String) {
    send_progress(
        client,
        token,
        WorkDoneProgress::End(WorkDoneProgressEnd {
            message: Some(message),
        }),
    )
    .await;
}

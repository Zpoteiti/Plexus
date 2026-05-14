use crate::{
    auth::AuthUser,
    chat::content::{ContentBlock, normalize_user_content},
    db::{messages, sessions},
    error::ApiError,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{
        IntoResponse, Response,
        sse::{KeepAlive, Sse},
    },
};
use plexus_common::ErrorCode;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use uuid::Uuid;

#[derive(Deserialize)]
pub struct SessionListQuery {
    limit: Option<i64>,
    offset: Option<i64>,
}

#[derive(Deserialize)]
pub struct CreateSessionRequest {
    title: Option<String>,
}

#[derive(Deserialize)]
pub struct RenameSessionRequest {
    title: String,
}

#[derive(Deserialize)]
pub struct MessageHistoryQuery {
    before: Option<Uuid>,
    limit: Option<i64>,
}

#[derive(Deserialize)]
pub struct StreamQuery {
    replay_limit: Option<i64>,
}

pub async fn list_sessions(
    auth: AuthUser,
    State(state): State<crate::app::AppState>,
    Query(query): Query<SessionListQuery>,
) -> Result<Json<Vec<sessions::Session>>, ApiError> {
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let offset = query.offset.unwrap_or(0).max(0);
    let rows = sessions::list_for_user(state.pool(), auth.user.id, limit, offset)
        .await
        .map_err(ApiError::from_sqlx)?;
    Ok(Json(rows))
}

pub async fn create_session(
    auth: AuthUser,
    State(state): State<crate::app::AppState>,
    Json(req): Json<CreateSessionRequest>,
) -> Result<(StatusCode, Json<sessions::Session>), ApiError> {
    let title =
        sessions::normalize_create_title(req.title.as_deref()).map_err(ApiError::invalid_args)?;
    let session = sessions::create_web_session(state.pool(), auth.user.id, &title)
        .await
        .map_err(ApiError::from_sqlx)?;
    Ok((StatusCode::CREATED, Json(session)))
}

pub async fn get_session(
    auth: AuthUser,
    State(state): State<crate::app::AppState>,
    Path(session_id): Path<Uuid>,
) -> Result<Json<sessions::Session>, ApiError> {
    let session = owned_session_or_404(state.pool(), auth.user.id, session_id).await?;
    Ok(Json(session))
}

pub async fn rename_session(
    auth: AuthUser,
    State(state): State<crate::app::AppState>,
    Path(session_id): Path<Uuid>,
    Json(req): Json<RenameSessionRequest>,
) -> Result<Json<sessions::Session>, ApiError> {
    let title = sessions::normalize_rename_title(&req.title).map_err(ApiError::invalid_args)?;
    let session = sessions::rename_owned(state.pool(), auth.user.id, session_id, &title)
        .await
        .map_err(ApiError::from_sqlx)?
        .ok_or_else(not_found)?;
    Ok(Json(session))
}

pub async fn delete_session(
    auth: AuthUser,
    State(state): State<crate::app::AppState>,
    Path(session_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let deleted = sessions::delete_owned(state.pool(), auth.user.id, session_id)
        .await
        .map_err(ApiError::from_sqlx)?;
    if !deleted {
        return Err(not_found());
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_messages(
    auth: AuthUser,
    State(state): State<crate::app::AppState>,
    Path(session_id): Path<Uuid>,
    Query(query): Query<MessageHistoryQuery>,
) -> Result<Json<Vec<messages::Message>>, ApiError> {
    let _session = owned_session_or_404(state.pool(), auth.user.id, session_id).await?;
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let rows = messages::list_before(state.pool(), session_id, query.before, limit)
        .await
        .map_err(ApiError::from_sqlx)?;
    Ok(Json(rows))
}

pub async fn post_message(
    auth: AuthUser,
    State(state): State<crate::app::AppState>,
    Path(session_id): Path<Uuid>,
    Json(body): Json<Map<String, Value>>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let session = owned_session_or_404(state.pool(), auth.user.id, session_id).await?;
    if session.channel != sessions::WEB_CHANNEL {
        return Err(ApiError::invalid_args(
            "browser REST can only write to web sessions",
        ));
    }

    let mut content = vec![runtime_block(&session)];
    content.extend(normalize_user_content(body.get("content").cloned())?);

    let message = messages::insert_message(state.pool(), session.id, "user", content)
        .await
        .map_err(ApiError::from_sqlx)?;
    sessions::touch_last_inbound(state.pool(), session.id)
        .await
        .map_err(ApiError::from_sqlx)?;
    state.chat().broker().broadcast(message.clone()).await;
    crate::chat::worker::spawn_response_worker(state.clone(), session.id);

    Ok((
        StatusCode::ACCEPTED,
        Json(json!({ "message_id": message.id })),
    ))
}

pub async fn stream_session(
    auth: AuthUser,
    State(state): State<crate::app::AppState>,
    Path(session_id): Path<Uuid>,
    Query(query): Query<StreamQuery>,
    headers: axum::http::HeaderMap,
) -> Result<Response, ApiError> {
    let _session = owned_session_or_404(state.pool(), auth.user.id, session_id).await?;
    let mut receiver = state.chat().broker().subscribe(session_id).await;
    let replay_limit = query.replay_limit.unwrap_or(50).clamp(0, 200);
    let last_event_id = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| Uuid::parse_str(value).ok());

    let replay = if let Some(last_event_id) = last_event_id {
        messages::replay_after(state.pool(), session_id, last_event_id)
            .await
            .map_err(ApiError::from_sqlx)?
    } else if replay_limit == 0 {
        Vec::new()
    } else {
        messages::replay_recent(state.pool(), session_id, replay_limit)
            .await
            .map_err(ApiError::from_sqlx)?
    };

    let stream = async_stream::stream! {
        for message in replay {
            yield crate::chat::sse::message_event(&message);
        }
        yield crate::chat::sse::history_end_event();
        loop {
            match receiver.recv().await {
                Ok(message) => yield crate::chat::sse::message_event(&message),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    };

    Ok(Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response())
}

pub async fn owned_session_or_404(
    pool: &sqlx::PgPool,
    user_id: Uuid,
    session_id: Uuid,
) -> Result<sessions::Session, ApiError> {
    sessions::find_owned(pool, user_id, session_id)
        .await
        .map_err(ApiError::from_sqlx)?
        .ok_or_else(not_found)
}

fn not_found() -> ApiError {
    ApiError::new(
        StatusCode::NOT_FOUND,
        ErrorCode::NotFound,
        "session not found",
    )
}

fn runtime_block(session: &sessions::Session) -> ContentBlock {
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    ContentBlock::text(format!(
        "<runtime>\ntime_unix: {now}\nchannel: {}\nchat_id: {}\n</runtime>",
        session.channel, session.chat_id
    ))
}

#![allow(missing_docs)]

mod common;

use std::time::Duration;

use axum::body::{Body, Bytes};
use futures_util::StreamExt;
use http::{Request, StatusCode};
use orka_core::{
    Conversation, ConversationId, ConversationMessage, ConversationMessageRole, MessageId, Payload,
    SessionId, StreamChunk, StreamChunkKind,
    traits::{ArtifactStore, ConversationStore, MessageBus},
};
use orka_server::router::MobileStreamEvent;
use tower::ServiceExt;

const JWT_SECRET: &str = "test-secret-key-at-least-32-bytes-long!";
const JWT_ISSUER: &str = "orka-tests";

fn bearer(user_id: &str, scopes: &[&str]) -> String {
    format!(
        "Bearer {}",
        common::make_jwt(JWT_SECRET, JWT_ISSUER, user_id, scopes)
    )
}

async fn seed_conversation(
    store: &dyn ConversationStore,
    user_id: &str,
    title: &str,
    updated_at: chrono::DateTime<chrono::Utc>,
) -> common::TestResult<Conversation> {
    let conversation_id = ConversationId::new();
    let mut conversation = Conversation::new(
        conversation_id,
        SessionId::from(conversation_id),
        user_id,
        title,
    );
    conversation.updated_at = updated_at;
    store.put_conversation(&conversation).await?;
    Ok(conversation)
}

async fn next_sse_frame<S>(stream: &mut S, buffer: &mut String) -> common::TestResult<String>
where
    S: futures_util::Stream<Item = Result<Bytes, axum::Error>> + Unpin,
{
    loop {
        if let Some(pos) = buffer.find("\n\n") {
            let frame = buffer[..pos].to_string();
            buffer.drain(..pos + 2);
            return Ok(frame);
        }

        let next = tokio::time::timeout(Duration::from_secs(1), stream.next()).await?;
        let chunk = next.ok_or_else(|| "SSE stream ended unexpectedly".to_string())??;
        buffer.push_str(std::str::from_utf8(&chunk)?);
    }
}

fn parse_sse_frame(frame: &str) -> common::TestResult<(String, serde_json::Value)> {
    let mut event = None;
    let mut data = None;
    for line in frame.lines() {
        if let Some(value) = line.strip_prefix("event:") {
            event = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("data:") {
            data = Some(serde_json::from_str::<serde_json::Value>(value.trim())?);
        }
    }

    Ok((
        event.ok_or_else(|| "missing SSE event name".to_string())?,
        data.ok_or_else(|| "missing SSE data".to_string())?,
    ))
}

fn multipart_upload_request(
    path: &str,
    authorization: String,
    filename: &str,
    content_type: &str,
    bytes: &[u8],
    caption: Option<&str>,
) -> common::TestResult<Request<Body>> {
    let boundary = "orka-mobile-test-boundary";
    let mut body = Vec::new();

    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        format!(
            "Content-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(format!("Content-Type: {content_type}\r\n\r\n").as_bytes());
    body.extend_from_slice(bytes);
    body.extend_from_slice(b"\r\n");

    if let Some(caption) = caption {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(b"Content-Disposition: form-data; name=\"caption\"\r\n\r\n");
        body.extend_from_slice(caption.as_bytes());
        body.extend_from_slice(b"\r\n");
    }

    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());

    common::request(
        Request::builder()
            .method("POST")
            .uri(path)
            .header("Authorization", authorization)
            .header(
                "content-type",
                format!("multipart/form-data; boundary={boundary}"),
            ),
        Body::from(body),
    )
}

#[tokio::test]
async fn mobile_me_returns_authenticated_identity() -> common::TestResult {
    let ctx = common::test_mobile_router_with_jwt(JWT_SECRET, JWT_ISSUER);
    let req = common::request(
        Request::builder().uri("/mobile/v1/me").header(
            "Authorization",
            bearer("user-123", &["chat:read", "chat:write"]),
        ),
        Body::empty(),
    )?;
    let resp = ctx.app.oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let json = common::json_body(resp).await?;
    assert_eq!(json["user_id"], "user-123");
    assert_eq!(
        json["scopes"],
        serde_json::json!(["chat:read", "chat:write"])
    );
    Ok(())
}

#[tokio::test]
async fn mobile_conversation_list_supports_pagination() -> common::TestResult {
    let ctx = common::test_mobile_router_with_jwt(JWT_SECRET, JWT_ISSUER);
    let now = chrono::Utc::now();
    let _older = seed_conversation(
        ctx.conversations.as_ref(),
        "user-a",
        "older",
        now - chrono::Duration::seconds(20),
    )
    .await?;
    let middle = seed_conversation(
        ctx.conversations.as_ref(),
        "user-a",
        "middle",
        now - chrono::Duration::seconds(10),
    )
    .await?;
    let newest = seed_conversation(ctx.conversations.as_ref(), "user-a", "newest", now).await?;
    let _other_user =
        seed_conversation(ctx.conversations.as_ref(), "user-b", "hidden", now).await?;

    let req = common::request(
        Request::builder()
            .uri("/mobile/v1/conversations?limit=1&offset=1")
            .header("Authorization", bearer("user-a", &["chat:read"])),
        Body::empty(),
    )?;
    let resp = ctx.app.oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let json = common::json_body(resp).await?;
    let array = common::json_array(&json)?;
    assert_eq!(array.len(), 1);
    assert_eq!(array[0]["id"], middle.id.to_string());
    assert_ne!(array[0]["id"], newest.id.to_string());
    Ok(())
}

#[tokio::test]
async fn mobile_message_list_supports_limit_and_offset() -> common::TestResult {
    let ctx = common::test_mobile_router_with_jwt(JWT_SECRET, JWT_ISSUER);
    let conversation = seed_conversation(
        ctx.conversations.as_ref(),
        "user-a",
        "thread",
        chrono::Utc::now(),
    )
    .await?;
    for index in 0..4 {
        ctx.conversations
            .append_message(&ConversationMessage::new(
                MessageId::new(),
                conversation.id,
                conversation.session_id,
                ConversationMessageRole::User,
                format!("message-{index}"),
            ))
            .await?;
    }

    let req = common::request(
        Request::builder()
            .uri(format!(
                "/mobile/v1/conversations/{}/messages?limit=2&offset=1",
                conversation.id
            ))
            .header("Authorization", bearer("user-a", &["chat:read"])),
        Body::empty(),
    )?;
    let resp = ctx.app.oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let json = common::json_body(resp).await?;
    let array = common::json_array(&json)?;
    assert_eq!(array.len(), 2);
    assert_eq!(array[0]["text"], "message-1");
    assert_eq!(array[1]["text"], "message-2");
    Ok(())
}

#[tokio::test]
async fn mobile_send_persists_user_message_and_publishes_inbound_envelope() -> common::TestResult {
    let ctx = common::test_mobile_router_with_jwt(JWT_SECRET, JWT_ISSUER);
    let conversation = seed_conversation(
        ctx.conversations.as_ref(),
        "user-a",
        "thread",
        chrono::Utc::now(),
    )
    .await?;
    let mut inbound = ctx.bus.subscribe("inbound").await?;

    let req = common::request(
        Request::builder()
            .method("POST")
            .uri(format!(
                "/mobile/v1/conversations/{}/messages",
                conversation.id
            ))
            .header("Authorization", bearer("user-a", &["chat:write"]))
            .header("content-type", "application/json"),
        Body::from(r#"{"text":"hello from mobile"}"#),
    )?;
    let resp = ctx.app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::ACCEPTED);

    let envelope = tokio::time::timeout(Duration::from_secs(1), inbound.recv())
        .await?
        .ok_or_else(|| "missing inbound envelope".to_string())?;
    assert_eq!(envelope.session_id, conversation.session_id);
    assert_eq!(
        envelope.metadata.get("user_id"),
        Some(&serde_json::json!("user-a"))
    );

    let messages = ctx
        .conversations
        .list_messages(&conversation.id, None, 0)
        .await?;
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].text, "hello from mobile");
    assert_eq!(messages[0].role, ConversationMessageRole::User);
    Ok(())
}

#[tokio::test]
async fn mobile_upload_returns_metadata_and_content() -> common::TestResult {
    let ctx = common::test_mobile_router_with_jwt(JWT_SECRET, JWT_ISSUER);
    let file_bytes = b"artifact-body";
    let upload_req = multipart_upload_request(
        "/mobile/v1/uploads",
        bearer("user-a", &["chat:write"]),
        "notes.txt",
        "text/plain",
        file_bytes,
        Some("Field notes"),
    )?;

    let upload_resp = ctx.app.clone().oneshot(upload_req).await?;
    assert_eq!(upload_resp.status(), StatusCode::CREATED);
    let upload_json = common::json_body(upload_resp).await?;
    let artifact_id = upload_json["artifact"]["id"]
        .as_str()
        .ok_or_else(|| "artifact id missing".to_string())?;
    assert_eq!(upload_json["artifact"]["mime_type"], "text/plain");
    assert_eq!(upload_json["artifact"]["filename"], "notes.txt");
    assert_eq!(upload_json["artifact"]["caption"], "Field notes");
    assert_eq!(upload_json["artifact"]["size_bytes"], file_bytes.len());

    let metadata_req = common::request(
        Request::builder()
            .uri(format!("/mobile/v1/artifacts/{artifact_id}"))
            .header("Authorization", bearer("user-a", &["chat:read"])),
        Body::empty(),
    )?;
    let metadata_resp = ctx.app.clone().oneshot(metadata_req).await?;
    assert_eq!(metadata_resp.status(), StatusCode::OK);
    let metadata_json = common::json_body(metadata_resp).await?;
    assert_eq!(metadata_json["id"], artifact_id);
    assert_eq!(metadata_json["conversation_id"], serde_json::Value::Null);
    assert_eq!(metadata_json["message_id"], serde_json::Value::Null);

    let content_req = common::request(
        Request::builder()
            .uri(format!("/mobile/v1/artifacts/{artifact_id}/content"))
            .header("Authorization", bearer("user-a", &["chat:read"])),
        Body::empty(),
    )?;
    let content_resp = ctx.app.clone().oneshot(content_req).await?;
    assert_eq!(content_resp.status(), StatusCode::OK);
    assert_eq!(
        content_resp
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("text/plain")
    );
    let content_body = axum::body::to_bytes(content_resp.into_body(), usize::MAX).await?;
    assert_eq!(content_body.as_ref(), file_bytes);

    let range_req = common::request(
        Request::builder()
            .uri(format!("/mobile/v1/artifacts/{artifact_id}/content"))
            .header("Authorization", bearer("user-a", &["chat:read"]))
            .header("Range", "bytes=0-6"),
        Body::empty(),
    )?;
    let range_resp = ctx.app.clone().oneshot(range_req).await?;
    assert_eq!(range_resp.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        range_resp
            .headers()
            .get("content-range")
            .and_then(|value| value.to_str().ok()),
        Some("bytes 0-6/13")
    );
    let range_body = axum::body::to_bytes(range_resp.into_body(), usize::MAX).await?;
    assert_eq!(range_body.as_ref(), b"artifac");

    Ok(())
}

#[tokio::test]
async fn mobile_send_with_artifacts_persists_attachment_and_publishes_rich_input() -> common::TestResult
{
    let ctx = common::test_mobile_router_with_jwt(JWT_SECRET, JWT_ISSUER);
    let conversation = seed_conversation(
        ctx.conversations.as_ref(),
        "user-a",
        "thread",
        chrono::Utc::now(),
    )
    .await?;
    let upload_req = multipart_upload_request(
        "/mobile/v1/uploads",
        bearer("user-a", &["chat:write"]),
        "notes.txt",
        "text/plain",
        b"artifact-body",
        Some("Draft attachment"),
    )?;
    let upload_resp = ctx.app.clone().oneshot(upload_req).await?;
    let upload_json = common::json_body(upload_resp).await?;
    let artifact_id = upload_json["artifact"]["id"]
        .as_str()
        .ok_or_else(|| "artifact id missing".to_string())?;

    let mut inbound = ctx.bus.subscribe("inbound").await?;
    let send_req = common::request(
        Request::builder()
            .method("POST")
            .uri(format!(
                "/mobile/v1/conversations/{}/messages",
                conversation.id
            ))
            .header("Authorization", bearer("user-a", &["chat:write"]))
            .header("content-type", "application/json"),
        Body::from(format!(
            r#"{{"text":"hello with file","artifact_ids":["{artifact_id}"]}}"#
        )),
    )?;
    let send_resp = ctx.app.clone().oneshot(send_req).await?;
    assert_eq!(send_resp.status(), StatusCode::ACCEPTED);
    let send_json = common::json_body(send_resp).await?;
    let message_id = send_json["message_id"]
        .as_str()
        .ok_or_else(|| "message id missing".to_string())?;

    let envelope = tokio::time::timeout(Duration::from_secs(1), inbound.recv())
        .await?
        .ok_or_else(|| "missing inbound envelope".to_string())?;
    match &envelope.payload {
        Payload::RichInput(input) => {
            assert_eq!(input.text.as_deref(), Some("hello with file"));
            assert_eq!(input.attachments.len(), 1);
            assert_eq!(input.attachments[0].mime_type, "text/plain");
            assert_eq!(input.attachments[0].caption.as_deref(), Some("Draft attachment"));
            assert_eq!(input.attachments[0].filename.as_deref(), Some("notes.txt"));
            assert_eq!(input.attachments[0].decode_data(), Some(b"artifact-body".to_vec()));
        }
        payload => panic!("expected rich input payload, got {payload:?}"),
    }

    let messages = ctx
        .conversations
        .list_messages(&conversation.id, None, 0)
        .await?;
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].text, "hello with file");
    assert_eq!(messages[0].artifacts.len(), 1);
    assert_eq!(messages[0].artifacts[0].filename, "notes.txt");
    assert_eq!(messages[0].artifacts[0].message_id.unwrap().to_string(), message_id);

    let stored_artifact = ctx
        .artifacts
        .get_artifact(&messages[0].artifacts[0].id)
        .await?
        .ok_or_else(|| "stored artifact missing".to_string())?;
    assert_eq!(stored_artifact.conversation_id.unwrap(), conversation.id);
    assert_eq!(stored_artifact.message_id.unwrap().to_string(), message_id);

    Ok(())
}

#[tokio::test]
async fn mobile_delete_artifact_removes_unattached_upload() -> common::TestResult {
    let ctx = common::test_mobile_router_with_jwt(JWT_SECRET, JWT_ISSUER);
    let upload_req = multipart_upload_request(
        "/mobile/v1/uploads",
        bearer("user-a", &["chat:write"]),
        "notes.txt",
        "text/plain",
        b"artifact-body",
        None,
    )?;
    let upload_resp = ctx.app.clone().oneshot(upload_req).await?;
    let upload_json = common::json_body(upload_resp).await?;
    let artifact_id = upload_json["artifact"]["id"]
        .as_str()
        .ok_or_else(|| "artifact id missing".to_string())?;

    let delete_req = common::request(
        Request::builder()
            .method("DELETE")
            .uri(format!("/mobile/v1/artifacts/{artifact_id}"))
            .header("Authorization", bearer("user-a", &["chat:write"])),
        Body::empty(),
    )?;
    let delete_resp = ctx.app.clone().oneshot(delete_req).await?;
    assert_eq!(delete_resp.status(), StatusCode::NO_CONTENT);

    let metadata_req = common::request(
        Request::builder()
            .uri(format!("/mobile/v1/artifacts/{artifact_id}"))
            .header("Authorization", bearer("user-a", &["chat:read"])),
        Body::empty(),
    )?;
    let metadata_resp = ctx.app.clone().oneshot(metadata_req).await?;
    assert_eq!(metadata_resp.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[tokio::test]
async fn mobile_routes_hide_other_users_conversations() -> common::TestResult {
    let ctx = common::test_mobile_router_with_jwt(JWT_SECRET, JWT_ISSUER);
    let conversation = seed_conversation(
        ctx.conversations.as_ref(),
        "owner",
        "private",
        chrono::Utc::now(),
    )
    .await?;

    let req = common::request(
        Request::builder()
            .uri(format!("/mobile/v1/conversations/{}", conversation.id))
            .header("Authorization", bearer("intruder", &["chat:read"])),
        Body::empty(),
    )?;
    let resp = ctx.app.oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test]
async fn mobile_stream_emits_delta_completed_and_done_frames() -> common::TestResult {
    let ctx = common::test_mobile_router_with_jwt(JWT_SECRET, JWT_ISSUER);
    let conversation = seed_conversation(
        ctx.conversations.as_ref(),
        "user-a",
        "stream",
        chrono::Utc::now(),
    )
    .await?;

    let req = common::request(
        Request::builder()
            .uri(format!(
                "/mobile/v1/conversations/{}/stream",
                conversation.id
            ))
            .header("Authorization", bearer("user-a", &["chat:read"])),
        Body::empty(),
    )?;
    let resp = ctx.app.clone().oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("text/event-stream")
    );

    let mut stream = resp.into_body().into_data_stream();
    let mut buffer = String::new();

    ctx.stream_registry.send(StreamChunk::new(
        conversation.session_id,
        "mobile",
        None,
        StreamChunkKind::Delta("partial".into()),
    ));
    let (event, data) = parse_sse_frame(&next_sse_frame(&mut stream, &mut buffer).await?)?;
    assert_eq!(event, "message_delta");
    assert_eq!(data["delta"], "partial");

    let mut artifact = orka_core::ConversationArtifact::new(
        "user-a",
        orka_core::ConversationArtifactOrigin::AssistantOutput,
        "text/plain",
        "artifact.txt",
    );
    ctx.artifacts.put_artifact(&artifact, b"artifact-body").await?;
    artifact.conversation_id = Some(conversation.id);
    ctx.mobile_events
        .publish(
            conversation.id,
            MobileStreamEvent::ArtifactReady {
                conversation_id: conversation.id,
                artifact: artifact.clone(),
            },
        )
        .await;
    let (event, data) = parse_sse_frame(&next_sse_frame(&mut stream, &mut buffer).await?)?;
    assert_eq!(event, "artifact_ready");
    assert_eq!(data["artifact"]["filename"], "artifact.txt");

    let message = ConversationMessage::new(
        MessageId::new(),
        conversation.id,
        conversation.session_id,
        ConversationMessageRole::Assistant,
        "final answer",
    );
    ctx.mobile_events
        .publish(
            conversation.id,
            MobileStreamEvent::MessageCompleted {
                message: message.clone(),
            },
        )
        .await;
    let (event, data) = parse_sse_frame(&next_sse_frame(&mut stream, &mut buffer).await?)?;
    assert_eq!(event, "message_completed");
    assert_eq!(data["message"]["id"], message.id.to_string());
    assert_eq!(data["message"]["text"], "final answer");

    ctx.stream_registry.send(StreamChunk::new(
        conversation.session_id,
        "mobile",
        None,
        StreamChunkKind::Done,
    ));
    let (event, data) = parse_sse_frame(&next_sse_frame(&mut stream, &mut buffer).await?)?;
    assert_eq!(event, "stream_done");
    assert_eq!(data["conversation_id"], conversation.id.to_string());
    Ok(())
}

async fn assert_refresh_token_rotation(
    app: axum::Router,
    refresh_token: &str,
) -> common::TestResult {
    let refresh_req = common::request(
        Request::builder()
            .method("POST")
            .uri("/mobile/v1/auth/refresh")
            .header("content-type", "application/json"),
        Body::from(format!(
            r#"{{"refresh_token":"{refresh_token}","device_id":"device-123"}}"#
        )),
    )?;
    let refresh_resp = app.clone().oneshot(refresh_req).await?;
    assert_eq!(refresh_resp.status(), StatusCode::OK);
    let refresh_json = common::json_body(refresh_resp).await?;
    assert_eq!(refresh_json["user_id"], "operator-1");
    assert_ne!(refresh_json["refresh_token"], refresh_token);

    let reused_refresh_req = common::request(
        Request::builder()
            .method("POST")
            .uri("/mobile/v1/auth/refresh")
            .header("content-type", "application/json"),
        Body::from(format!(
            r#"{{"refresh_token":"{refresh_token}","device_id":"device-123"}}"#
        )),
    )?;
    let reused_refresh_resp = app.oneshot(reused_refresh_req).await?;
    assert_eq!(reused_refresh_resp.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}

#[tokio::test]
async fn mobile_pairing_create_complete_and_refresh_issue_valid_mobile_session()
-> common::TestResult {
    let ctx = common::test_mobile_router_with_jwt(JWT_SECRET, JWT_ISSUER);

    let create_req = common::request(
        Request::builder()
            .method("POST")
            .uri("/mobile/v1/pairings")
            .header("Authorization", bearer("operator-1", &["mobile:pair"]))
            .header("content-type", "application/json"),
        Body::from(r#"{"server_base_url":"https://orka.example.com"}"#),
    )?;
    let create_resp = ctx.app.clone().oneshot(create_req).await?;
    assert_eq!(create_resp.status(), StatusCode::CREATED);
    let create_json = common::json_body(create_resp).await?;
    let pairing_id = create_json["pairing_id"]
        .as_str()
        .ok_or_else(|| "pairing_id missing".to_string())?;
    let pairing_secret = create_json["pairing_secret"]
        .as_str()
        .ok_or_else(|| "pairing_secret missing".to_string())?;
    assert!(
        create_json["pairing_uri"]
            .as_str()
            .is_some_and(|value| value.starts_with("mobileorka://pair?"))
    );

    let status_req = common::request(
        Request::builder()
            .uri(format!("/mobile/v1/pairings/{pairing_id}"))
            .header("Authorization", bearer("operator-1", &["mobile:pair"])),
        Body::empty(),
    )?;
    let status_resp = ctx.app.clone().oneshot(status_req).await?;
    assert_eq!(status_resp.status(), StatusCode::OK);
    let status_json = common::json_body(status_resp).await?;
    assert_eq!(status_json["status"], "pending");

    let complete_req = common::request(
        Request::builder()
            .method("POST")
            .uri("/mobile/v1/pairings/complete")
            .header("content-type", "application/json"),
        Body::from(format!(
            r#"{{
                "pairing_id":"{pairing_id}",
                "pairing_secret":"{pairing_secret}",
                "device_id":"device-123",
                "device_name":"Pixel 9",
                "platform":"android"
            }}"#
        )),
    )?;
    let complete_resp = ctx.app.clone().oneshot(complete_req).await?;
    assert_eq!(complete_resp.status(), StatusCode::OK);
    let complete_json = common::json_body(complete_resp).await?;
    let access_token = complete_json["access_token"]
        .as_str()
        .ok_or_else(|| "access_token missing".to_string())?;
    let refresh_token = complete_json["refresh_token"]
        .as_str()
        .ok_or_else(|| "refresh_token missing".to_string())?;
    assert_eq!(complete_json["user_id"], "operator-1");

    let me_req = common::request(
        Request::builder()
            .uri("/mobile/v1/me")
            .header("Authorization", format!("Bearer {access_token}")),
        Body::empty(),
    )?;
    let me_resp = ctx.app.clone().oneshot(me_req).await?;
    assert_eq!(me_resp.status(), StatusCode::OK);
    let me_json = common::json_body(me_resp).await?;
    assert_eq!(me_json["user_id"], "operator-1");

    let completed_status_req = common::request(
        Request::builder()
            .uri(format!("/mobile/v1/pairings/{pairing_id}"))
            .header("Authorization", bearer("operator-1", &["mobile:pair"])),
        Body::empty(),
    )?;
    let completed_status_resp = ctx.app.clone().oneshot(completed_status_req).await?;
    assert_eq!(completed_status_resp.status(), StatusCode::OK);
    let completed_status_json = common::json_body(completed_status_resp).await?;
    assert_eq!(completed_status_json["status"], "completed");
    assert_eq!(completed_status_json["device_label"], "Pixel 9 (android)");

    assert_refresh_token_rotation(ctx.app, refresh_token).await
}

#[tokio::test]
async fn mobile_pairing_cannot_be_completed_twice() -> common::TestResult {
    let ctx = common::test_mobile_router_with_jwt(JWT_SECRET, JWT_ISSUER);

    let create_req = common::request(
        Request::builder()
            .method("POST")
            .uri("/mobile/v1/pairings")
            .header("Authorization", bearer("operator-1", &["mobile:pair"]))
            .header("content-type", "application/json"),
        Body::from(r#"{"server_base_url":"https://orka.example.com"}"#),
    )?;
    let create_resp = ctx.app.clone().oneshot(create_req).await?;
    let create_json = common::json_body(create_resp).await?;
    let pairing_id = create_json["pairing_id"]
        .as_str()
        .ok_or_else(|| "pairing_id missing".to_string())?;
    let pairing_secret = create_json["pairing_secret"]
        .as_str()
        .ok_or_else(|| "pairing_secret missing".to_string())?;

    for expected in [StatusCode::OK, StatusCode::GONE] {
        let complete_req = common::request(
            Request::builder()
                .method("POST")
                .uri("/mobile/v1/pairings/complete")
                .header("content-type", "application/json"),
            Body::from(format!(
                r#"{{
                    "pairing_id":"{pairing_id}",
                    "pairing_secret":"{pairing_secret}",
                    "device_id":"device-123",
                    "device_name":"Pixel 9",
                    "platform":"android"
                }}"#
            )),
        )?;
        let resp = ctx.app.clone().oneshot(complete_req).await?;
        assert_eq!(resp.status(), expected);
    }

    Ok(())
}

#[tokio::test]
async fn openapi_spec_includes_mobile_paths() -> common::TestResult {
    let app = common::test_router();
    let req = common::request(
        Request::builder().uri("/api-doc/openapi.json"),
        Body::empty(),
    )?;
    let resp = app.oneshot(req).await?;
    assert_eq!(resp.status(), StatusCode::OK);

    let json = common::json_body(resp).await?;
    assert!(json["paths"]["/mobile/v1/me"].is_object());
    assert!(json["paths"]["/mobile/v1/conversations"].is_object());
    assert!(json["paths"]["/mobile/v1/conversations/{id}/messages"].is_object());
    assert!(json["paths"]["/mobile/v1/uploads"].is_object());
    assert!(json["paths"]["/mobile/v1/artifacts/{id}"].is_object());
    assert!(json["paths"]["/mobile/v1/artifacts/{id}/content"].is_object());
    Ok(())
}

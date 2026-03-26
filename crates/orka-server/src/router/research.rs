//! Research campaign management endpoints.

use std::{collections::HashMap, sync::Arc};

use axum::{
    Json,
    extract::{Path, Query},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use orka_core::{Error, SessionId, StreamRegistry};
use uuid::Uuid;
use orka_research::{CreateResearchCampaign, PromotionSubmission, ResearchService};

#[derive(serde::Deserialize)]
struct PromoteCandidateRequest {
    #[serde(default)]
    approved: bool,
}

#[derive(serde::Deserialize)]
struct RejectPromotionRequest {
    reason: Option<String>,
}

fn require_research(svc: Option<Arc<ResearchService>>) -> Result<Arc<ResearchService>, Response> {
    svc.ok_or_else(|| {
        (StatusCode::SERVICE_UNAVAILABLE, "research not enabled").into_response()
    })
}

fn research_error(e: Error) -> Response {
    match e {
        Error::ResearchNotFound(msg) => (StatusCode::NOT_FOUND, msg).into_response(),
        Error::ResearchConflict(msg) => (StatusCode::CONFLICT, msg).into_response(),
        e => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub(super) fn routes(
    research_service: Option<Arc<ResearchService>>,
    stream_registry: StreamRegistry,
) -> axum::Router {
    axum::Router::new()
        .route(
            "/api/v1/research/campaigns",
            axum::routing::get({
                let svc = research_service.clone();
                move || {
                    let svc = svc.clone();
                    async move {
                        let service = require_research(svc)?;
                        match service.list_campaigns().await {
                            Ok(campaigns) => Ok(Json(campaigns).into_response()),
                            Err(e) => Err(research_error(e)),
                        }
                    }
                }
            })
            .post({
                let svc = research_service.clone();
                move |Json(input): Json<CreateResearchCampaign>| {
                    let svc = svc.clone();
                    async move {
                        let service = require_research(svc)?;
                        match service.create_campaign(input).await {
                            Ok(campaign) => Ok((StatusCode::CREATED, Json(campaign)).into_response()),
                            Err(e) => Err(research_error(e)),
                        }
                    }
                }
            }),
        )
        .route(
            "/api/v1/research/campaigns/{id}",
            axum::routing::get({
                let svc = research_service.clone();
                move |Path(id): Path<String>| {
                    let svc = svc.clone();
                    async move {
                        let service = require_research(svc)?;
                        match service.get_campaign(&id).await {
                            Ok(Some(campaign)) => Ok(Json(campaign).into_response()),
                            Ok(None) => Err((StatusCode::NOT_FOUND, "campaign not found").into_response()),
                            Err(e) => Err(research_error(e)),
                        }
                    }
                }
            })
            .delete({
                let svc = research_service.clone();
                move |Path(id): Path<String>| {
                    let svc = svc.clone();
                    async move {
                        let service = require_research(svc)?;
                        match service.delete_campaign(&id).await {
                            Ok(true) => Ok(Json(serde_json::json!({ "deleted": true })).into_response()),
                            Ok(false) => Err((StatusCode::NOT_FOUND, "campaign not found").into_response()),
                            Err(e) => Err(research_error(e)),
                        }
                    }
                }
            }),
        )
        .route(
            "/api/v1/research/campaigns/{id}/pause",
            axum::routing::post({
                let svc = research_service.clone();
                move |Path(id): Path<String>| {
                    let svc = svc.clone();
                    async move {
                        let service = require_research(svc)?;
                        match service.pause_campaign(&id).await {
                            Ok(Some(campaign)) => Ok(Json(campaign).into_response()),
                            Ok(None) => Err((StatusCode::NOT_FOUND, "campaign not found").into_response()),
                            Err(e) => Err(research_error(e)),
                        }
                    }
                }
            }),
        )
        .route(
            "/api/v1/research/campaigns/{id}/resume",
            axum::routing::post({
                let svc = research_service.clone();
                move |Path(id): Path<String>| {
                    let svc = svc.clone();
                    async move {
                        let service = require_research(svc)?;
                        match service.resume_campaign(&id).await {
                            Ok(Some(campaign)) => Ok(Json(campaign).into_response()),
                            Ok(None) => Err((StatusCode::NOT_FOUND, "campaign not found").into_response()),
                            Err(e) => Err(research_error(e)),
                        }
                    }
                }
            }),
        )
        .route(
            "/api/v1/research/campaigns/{id}/runs",
            axum::routing::post({
                let svc = research_service.clone();
                move |Path(id): Path<String>| {
                    let svc = svc.clone();
                    async move {
                        let service = require_research(svc)?;
                        match service.run_campaign_async(&id).await {
                            Ok(run) => Ok((StatusCode::ACCEPTED, Json(run)).into_response()),
                            Err(e) => Err(research_error(e)),
                        }
                    }
                }
            }),
        )
        .route(
            "/api/v1/research/runs",
            axum::routing::get({
                let svc = research_service.clone();
                move |Query(params): Query<HashMap<String, String>>| {
                    let svc = svc.clone();
                    async move {
                        let service = require_research(svc)?;
                        let campaign_id = params.get("campaign_id").map(String::as_str);
                        match service.list_runs(campaign_id).await {
                            Ok(runs) => Ok(Json(runs).into_response()),
                            Err(e) => Err(research_error(e)),
                        }
                    }
                }
            }),
        )
        .route(
            "/api/v1/research/runs/{id}",
            axum::routing::get({
                let svc = research_service.clone();
                move |Path(id): Path<String>| {
                    let svc = svc.clone();
                    async move {
                        let service = require_research(svc)?;
                        match service.get_run(&id).await {
                            Ok(Some(run)) => Ok(Json(run).into_response()),
                            Ok(None) => Err((StatusCode::NOT_FOUND, "run not found").into_response()),
                            Err(e) => Err(research_error(e)),
                        }
                    }
                }
            }),
        )
        .route(
            "/api/v1/research/runs/{id}/stream",
            axum::routing::get({
                let svc = research_service.clone();
                let reg = stream_registry.clone();
                move |Path(id): Path<String>| {
                    let svc = svc.clone();
                    let reg = reg.clone();
                    async move {
                        use axum::response::sse::{Event, Sse};
                        use tokio_stream::wrappers::UnboundedReceiverStream;
                        use tokio_stream::StreamExt;

                        let service = require_research(svc)?;
                        let run = match service.get_run(&id).await {
                            Ok(Some(r)) => r,
                            Ok(None) => {
                                return Err((StatusCode::NOT_FOUND, "run not found").into_response())
                            }
                            Err(e) => return Err(research_error(e)),
                        };

                        let session_id: SessionId = run
                            .metadata
                            .get("stream_session_id")
                            .and_then(|v| v.as_str())
                            .and_then(|s| Uuid::parse_str(s).ok().map(SessionId))
                            .unwrap_or_else(SessionId::new);

                        let rx = reg.subscribe(session_id);
                        let stream = UnboundedReceiverStream::new(rx).filter_map(|chunk| {
                            let data = serde_json::to_string(&chunk.kind).ok()?;
                            Some(Ok::<Event, std::convert::Infallible>(
                                Event::default().data(data),
                            ))
                        });

                        Ok(Sse::new(stream).into_response())
                    }
                }
            }),
        )
        .route(
            "/api/v1/research/candidates",
            axum::routing::get({
                let svc = research_service.clone();
                move |Query(params): Query<HashMap<String, String>>| {
                    let svc = svc.clone();
                    async move {
                        let service = require_research(svc)?;
                        let campaign_id = params.get("campaign_id").map(String::as_str);
                        match service.list_candidates(campaign_id).await {
                            Ok(candidates) => Ok(Json(candidates).into_response()),
                            Err(e) => Err(research_error(e)),
                        }
                    }
                }
            }),
        )
        .route(
            "/api/v1/research/candidates/{id}",
            axum::routing::get({
                let svc = research_service.clone();
                move |Path(id): Path<String>| {
                    let svc = svc.clone();
                    async move {
                        let service = require_research(svc)?;
                        match service.get_candidate(&id).await {
                            Ok(Some(candidate)) => Ok(Json(candidate).into_response()),
                            Ok(None) => Err((StatusCode::NOT_FOUND, "candidate not found").into_response()),
                            Err(e) => Err(research_error(e)),
                        }
                    }
                }
            }),
        )
        .route(
            "/api/v1/research/candidates/{id}/promote",
            axum::routing::post({
                let svc = research_service.clone();
                move |Path(id): Path<String>, Json(body): Json<PromoteCandidateRequest>| {
                    let svc = svc.clone();
                    async move {
                        let service = require_research(svc)?;
                        match service.submit_promotion(&id, body.approved).await {
                            Ok(PromotionSubmission::Promoted { candidate }) => {
                                Ok(Json(candidate).into_response())
                            }
                            Ok(PromotionSubmission::ApprovalRequired { request }) => {
                                Ok((StatusCode::ACCEPTED, Json(request)).into_response())
                            }
                            Err(e) => Err(research_error(e)),
                        }
                    }
                }
            }),
        )
        .route(
            "/api/v1/research/promotions",
            axum::routing::get({
                let svc = research_service.clone();
                move |Query(params): Query<HashMap<String, String>>| {
                    let svc = svc.clone();
                    async move {
                        let service = require_research(svc)?;
                        let campaign_id = params.get("campaign_id").map(String::as_str);
                        match service.list_promotion_requests(campaign_id).await {
                            Ok(requests) => Ok(Json(requests).into_response()),
                            Err(e) => Err(research_error(e)),
                        }
                    }
                }
            }),
        )
        .route(
            "/api/v1/research/promotions/{id}",
            axum::routing::get({
                let svc = research_service.clone();
                move |Path(id): Path<String>| {
                    let svc = svc.clone();
                    async move {
                        let service = require_research(svc)?;
                        match service.get_promotion_request(&id).await {
                            Ok(Some(request)) => Ok(Json(request).into_response()),
                            Ok(None) => Err((StatusCode::NOT_FOUND, "promotion request not found").into_response()),
                            Err(e) => Err(research_error(e)),
                        }
                    }
                }
            }),
        )
        .route(
            "/api/v1/research/promotions/{id}/approve",
            axum::routing::post({
                let svc = research_service.clone();
                move |Path(id): Path<String>| {
                    let svc = svc.clone();
                    async move {
                        let service = require_research(svc)?;
                        match service.approve_promotion_request(&id).await {
                            Ok(candidate) => Ok(Json(candidate).into_response()),
                            Err(e) => Err(research_error(e)),
                        }
                    }
                }
            }),
        )
        .route(
            "/api/v1/research/promotions/{id}/reject",
            axum::routing::post({
                move |Path(id): Path<String>, Json(body): Json<RejectPromotionRequest>| {
                    let svc = research_service.clone();
                    async move {
                        let service = require_research(svc)?;
                        match service.reject_promotion_request(&id, body.reason).await {
                            Ok(request) => Ok(Json(request).into_response()),
                            Err(e) => Err(research_error(e)),
                        }
                    }
                }
            }),
        )
}

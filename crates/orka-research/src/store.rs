use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use orka_core::{Error, Result};
use redis::AsyncCommands;
use tokio::sync::RwLock;

use crate::types::{ResearchCampaign, ResearchCandidate, ResearchPromotionRequest, ResearchRun};

/// Persistence interface for research campaigns and their artifacts.
#[async_trait]
pub trait ResearchStore: Send + Sync + 'static {
    /// Insert or update a campaign.
    async fn put_campaign(&self, campaign: &ResearchCampaign) -> Result<()>;
    /// Load a single campaign by ID.
    async fn get_campaign(&self, id: &str) -> Result<Option<ResearchCampaign>>;
    /// List all campaigns.
    async fn list_campaigns(&self) -> Result<Vec<ResearchCampaign>>;
    /// Delete a campaign by ID.
    async fn delete_campaign(&self, id: &str) -> Result<bool>;

    /// Insert or update a run.
    async fn put_run(&self, run: &ResearchRun) -> Result<()>;
    /// Load a run by ID.
    async fn get_run(&self, id: &str) -> Result<Option<ResearchRun>>;
    /// List runs, optionally filtered by campaign.
    async fn list_runs(&self, campaign_id: Option<&str>) -> Result<Vec<ResearchRun>>;
    /// Delete a run by ID.
    async fn delete_run(&self, id: &str) -> Result<bool>;

    /// Insert or update a candidate.
    async fn put_candidate(&self, candidate: &ResearchCandidate) -> Result<()>;
    /// Load a candidate by ID.
    async fn get_candidate(&self, id: &str) -> Result<Option<ResearchCandidate>>;
    /// List candidates, optionally filtered by campaign.
    async fn list_candidates(&self, campaign_id: Option<&str>) -> Result<Vec<ResearchCandidate>>;
    /// Delete a candidate by ID.
    async fn delete_candidate(&self, id: &str) -> Result<bool>;

    /// Insert or update a promotion request.
    async fn put_promotion_request(&self, request: &ResearchPromotionRequest) -> Result<()>;
    /// Load a promotion request by ID.
    async fn get_promotion_request(&self, id: &str) -> Result<Option<ResearchPromotionRequest>>;
    /// List promotion requests, optionally filtered by campaign.
    async fn list_promotion_requests(
        &self,
        campaign_id: Option<&str>,
    ) -> Result<Vec<ResearchPromotionRequest>>;
    /// Delete a promotion request by ID.
    async fn delete_promotion_request(&self, id: &str) -> Result<bool>;
}

/// In-memory store useful for tests and embedded usage.
pub struct InMemoryResearchStore {
    campaigns: RwLock<HashMap<String, ResearchCampaign>>,
    runs: RwLock<HashMap<String, ResearchRun>>,
    candidates: RwLock<HashMap<String, ResearchCandidate>>,
    promotion_requests: RwLock<HashMap<String, ResearchPromotionRequest>>,
}

impl InMemoryResearchStore {
    /// Create an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            campaigns: RwLock::new(HashMap::new()),
            runs: RwLock::new(HashMap::new()),
            candidates: RwLock::new(HashMap::new()),
            promotion_requests: RwLock::new(HashMap::new()),
        }
    }
}

impl Default for InMemoryResearchStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ResearchStore for InMemoryResearchStore {
    async fn put_campaign(&self, campaign: &ResearchCampaign) -> Result<()> {
        self.campaigns
            .write()
            .await
            .insert(campaign.id.clone(), campaign.clone());
        Ok(())
    }

    async fn get_campaign(&self, id: &str) -> Result<Option<ResearchCampaign>> {
        Ok(self.campaigns.read().await.get(id).cloned())
    }

    async fn list_campaigns(&self) -> Result<Vec<ResearchCampaign>> {
        let mut campaigns: Vec<_> = self.campaigns.read().await.values().cloned().collect();
        campaigns.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(campaigns)
    }

    async fn delete_campaign(&self, id: &str) -> Result<bool> {
        Ok(self.campaigns.write().await.remove(id).is_some())
    }

    async fn put_run(&self, run: &ResearchRun) -> Result<()> {
        self.runs.write().await.insert(run.id.clone(), run.clone());
        Ok(())
    }

    async fn get_run(&self, id: &str) -> Result<Option<ResearchRun>> {
        Ok(self.runs.read().await.get(id).cloned())
    }

    async fn list_runs(&self, campaign_id: Option<&str>) -> Result<Vec<ResearchRun>> {
        let mut runs: Vec<_> = self.runs.read().await.values().cloned().collect();
        if let Some(campaign_id) = campaign_id {
            runs.retain(|run| run.campaign_id == campaign_id);
        }
        runs.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(runs)
    }

    async fn delete_run(&self, id: &str) -> Result<bool> {
        Ok(self.runs.write().await.remove(id).is_some())
    }

    async fn put_candidate(&self, candidate: &ResearchCandidate) -> Result<()> {
        self.candidates
            .write()
            .await
            .insert(candidate.id.clone(), candidate.clone());
        Ok(())
    }

    async fn get_candidate(&self, id: &str) -> Result<Option<ResearchCandidate>> {
        Ok(self.candidates.read().await.get(id).cloned())
    }

    async fn list_candidates(&self, campaign_id: Option<&str>) -> Result<Vec<ResearchCandidate>> {
        let mut candidates: Vec<_> = self.candidates.read().await.values().cloned().collect();
        if let Some(campaign_id) = campaign_id {
            candidates.retain(|candidate| candidate.campaign_id == campaign_id);
        }
        candidates.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(candidates)
    }

    async fn delete_candidate(&self, id: &str) -> Result<bool> {
        Ok(self.candidates.write().await.remove(id).is_some())
    }

    async fn put_promotion_request(&self, request: &ResearchPromotionRequest) -> Result<()> {
        self.promotion_requests
            .write()
            .await
            .insert(request.id.clone(), request.clone());
        Ok(())
    }

    async fn get_promotion_request(&self, id: &str) -> Result<Option<ResearchPromotionRequest>> {
        Ok(self.promotion_requests.read().await.get(id).cloned())
    }

    async fn list_promotion_requests(
        &self,
        campaign_id: Option<&str>,
    ) -> Result<Vec<ResearchPromotionRequest>> {
        let mut requests: Vec<_> = self
            .promotion_requests
            .read()
            .await
            .values()
            .cloned()
            .collect();
        if let Some(campaign_id) = campaign_id {
            requests.retain(|request| request.campaign_id == campaign_id);
        }
        requests.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(requests)
    }

    async fn delete_promotion_request(&self, id: &str) -> Result<bool> {
        Ok(self.promotion_requests.write().await.remove(id).is_some())
    }
}

const CAMPAIGN_IDS_KEY: &str = "orka:research:campaigns";
const RUN_IDS_KEY: &str = "orka:research:runs";
const CANDIDATE_IDS_KEY: &str = "orka:research:candidates";
const PROMOTION_REQUEST_IDS_KEY: &str = "orka:research:promotion_requests";
const CAMPAIGN_KEY_PREFIX: &str = "orka:research:campaign:";
const RUN_KEY_PREFIX: &str = "orka:research:run:";
const CANDIDATE_KEY_PREFIX: &str = "orka:research:candidate:";
const PROMOTION_REQUEST_KEY_PREFIX: &str = "orka:research:promotion_request:";

/// Redis-backed research store.
pub struct RedisResearchStore {
    pool: Arc<deadpool_redis::Pool>,
}

impl RedisResearchStore {
    /// Create a new Redis store.
    pub fn new(redis_url: &str) -> Result<Self> {
        let cfg = deadpool_redis::Config::from_url(redis_url);
        let pool = cfg
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .map_err(|e| Error::Research(format!("failed to create Redis pool: {e}")))?;
        Ok(Self {
            pool: Arc::new(pool),
        })
    }

    async fn with_conn<T>(
        &self,
        f: impl FnOnce(
            &mut deadpool_redis::Connection,
        ) -> futures_util::future::BoxFuture<'_, Result<T>>,
    ) -> Result<T> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| Error::Research(format!("Redis connection failed: {e}")))?;
        f(&mut conn).await
    }
}

fn campaign_key(id: &str) -> String {
    format!("{CAMPAIGN_KEY_PREFIX}{id}")
}

fn run_key(id: &str) -> String {
    format!("{RUN_KEY_PREFIX}{id}")
}

fn candidate_key(id: &str) -> String {
    format!("{CANDIDATE_KEY_PREFIX}{id}")
}

fn promotion_request_key(id: &str) -> String {
    format!("{PROMOTION_REQUEST_KEY_PREFIX}{id}")
}

fn campaign_runs_key(campaign_id: &str) -> String {
    format!("{CAMPAIGN_KEY_PREFIX}{campaign_id}:runs")
}

fn campaign_candidates_key(campaign_id: &str) -> String {
    format!("{CAMPAIGN_KEY_PREFIX}{campaign_id}:candidates")
}

fn campaign_promotions_key(campaign_id: &str) -> String {
    format!("{CAMPAIGN_KEY_PREFIX}{campaign_id}:promotions")
}

#[async_trait]
impl ResearchStore for RedisResearchStore {
    async fn put_campaign(&self, campaign: &ResearchCampaign) -> Result<()> {
        let payload = serde_json::to_string(campaign)?;
        let id = campaign.id.clone();
        self.with_conn(move |conn| {
            Box::pin(async move {
                redis::pipe()
                    .atomic()
                    .set(campaign_key(&id), payload)
                    .ignore()
                    .sadd(CAMPAIGN_IDS_KEY, &id)
                    .ignore()
                    .query_async::<()>(conn)
                    .await
                    .map_err(|e| Error::Research(format!("failed to store campaign: {e}")))?;
                Ok(())
            })
        })
        .await
    }

    async fn get_campaign(&self, id: &str) -> Result<Option<ResearchCampaign>> {
        let id = id.to_string();
        self.with_conn(move |conn| {
            Box::pin(async move {
                let data: Option<String> = conn
                    .get(campaign_key(&id))
                    .await
                    .map_err(|e| Error::Research(format!("failed to load campaign: {e}")))?;
                data.map(|json| serde_json::from_str(&json))
                    .transpose()
                    .map_err(Error::from)
            })
        })
        .await
    }

    async fn list_campaigns(&self) -> Result<Vec<ResearchCampaign>> {
        self.with_conn(|conn| {
            Box::pin(async move {
                let ids: Vec<String> = conn
                    .smembers(CAMPAIGN_IDS_KEY)
                    .await
                    .map_err(|e| Error::Research(format!("failed to list campaign ids: {e}")))?;
                let mut items = Vec::with_capacity(ids.len());
                for id in ids {
                    if let Some(data) = conn
                        .get::<_, Option<String>>(campaign_key(&id))
                        .await
                        .map_err(|e| Error::Research(format!("failed to load campaign: {e}")))?
                    {
                        items.push(serde_json::from_str::<ResearchCampaign>(&data)?);
                    }
                }
                items.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
                Ok(items)
            })
        })
        .await
    }

    async fn delete_campaign(&self, id: &str) -> Result<bool> {
        let id = id.to_string();
        self.with_conn(move |conn| {
            Box::pin(async move {
                let (removed,): (i64,) = redis::pipe()
                    .atomic()
                    .del(campaign_key(&id))
                    .srem(CAMPAIGN_IDS_KEY, &id)
                    .ignore()
                    .query_async(conn)
                    .await
                    .map_err(|e| Error::Research(format!("failed to delete campaign: {e}")))?;
                Ok(removed > 0)
            })
        })
        .await
    }

    async fn put_run(&self, run: &ResearchRun) -> Result<()> {
        let payload = serde_json::to_string(run)?;
        let id = run.id.clone();
        let campaign_id = run.campaign_id.clone();
        self.with_conn(move |conn| {
            Box::pin(async move {
                redis::pipe()
                    .atomic()
                    .set(run_key(&id), payload)
                    .ignore()
                    .sadd(RUN_IDS_KEY, &id)
                    .ignore()
                    .sadd(campaign_runs_key(&campaign_id), &id)
                    .ignore()
                    .query_async::<()>(conn)
                    .await
                    .map_err(|e| Error::Research(format!("failed to store run: {e}")))?;
                Ok(())
            })
        })
        .await
    }

    async fn get_run(&self, id: &str) -> Result<Option<ResearchRun>> {
        let id = id.to_string();
        self.with_conn(move |conn| {
            Box::pin(async move {
                let data: Option<String> = conn
                    .get(run_key(&id))
                    .await
                    .map_err(|e| Error::Research(format!("failed to load run: {e}")))?;
                data.map(|json| serde_json::from_str(&json))
                    .transpose()
                    .map_err(Error::from)
            })
        })
        .await
    }

    async fn list_runs(&self, campaign_id: Option<&str>) -> Result<Vec<ResearchRun>> {
        let campaign_id = campaign_id.map(str::to_string);
        self.with_conn(move |conn| {
            Box::pin(async move {
                let ids: Vec<String> = if let Some(ref cid) = campaign_id {
                    conn.smembers(campaign_runs_key(cid))
                        .await
                        .map_err(|e| Error::Research(format!("failed to list run ids: {e}")))?
                } else {
                    conn.smembers(RUN_IDS_KEY)
                        .await
                        .map_err(|e| Error::Research(format!("failed to list run ids: {e}")))?
                };
                let mut items = Vec::with_capacity(ids.len());
                for id in ids {
                    if let Some(data) = conn
                        .get::<_, Option<String>>(run_key(&id))
                        .await
                        .map_err(|e| Error::Research(format!("failed to load run: {e}")))?
                    {
                        items.push(serde_json::from_str::<ResearchRun>(&data)?);
                    }
                }
                items.sort_by(|a, b| b.created_at.cmp(&a.created_at));
                Ok(items)
            })
        })
        .await
    }

    async fn delete_run(&self, id: &str) -> Result<bool> {
        // Load first to get campaign_id for secondary index cleanup.
        let campaign_id = self.get_run(id).await?.map(|r| r.campaign_id);
        let id = id.to_string();
        self.with_conn(move |conn| {
            Box::pin(async move {
                let mut pipe = redis::pipe();
                pipe.atomic()
                    .del(run_key(&id))
                    .srem(RUN_IDS_KEY, &id)
                    .ignore();
                if let Some(ref cid) = campaign_id {
                    pipe.srem(campaign_runs_key(cid), &id).ignore();
                }
                let (removed,): (i64,) = pipe
                    .query_async(conn)
                    .await
                    .map_err(|e| Error::Research(format!("failed to delete run: {e}")))?;
                Ok(removed > 0)
            })
        })
        .await
    }

    async fn put_candidate(&self, candidate: &ResearchCandidate) -> Result<()> {
        let payload = serde_json::to_string(candidate)?;
        let id = candidate.id.clone();
        let campaign_id = candidate.campaign_id.clone();
        self.with_conn(move |conn| {
            Box::pin(async move {
                redis::pipe()
                    .atomic()
                    .set(candidate_key(&id), payload)
                    .ignore()
                    .sadd(CANDIDATE_IDS_KEY, &id)
                    .ignore()
                    .sadd(campaign_candidates_key(&campaign_id), &id)
                    .ignore()
                    .query_async::<()>(conn)
                    .await
                    .map_err(|e| Error::Research(format!("failed to store candidate: {e}")))?;
                Ok(())
            })
        })
        .await
    }

    async fn get_candidate(&self, id: &str) -> Result<Option<ResearchCandidate>> {
        let id = id.to_string();
        self.with_conn(move |conn| {
            Box::pin(async move {
                let data: Option<String> = conn
                    .get(candidate_key(&id))
                    .await
                    .map_err(|e| Error::Research(format!("failed to load candidate: {e}")))?;
                data.map(|json| serde_json::from_str(&json))
                    .transpose()
                    .map_err(Error::from)
            })
        })
        .await
    }

    async fn list_candidates(&self, campaign_id: Option<&str>) -> Result<Vec<ResearchCandidate>> {
        let campaign_id = campaign_id.map(str::to_string);
        self.with_conn(move |conn| {
            Box::pin(async move {
                let ids: Vec<String> = if let Some(ref cid) = campaign_id {
                    conn.smembers(campaign_candidates_key(cid))
                        .await
                        .map_err(|e| {
                            Error::Research(format!("failed to list candidate ids: {e}"))
                        })?
                } else {
                    conn.smembers(CANDIDATE_IDS_KEY).await.map_err(|e| {
                        Error::Research(format!("failed to list candidate ids: {e}"))
                    })?
                };
                let mut items = Vec::with_capacity(ids.len());
                for id in ids {
                    if let Some(data) = conn
                        .get::<_, Option<String>>(candidate_key(&id))
                        .await
                        .map_err(|e| Error::Research(format!("failed to load candidate: {e}")))?
                    {
                        items.push(serde_json::from_str::<ResearchCandidate>(&data)?);
                    }
                }
                items.sort_by(|a, b| b.created_at.cmp(&a.created_at));
                Ok(items)
            })
        })
        .await
    }

    async fn delete_candidate(&self, id: &str) -> Result<bool> {
        let campaign_id = self.get_candidate(id).await?.map(|c| c.campaign_id);
        let id = id.to_string();
        self.with_conn(move |conn| {
            Box::pin(async move {
                let mut pipe = redis::pipe();
                pipe.atomic()
                    .del(candidate_key(&id))
                    .srem(CANDIDATE_IDS_KEY, &id)
                    .ignore();
                if let Some(ref cid) = campaign_id {
                    pipe.srem(campaign_candidates_key(cid), &id).ignore();
                }
                let (removed,): (i64,) = pipe
                    .query_async(conn)
                    .await
                    .map_err(|e| Error::Research(format!("failed to delete candidate: {e}")))?;
                Ok(removed > 0)
            })
        })
        .await
    }

    async fn put_promotion_request(&self, request: &ResearchPromotionRequest) -> Result<()> {
        let payload = serde_json::to_string(request)?;
        let id = request.id.clone();
        let campaign_id = request.campaign_id.clone();
        self.with_conn(move |conn| {
            Box::pin(async move {
                redis::pipe()
                    .atomic()
                    .set(promotion_request_key(&id), payload)
                    .ignore()
                    .sadd(PROMOTION_REQUEST_IDS_KEY, &id)
                    .ignore()
                    .sadd(campaign_promotions_key(&campaign_id), &id)
                    .ignore()
                    .query_async::<()>(conn)
                    .await
                    .map_err(|e| {
                        Error::Research(format!("failed to store promotion request: {e}"))
                    })?;
                Ok(())
            })
        })
        .await
    }

    async fn get_promotion_request(&self, id: &str) -> Result<Option<ResearchPromotionRequest>> {
        let id = id.to_string();
        self.with_conn(move |conn| {
            Box::pin(async move {
                let data: Option<String> =
                    conn.get(promotion_request_key(&id)).await.map_err(|e| {
                        Error::Research(format!("failed to load promotion request: {e}"))
                    })?;
                data.map(|json| serde_json::from_str(&json))
                    .transpose()
                    .map_err(Error::from)
            })
        })
        .await
    }

    async fn list_promotion_requests(
        &self,
        campaign_id: Option<&str>,
    ) -> Result<Vec<ResearchPromotionRequest>> {
        let campaign_id = campaign_id.map(str::to_string);
        self.with_conn(move |conn| {
            Box::pin(async move {
                let ids: Vec<String> = if let Some(ref cid) = campaign_id {
                    conn.smembers(campaign_promotions_key(cid))
                        .await
                        .map_err(|e| {
                            Error::Research(format!("failed to list promotion request ids: {e}"))
                        })?
                } else {
                    conn.smembers(PROMOTION_REQUEST_IDS_KEY)
                        .await
                        .map_err(|e| {
                            Error::Research(format!("failed to list promotion request ids: {e}"))
                        })?
                };
                let mut items = Vec::with_capacity(ids.len());
                for id in ids {
                    if let Some(data) = conn
                        .get::<_, Option<String>>(promotion_request_key(&id))
                        .await
                        .map_err(|e| {
                            Error::Research(format!("failed to load promotion request: {e}"))
                        })?
                    {
                        items.push(serde_json::from_str::<ResearchPromotionRequest>(&data)?);
                    }
                }
                items.sort_by(|a, b| b.created_at.cmp(&a.created_at));
                Ok(items)
            })
        })
        .await
    }

    async fn delete_promotion_request(&self, id: &str) -> Result<bool> {
        let campaign_id = self
            .get_promotion_request(id)
            .await?
            .map(|r| r.campaign_id);
        let id = id.to_string();
        self.with_conn(move |conn| {
            Box::pin(async move {
                let mut pipe = redis::pipe();
                pipe.atomic()
                    .del(promotion_request_key(&id))
                    .srem(PROMOTION_REQUEST_IDS_KEY, &id)
                    .ignore();
                if let Some(ref cid) = campaign_id {
                    pipe.srem(campaign_promotions_key(cid), &id).ignore();
                }
                let (removed,): (i64,) = pipe
                    .query_async(conn)
                    .await
                    .map_err(|e| {
                        Error::Research(format!("failed to delete promotion request: {e}"))
                    })?;
                Ok(removed > 0)
            })
        })
        .await
    }
}

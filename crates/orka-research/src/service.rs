use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
};

use chrono::Utc;
use orka_checkpoint::{Checkpoint, CheckpointId, CheckpointStore, InterruptReason, RunStatus};
use orka_core::{
    Envelope, Error, ErrorCategory, NoopEventSink, Result, SessionId, SkillContext, SkillInput,
    StreamRegistry,
    config::ResearchConfig,
    traits::{EventSink, SecretManager},
};
use orka_scheduler::{ScheduleStore, types::Schedule};
use orka_skills::SkillRegistry;
use regex::Regex;
use tracing::{info, warn};
use uuid::Uuid;

use crate::{
    store::ResearchStore,
    types::{
        CandidateStatus, ComparisonDirection, CreateResearchCampaign, EvaluationMetricConfig,
        EvaluationResult, PromotionRequestStatus, PromotionSubmission, ResearchCampaign,
        ResearchCandidate, ResearchPromotionRequest, ResearchRun, ResearchRunStatus,
    },
};

/// Service orchestrating research campaigns, runs, and candidate promotion.
pub struct ResearchService {
    store: Arc<dyn ResearchStore>,
    scheduler_store: Option<Arc<dyn ScheduleStore>>,
    checkpoint_store: Option<Arc<dyn CheckpointStore>>,
    config: ResearchConfig,
    secrets: Arc<dyn SecretManager>,
    event_sink: Option<Arc<dyn EventSink>>,
    skills: OnceLock<Arc<SkillRegistry>>,
    /// Optional stream registry for forwarding `coding_delegate` progress
    /// events to SSE subscribers keyed by run session ID.
    stream_registry: OnceLock<StreamRegistry>,
}

impl ResearchService {
    /// Create a new research service.
    #[must_use]
    pub fn new(
        store: Arc<dyn ResearchStore>,
        scheduler_store: Option<Arc<dyn ScheduleStore>>,
        checkpoint_store: Option<Arc<dyn CheckpointStore>>,
        config: ResearchConfig,
        secrets: Arc<dyn SecretManager>,
        event_sink: Option<Arc<dyn EventSink>>,
    ) -> Self {
        Self {
            store,
            scheduler_store,
            checkpoint_store,
            config,
            secrets,
            event_sink,
            skills: OnceLock::new(),
            stream_registry: OnceLock::new(),
        }
    }

    /// Bind the `StreamRegistry` once available (called after bootstrap).
    /// Subsequent calls are no-ops.
    pub fn bind_stream_registry(&self, registry: StreamRegistry) {
        let _ = self.stream_registry.set(registry);
    }

    /// Return the stream registry, if any.
    pub fn stream_registry(&self) -> Option<&StreamRegistry> {
        self.stream_registry.get()
    }

    /// Bind the global skill registry once the server bootstrap has finished
    /// constructing it.
    pub fn bind_registry(&self, skills: Arc<SkillRegistry>) {
        let _ = self.skills.set(skills);
    }

    pub(crate) fn registry(&self) -> Result<&Arc<SkillRegistry>> {
        self.skills.get().ok_or_else(|| Error::SkillCategorized {
            message: "research service is not bound to the skill registry".into(),
            category: ErrorCategory::Environmental,
        })
    }

    fn skill_context(&self, cwd: Option<String>, worktree_cwd: Option<String>) -> SkillContext {
        SkillContext::new(self.secrets.clone(), self.event_sink.clone())
            .with_user_cwd(cwd)
            .with_worktree_cwd(worktree_cwd)
    }

    async fn invoke_skill(
        &self,
        name: &str,
        args: HashMap<String, serde_json::Value>,
        cwd: Option<String>,
        worktree_cwd: Option<String>,
    ) -> Result<orka_core::SkillOutput> {
        self.registry()?
            .invoke(
                name,
                SkillInput::new(args).with_context(self.skill_context(cwd, worktree_cwd)),
            )
            .await
    }

    /// Invoke a skill with optional streaming support.
    ///
    /// When a `stream_session_id` is provided and a `StreamRegistry` is
    /// attached, progress events from `coding_delegate` are forwarded to the
    /// registry so SSE subscribers receive real-time updates.
    async fn invoke_skill_with_progress(
        &self,
        name: &str,
        args: HashMap<String, serde_json::Value>,
        cwd: Option<String>,
        worktree_cwd: Option<String>,
        stream_session_id: Option<SessionId>,
    ) -> Result<orka_core::SkillOutput> {
        let mut ctx = self.skill_context(cwd, worktree_cwd);

        if name == "coding_delegate"
            && let (Some(session_id), Some(registry)) =
                (stream_session_id, self.stream_registry.get().cloned())
        {
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<serde_json::Value>();
            ctx = ctx.with_progress(tx);
            // Use the service's event_sink if available, otherwise fall back to
            // a no-op sink so forward_delegate_progress can still emit domain events.
            let event_sink: std::sync::Arc<dyn orka_core::traits::EventSink> =
                self.event_sink.clone().map_or_else(
                    || std::sync::Arc::new(NoopEventSink) as std::sync::Arc<_>,
                    |sink| sink as std::sync::Arc<_>,
                );
            tokio::spawn(orka_core::stream::forward_delegate_progress(
                rx,
                registry,
                event_sink,
                session_id,
                "research".to_string(),
                None,
                orka_core::MessageId::new(),
            ));
        }

        self.registry()?
            .invoke(name, SkillInput::new(args).with_context(ctx))
            .await
    }

    /// Create a campaign and provision an optional schedule.
    pub async fn create_campaign(&self, input: CreateResearchCampaign) -> Result<ResearchCampaign> {
        validate_campaign_input(&input)?;
        let now = Utc::now();
        let id = Uuid::now_v7().to_string();
        let mut campaign = ResearchCampaign {
            id: id.clone(),
            name: input.name,
            workspace: input.workspace,
            repo_path: input.repo_path,
            baseline_ref: input.baseline_ref,
            task: input.task,
            context: input.context,
            verification_command: input.verification_command,
            editable_paths: input.editable_paths,
            metric: input.metric,
            cron: input.cron,
            schedule_id: None,
            target_branch: input.target_branch,
            active: true,
            last_run_id: None,
            best_candidate_id: None,
            created_at: now,
            updated_at: now,
        };

        if let Some(cron) = campaign.cron.clone() {
            let Some(store) = &self.scheduler_store else {
                return Err(Error::Scheduler(
                    "research campaign requested a cron schedule but scheduler is not enabled"
                        .into(),
                ));
            };
            let schedule_id = Uuid::now_v7().to_string();
            let schedule = Schedule {
                id: schedule_id.clone(),
                name: format!("research:{}", campaign.id),
                cron: Some(cron),
                run_at: None,
                timezone: Some("UTC".into()),
                skill: Some("research_campaign_run".into()),
                args: Some(HashMap::from([(
                    "campaign_id".into(),
                    serde_json::json!(campaign.id),
                )])),
                message: None,
                next_run: next_cron_timestamp(campaign.cron.as_deref().unwrap_or_default())?,
                created_at: now.to_rfc3339(),
                completed: false,
            };
            store.add(&schedule).await?;
            campaign.schedule_id = Some(schedule_id);
        }

        self.store.put_campaign(&campaign).await?;
        info!(campaign_id = %campaign.id, name = %campaign.name, "research campaign created");
        Ok(campaign)
    }

    /// List all campaigns.
    pub async fn list_campaigns(&self) -> Result<Vec<ResearchCampaign>> {
        self.store.list_campaigns().await
    }

    /// Get a campaign by ID.
    pub async fn get_campaign(&self, id: &str) -> Result<Option<ResearchCampaign>> {
        self.store.get_campaign(id).await
    }

    /// Delete a campaign, its optional schedule, and all associated runs,
    /// candidates, and promotion requests.
    pub async fn delete_campaign(&self, id: &str) -> Result<bool> {
        if let Some(campaign) = self.store.get_campaign(id).await?
            && let (Some(store), Some(schedule_id)) = (&self.scheduler_store, campaign.schedule_id)
        {
            let _ = store.remove(&schedule_id).await?;
        }
        // Cascade: remove all child entities for this campaign.
        for run in self.store.list_runs(Some(id)).await? {
            self.store.delete_run(&run.id).await?;
        }
        for candidate in self.store.list_candidates(Some(id)).await? {
            self.store.delete_candidate(&candidate.id).await?;
        }
        for request in self.store.list_promotion_requests(Some(id)).await? {
            self.store.delete_promotion_request(&request.id).await?;
        }
        let deleted = self.store.delete_campaign(id).await?;
        if deleted {
            info!(campaign_id = %id, "research campaign deleted");
        }
        Ok(deleted)
    }

    /// Pause a campaign.
    pub async fn pause_campaign(&self, id: &str) -> Result<Option<ResearchCampaign>> {
        self.set_campaign_active(id, false).await
    }

    /// Resume a campaign.
    pub async fn resume_campaign(&self, id: &str) -> Result<Option<ResearchCampaign>> {
        self.set_campaign_active(id, true).await
    }

    async fn set_campaign_active(
        &self,
        id: &str,
        active: bool,
    ) -> Result<Option<ResearchCampaign>> {
        let Some(mut campaign) = self.store.get_campaign(id).await? else {
            return Ok(None);
        };
        campaign.active = active;
        campaign.updated_at = Utc::now();
        self.store.put_campaign(&campaign).await?;
        if active {
            info!(campaign_id = %campaign.id, "research campaign resumed");
        } else {
            info!(campaign_id = %campaign.id, "research campaign paused");
        }
        Ok(Some(campaign))
    }

    /// Run a campaign immediately, returning once the pipeline completes.
    ///
    /// Used in tests and the scheduler. For the HTTP API use
    /// [`Self::run_campaign_async`] which spawns the pipeline as a background
    /// task so the response is not tied to the request connection lifetime.
    pub async fn run_campaign(&self, campaign_id: &str) -> Result<ResearchRun> {
        let Some(mut campaign) = self.store.get_campaign(campaign_id).await? else {
            return Err(Error::ResearchNotFound(format!(
                "campaign '{campaign_id}' not found"
            )));
        };
        if !campaign.active {
            return Err(Error::ResearchConflict(format!(
                "campaign '{}' is paused and cannot be executed",
                campaign.id
            )));
        }

        let now = Utc::now();
        let run_id = Uuid::now_v7().to_string();
        let branch = build_branch_name(&campaign.name);
        let stream_session_id = SessionId::new();
        let mut metadata = HashMap::new();
        metadata.insert(
            "stream_session_id".to_string(),
            serde_json::json!(stream_session_id.to_string()),
        );
        let mut run = ResearchRun {
            id: run_id.clone(),
            campaign_id: campaign.id.clone(),
            status: ResearchRunStatus::Running,
            branch: Some(branch.clone()),
            worktree_path: None,
            candidate_id: None,
            evaluation: None,
            error: None,
            metadata,
            created_at: now,
            started_at: Some(now),
            finished_at: None,
        };
        self.store.put_run(&run).await?;
        info!(campaign_id = %campaign.id, run_id = %run_id, "research campaign run started");

        let result = self
            .execute_campaign_run(&campaign, &run_id, &branch, Some(stream_session_id))
            .await;

        match result {
            Ok((candidate, evaluation)) => {
                run.status = ResearchRunStatus::Completed;
                run.worktree_path = Some(candidate.worktree_path.clone());
                run.candidate_id = Some(candidate.id.clone());
                run.evaluation = Some(evaluation.clone());
                run.finished_at = Some(Utc::now());
                self.store.put_candidate(&candidate).await?;
                self.store.put_run(&run).await?;

                campaign.last_run_id = Some(run.id.clone());
                if candidate.status == CandidateStatus::Kept {
                    let replace_best = match &campaign.best_candidate_id {
                        None => true,
                        Some(best_id) => {
                            if let Some(best) = self.store.get_candidate(best_id).await? {
                                is_candidate_better(best.improvement, candidate.improvement)
                            } else {
                                true
                            }
                        }
                    };
                    if replace_best {
                        campaign.best_candidate_id = Some(candidate.id.clone());
                    }
                }
                campaign.updated_at = Utc::now();
                self.store.put_campaign(&campaign).await?;
                info!(campaign_id = %campaign.id, run_id = %run.id, candidate_id = %candidate.id, "research campaign run completed");
                Ok(run)
            }
            Err(err) => {
                run.status = ResearchRunStatus::Failed;
                run.error = Some(err.to_string());
                run.finished_at = Some(Utc::now());
                self.store.put_run(&run).await?;
                warn!(campaign_id = %campaign.id, run_id = %run.id, %err, "research campaign run failed");
                Err(err)
            }
        }
    }

    /// Run a campaign immediately, spawn the pipeline as a background task,
    /// and return the initial [`ResearchRun`] (status = Running) right away.
    ///
    /// The background task updates the run to Completed / Failed when done.
    /// This avoids holding the HTTP response open for the full pipeline
    /// duration (which can be many minutes for `coding_delegate`).
    pub async fn run_campaign_async(self: Arc<Self>, campaign_id: &str) -> Result<ResearchRun> {
        let Some(mut campaign) = self.store.get_campaign(campaign_id).await? else {
            return Err(Error::ResearchNotFound(format!(
                "campaign '{campaign_id}' not found"
            )));
        };
        if !campaign.active {
            return Err(Error::ResearchConflict(format!(
                "campaign '{}' is paused and cannot be executed",
                campaign.id
            )));
        }

        let now = Utc::now();
        let run_id = Uuid::now_v7().to_string();
        let branch = build_branch_name(&campaign.name);
        let stream_session_id = SessionId::new();
        let mut metadata = HashMap::new();
        metadata.insert(
            "stream_session_id".to_string(),
            serde_json::json!(stream_session_id.to_string()),
        );
        let run = ResearchRun {
            id: run_id.clone(),
            campaign_id: campaign.id.clone(),
            status: ResearchRunStatus::Running,
            branch: Some(branch.clone()),
            worktree_path: None,
            candidate_id: None,
            evaluation: None,
            error: None,
            metadata,
            created_at: now,
            started_at: Some(now),
            finished_at: None,
        };
        self.store.put_run(&run).await?;
        info!(campaign_id = %campaign.id, run_id = %run_id, "research campaign run started");

        let this = self.clone();
        let mut bg_run = run.clone();
        tokio::spawn(async move {
            let result = this
                .execute_campaign_run(&campaign, &run_id, &branch, Some(stream_session_id))
                .await;
            match result {
                Ok((candidate, evaluation)) => {
                    bg_run.status = ResearchRunStatus::Completed;
                    bg_run.worktree_path = Some(candidate.worktree_path.clone());
                    bg_run.candidate_id = Some(candidate.id.clone());
                    bg_run.evaluation = Some(evaluation.clone());
                    bg_run.finished_at = Some(Utc::now());
                    if let Err(e) = this.store.put_candidate(&candidate).await {
                        warn!(run_id = %bg_run.id, %e, "failed to persist research candidate");
                    }
                    if let Err(e) = this.store.put_run(&bg_run).await {
                        warn!(run_id = %bg_run.id, %e, "failed to persist run completion");
                    }
                    campaign.last_run_id = Some(bg_run.id.clone());
                    if candidate.status == CandidateStatus::Kept {
                        let replace_best = match &campaign.best_candidate_id {
                            None => true,
                            Some(best_id) => {
                                if let Ok(Some(best)) = this.store.get_candidate(best_id).await {
                                    is_candidate_better(best.improvement, candidate.improvement)
                                } else {
                                    true
                                }
                            }
                        };
                        if replace_best {
                            campaign.best_candidate_id = Some(candidate.id.clone());
                        }
                    }
                    campaign.updated_at = Utc::now();
                    if let Err(e) = this.store.put_campaign(&campaign).await {
                        warn!(campaign_id = %campaign.id, %e, "failed to persist campaign after run");
                    }
                    info!(campaign_id = %campaign.id, run_id = %bg_run.id, candidate_id = %candidate.id, "research campaign run completed");
                }
                Err(err) => {
                    bg_run.status = ResearchRunStatus::Failed;
                    bg_run.error = Some(err.to_string());
                    bg_run.finished_at = Some(Utc::now());
                    if let Err(e) = this.store.put_run(&bg_run).await {
                        warn!(run_id = %bg_run.id, %e, "failed to persist run failure");
                    }
                    warn!(campaign_id = %campaign.id, run_id = %bg_run.id, %err, "research campaign run failed");
                }
            }
        });

        Ok(run)
    }

    async fn execute_campaign_run(
        &self,
        campaign: &ResearchCampaign,
        run_id: &str,
        branch: &str,
        stream_session_id: Option<SessionId>,
    ) -> Result<(ResearchCandidate, EvaluationResult)> {
        let repo_cwd = Some(campaign.repo_path.clone());
        let worktree = self
            .invoke_skill(
                "git_worktree_create",
                HashMap::from([
                    ("branch".into(), serde_json::json!(branch)),
                    ("base".into(), serde_json::json!(campaign.baseline_ref)),
                    ("agent_id".into(), serde_json::json!("research")),
                ]),
                repo_cwd.clone(),
                None,
            )
            .await?;
        let worktree_path = worktree.data["path"]
            .as_str()
            .ok_or_else(|| Error::Research("git_worktree_create did not return a path".into()))?
            .to_string();

        let coding_context = build_coding_context(campaign);
        let coding_result = self
            .invoke_skill_with_progress(
                "coding_delegate",
                HashMap::from([
                    ("task".into(), serde_json::json!(campaign.task)),
                    ("context".into(), serde_json::json!(coding_context)),
                    (
                        "verification".into(),
                        serde_json::json!(campaign.verification_command),
                    ),
                    ("working_dir".into(), serde_json::json!(worktree_path)),
                ]),
                repo_cwd.clone(),
                Some(worktree_path.clone()),
                stream_session_id,
            )
            .await?;

        let evaluation = self
            .invoke_skill(
                "experiment_run",
                HashMap::from_iter([
                    (
                        "command".into(),
                        serde_json::json!(campaign.verification_command),
                    ),
                    ("working_dir".into(), serde_json::json!(worktree_path)),
                    (
                        "metric_name".into(),
                        serde_json::json!(campaign.metric.as_ref().map(|m| m.name.clone())),
                    ),
                    (
                        "metric_regex".into(),
                        serde_json::json!(campaign.metric.as_ref().and_then(|m| m.regex.clone())),
                    ),
                ]),
                repo_cwd.clone(),
                Some(worktree_path.clone()),
            )
            .await?;
        let evaluation: EvaluationResult = serde_json::from_value(evaluation.data)?;

        let diff_output = self
            .invoke_skill(
                "git_diff",
                HashMap::from([("max_lines".into(), serde_json::json!(200_u64))]),
                repo_cwd,
                Some(worktree_path.clone()),
            )
            .await?;
        let diff_summary = diff_output.data["diff"].as_str().unwrap_or("").to_string();

        let comparison = compare_against_metric(&evaluation, campaign.metric.as_ref());
        let candidate = ResearchCandidate {
            id: Uuid::now_v7().to_string(),
            campaign_id: campaign.id.clone(),
            run_id: run_id.to_string(),
            branch: branch.to_string(),
            worktree_path,
            backend: coding_result.data["backend"].as_str().map(String::from),
            coding_summary: coding_result.data["result"]
                .as_str()
                .unwrap_or("")
                .to_string(),
            diff_summary,
            evaluation: evaluation.clone(),
            improvement: comparison.0,
            status: comparison.1,
            promoted_at: None,
            created_at: Utc::now(),
        };
        Ok((candidate, evaluation))
    }

    /// List runs, optionally filtered by campaign.
    pub async fn list_runs(&self, campaign_id: Option<&str>) -> Result<Vec<ResearchRun>> {
        self.store.list_runs(campaign_id).await
    }

    /// Get a run by ID.
    pub async fn get_run(&self, id: &str) -> Result<Option<ResearchRun>> {
        self.store.get_run(id).await
    }

    /// List candidates, optionally filtered by campaign.
    pub async fn list_candidates(
        &self,
        campaign_id: Option<&str>,
    ) -> Result<Vec<ResearchCandidate>> {
        self.store.list_candidates(campaign_id).await
    }

    /// Get a candidate by ID.
    pub async fn get_candidate(&self, id: &str) -> Result<Option<ResearchCandidate>> {
        self.store.get_candidate(id).await
    }

    /// List promotion requests, optionally filtered by campaign.
    pub async fn list_promotion_requests(
        &self,
        campaign_id: Option<&str>,
    ) -> Result<Vec<ResearchPromotionRequest>> {
        self.store.list_promotion_requests(campaign_id).await
    }

    /// Get a promotion request by ID.
    pub async fn get_promotion_request(
        &self,
        id: &str,
    ) -> Result<Option<ResearchPromotionRequest>> {
        self.store.get_promotion_request(id).await
    }

    /// Submit a promotion attempt.
    ///
    /// When approval is required and `approved` is `false`, this creates a
    /// persistent HITL request instead of merging immediately.
    pub async fn submit_promotion(
        &self,
        candidate_id: &str,
        approved: bool,
    ) -> Result<PromotionSubmission> {
        let (candidate, campaign) = self.load_promotable_candidate(candidate_id).await?;
        if self.promotion_requires_approval(&campaign) && !approved {
            return self
                .create_promotion_request(&candidate, &campaign)
                .await
                .map(|request| PromotionSubmission::ApprovalRequired { request });
        }
        self.promote_candidate(candidate_id, true)
            .await
            .map(|candidate| PromotionSubmission::Promoted {
                candidate: Box::new(candidate),
            })
    }

    /// Approve a pending promotion request and apply the merge.
    pub async fn approve_promotion_request(&self, request_id: &str) -> Result<ResearchCandidate> {
        let Some(mut request) = self.store.get_promotion_request(request_id).await? else {
            return Err(Error::ResearchNotFound(format!(
                "promotion request '{request_id}' not found"
            )));
        };
        if request.status != PromotionRequestStatus::Pending {
            return Err(Error::ResearchConflict(format!(
                "promotion request '{}' is not pending",
                request.id
            )));
        }

        let candidate = self.promote_candidate(&request.candidate_id, true).await?;
        request.status = PromotionRequestStatus::Approved;
        request.resolved_at = Some(Utc::now());
        self.store.put_promotion_request(&request).await?;
        self.save_terminal_promotion_checkpoint(&request, true)
            .await?;
        info!(request_id = %request.id, candidate_id = %request.candidate_id, "promotion request approved");
        Ok(candidate)
    }

    /// Reject a pending promotion request without mutating the target branch.
    pub async fn reject_promotion_request(
        &self,
        request_id: &str,
        reason: Option<String>,
    ) -> Result<ResearchPromotionRequest> {
        let Some(mut request) = self.store.get_promotion_request(request_id).await? else {
            return Err(Error::ResearchNotFound(format!(
                "promotion request '{request_id}' not found"
            )));
        };
        if request.status != PromotionRequestStatus::Pending {
            return Err(Error::ResearchConflict(format!(
                "promotion request '{}' is not pending",
                request.id
            )));
        }
        request.status = PromotionRequestStatus::Rejected;
        request.reason = reason.or_else(|| Some("rejected by human operator".into()));
        request.resolved_at = Some(Utc::now());
        self.store.put_promotion_request(&request).await?;
        self.save_terminal_promotion_checkpoint(&request, false)
            .await?;
        info!(request_id = %request.id, "promotion request rejected");
        Ok(request)
    }

    /// Promote a candidate by merging its branch into the campaign target
    /// branch.
    pub async fn promote_candidate(
        &self,
        candidate_id: &str,
        approved: bool,
    ) -> Result<ResearchCandidate> {
        let (mut candidate, campaign) = self.load_promotable_candidate(candidate_id).await?;
        if self.promotion_requires_approval(&campaign) && !approved {
            return Err(Error::ResearchConflict(format!(
                "promotion of candidate '{}' into '{}' requires explicit approval; submit a promotion request first or retry with approved=true",
                candidate.id, campaign.target_branch
            )));
        }

        self.apply_candidate_promotion(&mut candidate, &campaign)
            .await?;
        Ok(candidate)
    }

    async fn load_promotable_candidate(
        &self,
        candidate_id: &str,
    ) -> Result<(ResearchCandidate, ResearchCampaign)> {
        let Some(candidate) = self.store.get_candidate(candidate_id).await? else {
            return Err(Error::ResearchNotFound(format!(
                "candidate '{candidate_id}' not found"
            )));
        };
        let Some(campaign) = self.store.get_campaign(&candidate.campaign_id).await? else {
            return Err(Error::ResearchNotFound(format!(
                "campaign '{}' not found for candidate '{}'",
                candidate.campaign_id, candidate.id
            )));
        };
        if candidate.status == CandidateStatus::Promoted {
            return Err(Error::ResearchConflict(format!(
                "candidate '{}' is already promoted",
                candidate.id
            )));
        }
        if candidate.status != CandidateStatus::Kept {
            return Err(Error::ResearchConflict(format!(
                "candidate '{}' is not eligible for promotion",
                candidate.id
            )));
        }
        Ok((candidate, campaign))
    }

    async fn apply_candidate_promotion(
        &self,
        candidate: &mut ResearchCandidate,
        campaign: &ResearchCampaign,
    ) -> Result<()> {
        self.invoke_skill(
            "git_checkout",
            HashMap::from([("target".into(), serde_json::json!(campaign.target_branch))]),
            Some(campaign.repo_path.clone()),
            None,
        )
        .await?;
        self.invoke_skill(
            "git_merge",
            HashMap::from([
                ("branch".into(), serde_json::json!(candidate.branch)),
                (
                    "message".into(),
                    serde_json::json!(format!(
                        "chore(research): promote candidate {}",
                        candidate.id
                    )),
                ),
                ("path".into(), serde_json::json!(campaign.repo_path)),
            ]),
            None,
            None,
        )
        .await?;

        candidate.status = CandidateStatus::Promoted;
        candidate.promoted_at = Some(Utc::now());
        self.store.put_candidate(candidate).await?;
        info!(candidate_id = %candidate.id, target_branch = %campaign.target_branch, "research candidate promoted");
        Ok(())
    }

    async fn create_promotion_request(
        &self,
        candidate: &ResearchCandidate,
        campaign: &ResearchCampaign,
    ) -> Result<ResearchPromotionRequest> {
        if let Some(existing) = self
            .find_pending_request_for_candidate(&candidate.id)
            .await?
        {
            return Ok(existing);
        }

        let request = ResearchPromotionRequest {
            id: Uuid::now_v7().to_string(),
            candidate_id: candidate.id.clone(),
            campaign_id: campaign.id.clone(),
            target_branch: campaign.target_branch.clone(),
            status: PromotionRequestStatus::Pending,
            reason: None,
            created_at: Utc::now(),
            resolved_at: None,
        };
        self.store.put_promotion_request(&request).await?;
        self.save_pending_promotion_checkpoint(&request).await?;
        Ok(request)
    }

    async fn find_pending_request_for_candidate(
        &self,
        candidate_id: &str,
    ) -> Result<Option<ResearchPromotionRequest>> {
        let requests = self.store.list_promotion_requests(None).await?;
        Ok(requests.into_iter().find(|request| {
            request.candidate_id == candidate_id
                && request.status == PromotionRequestStatus::Pending
        }))
    }

    fn build_promotion_checkpoint(
        trigger: Envelope,
        run_id: String,
        status: RunStatus,
    ) -> Checkpoint {
        Checkpoint {
            id: CheckpointId::new(),
            run_id,
            session_id: trigger.session_id,
            graph_id: "research-promotion".into(),
            trigger,
            completed_node: "research_promote".into(),
            resume_node: None,
            state: HashMap::new(),
            messages: Vec::new(),
            total_tokens: 0,
            total_iterations: 0,
            agents_executed: vec!["research".into()],
            changelog: Vec::new(),
            status,
            created_at: Utc::now(),
        }
    }

    async fn save_pending_promotion_checkpoint(
        &self,
        request: &ResearchPromotionRequest,
    ) -> Result<()> {
        let Some(store) = &self.checkpoint_store else {
            return Ok(());
        };
        let mut trigger = Envelope::text(
            "research",
            SessionId::new(),
            format!("Approve promotion request {}", request.id),
        );
        trigger.insert_meta("research:promotion_request_id", request.id.clone());
        trigger.insert_meta("research:candidate_id", request.candidate_id.clone());
        let status = RunStatus::Interrupted {
            reason: InterruptReason::HumanApproval {
                tool_name: "research_promote".into(),
                tool_input: serde_json::json!({
                    "request_id": request.id,
                    "candidate_id": request.candidate_id,
                    "target_branch": request.target_branch,
                }),
                agent_id: "research".into(),
            },
        };
        store
            .save(&Self::build_promotion_checkpoint(
                trigger,
                request.id.clone(),
                status,
            ))
            .await
    }

    async fn save_terminal_promotion_checkpoint(
        &self,
        request: &ResearchPromotionRequest,
        approved: bool,
    ) -> Result<()> {
        let Some(store) = &self.checkpoint_store else {
            return Ok(());
        };
        let mut trigger = Envelope::text(
            "research",
            SessionId::new(),
            format!("Promotion request {} resolved", request.id),
        );
        trigger.insert_meta("research:promotion_request_id", request.id.clone());
        let status = if approved {
            RunStatus::Completed
        } else {
            RunStatus::Failed {
                error: request
                    .reason
                    .clone()
                    .unwrap_or_else(|| "rejected by human operator".into()),
            }
        };
        store
            .save(&Self::build_promotion_checkpoint(
                trigger,
                request.id.clone(),
                status,
            ))
            .await
    }

    fn promotion_requires_approval(&self, campaign: &ResearchCampaign) -> bool {
        self.config.require_promotion_approval
            || branch_matches_any(
                &campaign.target_branch,
                &self.config.protected_target_branches,
            )
    }
}

fn validate_campaign_input(input: &CreateResearchCampaign) -> Result<()> {
    if input.name.trim().is_empty() {
        return Err(Error::SkillCategorized {
            message: "campaign name must not be empty".into(),
            category: ErrorCategory::Input,
        });
    }
    if input.task.trim().is_empty() || input.verification_command.trim().is_empty() {
        return Err(Error::SkillCategorized {
            message: "task and verification_command are required".into(),
            category: ErrorCategory::Input,
        });
    }
    if input.editable_paths.is_empty() {
        return Err(Error::SkillCategorized {
            message: "editable_paths must contain at least one path".into(),
            category: ErrorCategory::Input,
        });
    }
    if let Some(metric) = &input.metric
        && let Some(regex) = &metric.regex
    {
        Regex::new(regex).map_err(|e| Error::SkillCategorized {
            message: format!("invalid metric regex: {e}"),
            category: ErrorCategory::Input,
        })?;
    }
    Ok(())
}

fn next_cron_timestamp(expr: &str) -> Result<i64> {
    use std::str::FromStr;
    let schedule = cron::Schedule::from_str(expr)
        .map_err(|e| Error::Scheduler(format!("invalid cron expression: {e}")))?;
    schedule
        .upcoming(Utc)
        .next()
        .map(|dt| dt.timestamp())
        .ok_or_else(|| Error::Scheduler("cron expression has no upcoming run".into()))
}

fn build_branch_name(name: &str) -> String {
    let slug: String = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    format!(
        "research/{}/{}",
        slug.trim_matches('-'),
        Utc::now().format("%Y%m%d%H%M%S")
    )
}

fn build_coding_context(campaign: &ResearchCampaign) -> String {
    let mut parts = Vec::new();
    if let Some(context) = &campaign.context {
        parts.push(context.clone());
    }
    parts.push(format!(
        "Only modify files inside these paths: {}.",
        campaign.editable_paths.join(", ")
    ));
    parts.push(format!(
        "After implementation, the candidate will be verified with: {}",
        campaign.verification_command
    ));
    parts.join("\n\n")
}

fn branch_matches_any(branch: &str, patterns: &[String]) -> bool {
    patterns.iter().any(|pattern| {
        glob::Pattern::new(pattern)
            .map(|compiled| compiled.matches(branch))
            .unwrap_or(false)
    })
}

fn is_candidate_better(current: Option<f64>, candidate: Option<f64>) -> bool {
    match (current, candidate) {
        (_, Some(candidate)) => current.is_none_or(|existing| candidate > existing),
        (None, None) => true,
        (Some(_), None) => false,
    }
}

fn compare_against_metric(
    evaluation: &EvaluationResult,
    metric: Option<&EvaluationMetricConfig>,
) -> (Option<f64>, CandidateStatus) {
    if !evaluation.success {
        return (None, CandidateStatus::Discarded);
    }
    let Some(metric) = metric else {
        return (None, CandidateStatus::Kept);
    };
    let (Some(candidate_value), Some(baseline_value)) =
        (evaluation.metric_value, metric.baseline_value)
    else {
        return (None, CandidateStatus::Kept);
    };
    let improvement = match metric.direction {
        ComparisonDirection::HigherIsBetter => candidate_value - baseline_value,
        ComparisonDirection::LowerIsBetter => baseline_value - candidate_value,
    };
    if metric
        .min_improvement
        .is_some_and(|threshold| improvement < threshold)
    {
        return (Some(improvement), CandidateStatus::Discarded);
    }
    (Some(improvement), CandidateStatus::Kept)
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use async_trait::async_trait;
    use orka_checkpoint::{Checkpoint, CheckpointId, CheckpointStore, RunStatus};
    use orka_core::{
        SkillInput, SkillOutput, SkillSchema, testing::InMemorySecretManager, traits::Skill,
    };
    use orka_skills::SkillRegistry;
    use tokio::sync::RwLock;

    use super::*;
    use crate::{
        create_research_service, create_research_skills, store::InMemoryResearchStore,
        types::CreateResearchCampaign, util::extract_metric,
    };

    struct StaticJsonSkill {
        name: &'static str,
        category: &'static str,
        data: serde_json::Value,
    }

    #[derive(Default)]
    struct TestCheckpointStore {
        inner: RwLock<HashMap<String, Vec<Checkpoint>>>,
    }

    #[async_trait]
    impl CheckpointStore for TestCheckpointStore {
        async fn save(&self, checkpoint: &Checkpoint) -> Result<()> {
            let mut guard = self.inner.write().await;
            guard
                .entry(checkpoint.run_id.clone())
                .or_default()
                .push(checkpoint.clone());
            Ok(())
        }

        async fn load_latest(&self, run_id: &str) -> Result<Option<Checkpoint>> {
            Ok(self
                .inner
                .read()
                .await
                .get(run_id)
                .and_then(|items| items.last().cloned()))
        }

        async fn load(&self, run_id: &str, id: &CheckpointId) -> Result<Option<Checkpoint>> {
            Ok(self
                .inner
                .read()
                .await
                .get(run_id)
                .and_then(|items| items.iter().find(|item| &item.id == id).cloned()))
        }

        async fn list(&self, run_id: &str) -> Result<Vec<CheckpointId>> {
            Ok(self
                .inner
                .read()
                .await
                .get(run_id)
                .map(|items| items.iter().map(|item| item.id.clone()).collect())
                .unwrap_or_default())
        }

        async fn delete_run(&self, run_id: &str) -> Result<()> {
            self.inner.write().await.remove(run_id);
            Ok(())
        }
    }

    #[async_trait]
    impl Skill for StaticJsonSkill {
        fn name(&self) -> &'static str {
            self.name
        }

        fn description(&self) -> &'static str {
            "test helper"
        }

        fn schema(&self) -> SkillSchema {
            SkillSchema::new(serde_json::json!({
                "type": "object",
                "properties": {},
            }))
        }

        fn category(&self) -> &'static str {
            self.category
        }

        async fn execute(&self, _input: SkillInput) -> Result<SkillOutput> {
            Ok(SkillOutput::new(self.data.clone()))
        }
    }

    #[test]
    fn extract_metric_parses_first_capture_group() {
        let metric = extract_metric("score=1.25", Some(r"score=(\d+\.\d+)")).unwrap();
        assert_eq!(metric, Some(1.25));
    }

    #[test]
    fn compare_against_metric_discards_regressions() {
        let evaluation = EvaluationResult {
            command: "make test".into(),
            working_dir: ".".into(),
            exit_code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
            duration_ms: 10,
            success: true,
            metric_name: Some("loss".into()),
            metric_value: Some(0.5),
        };
        let metric = EvaluationMetricConfig {
            name: "loss".into(),
            regex: None,
            direction: ComparisonDirection::LowerIsBetter,
            baseline_value: Some(0.4),
            min_improvement: Some(0.01),
        };
        let (_, status) = compare_against_metric(&evaluation, Some(&metric));
        assert_eq!(status, CandidateStatus::Discarded);
    }

    #[tokio::test]
    async fn run_campaign_creates_kept_candidate() {
        let store = Arc::new(InMemoryResearchStore::new());
        let service = create_research_service(
            store,
            None,
            None,
            ResearchConfig::default(),
            Arc::new(InMemorySecretManager::new()),
            None,
        );

        let mut registry = SkillRegistry::new();
        registry.register(Arc::new(StaticJsonSkill {
            name: "git_worktree_create",
            category: "git_worktree",
            data: serde_json::json!({ "path": "/tmp/research-wt" }),
        }));
        registry.register(Arc::new(StaticJsonSkill {
            name: "coding_delegate",
            category: "coding",
            data: serde_json::json!({ "backend": "codex", "result": "implemented changes" }),
        }));
        registry.register(Arc::new(StaticJsonSkill {
            name: "git_diff",
            category: "git",
            data: serde_json::json!({ "diff": "diff --git a/file b/file" }),
        }));
        registry.register(Arc::new(StaticJsonSkill {
            name: "shell_exec",
            category: "shell",
            data: serde_json::json!({
                "exit_code": 0,
                "stdout": "score=0.95",
                "stderr": "",
                "duration_ms": 12,
            }),
        }));
        for skill in create_research_skills(service.clone()) {
            registry.register(skill);
        }
        service.bind_registry(Arc::new(registry));

        let campaign = service
            .create_campaign(CreateResearchCampaign {
                name: "eval-upgrade".into(),
                workspace: "default".into(),
                repo_path: ".".into(),
                baseline_ref: "HEAD".into(),
                task: "Improve the evaluation score.".into(),
                context: Some("Keep the change minimal.".into()),
                verification_command: "run-eval".into(),
                editable_paths: vec!["crates/orka-research".into()],
                metric: Some(EvaluationMetricConfig {
                    name: "score".into(),
                    regex: Some(r"score=(\d+\.\d+)".into()),
                    direction: ComparisonDirection::HigherIsBetter,
                    baseline_value: Some(0.90),
                    min_improvement: Some(0.01),
                }),
                cron: None,
                target_branch: "main".into(),
            })
            .await
            .unwrap();

        let run = service.run_campaign(&campaign.id).await.unwrap();
        assert_eq!(run.status, ResearchRunStatus::Completed);
        let candidates = service.list_candidates(Some(&campaign.id)).await.unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].status, CandidateStatus::Kept);
        assert_eq!(candidates[0].backend.as_deref(), Some("codex"));
    }

    #[tokio::test]
    async fn promote_candidate_requires_explicit_approval() {
        let store = Arc::new(InMemoryResearchStore::new());
        let checkpoint_store = Arc::new(TestCheckpointStore::default());
        let service = create_research_service(
            store,
            None,
            Some(checkpoint_store.clone()),
            ResearchConfig::default(),
            Arc::new(InMemorySecretManager::new()),
            None,
        );

        let mut registry = SkillRegistry::new();
        registry.register(Arc::new(StaticJsonSkill {
            name: "git_worktree_create",
            category: "git_worktree",
            data: serde_json::json!({ "path": "/tmp/research-wt" }),
        }));
        registry.register(Arc::new(StaticJsonSkill {
            name: "coding_delegate",
            category: "coding",
            data: serde_json::json!({ "backend": "codex", "result": "implemented changes" }),
        }));
        registry.register(Arc::new(StaticJsonSkill {
            name: "git_diff",
            category: "git",
            data: serde_json::json!({ "diff": "diff --git a/file b/file" }),
        }));
        registry.register(Arc::new(StaticJsonSkill {
            name: "shell_exec",
            category: "shell",
            data: serde_json::json!({
                "exit_code": 0,
                "stdout": "score=0.95",
                "stderr": "",
                "duration_ms": 12,
            }),
        }));
        registry.register(Arc::new(StaticJsonSkill {
            name: "git_checkout",
            category: "git",
            data: serde_json::json!({}),
        }));
        registry.register(Arc::new(StaticJsonSkill {
            name: "git_merge",
            category: "git",
            data: serde_json::json!({}),
        }));
        for skill in create_research_skills(service.clone()) {
            registry.register(skill);
        }
        service.bind_registry(Arc::new(registry));

        let campaign = service
            .create_campaign(CreateResearchCampaign {
                name: "promote-me".into(),
                workspace: "default".into(),
                repo_path: ".".into(),
                baseline_ref: "HEAD".into(),
                task: "Improve the evaluation score.".into(),
                context: None,
                verification_command: "run-eval".into(),
                editable_paths: vec!["crates/orka-research".into()],
                metric: None,
                cron: None,
                target_branch: "main".into(),
            })
            .await
            .unwrap();
        let run = service.run_campaign(&campaign.id).await.unwrap();
        let candidate_id = run.candidate_id.as_deref().unwrap();

        let submission = service.submit_promotion(candidate_id, false).await.unwrap();
        let request = match submission {
            PromotionSubmission::ApprovalRequired { request } => request,
            PromotionSubmission::Promoted { .. } => panic!("expected approval request"),
        };
        assert_eq!(request.status, PromotionRequestStatus::Pending);
        let pending = checkpoint_store
            .load_latest(&request.id)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(pending.status, RunStatus::Interrupted { .. }));

        let promoted = service
            .approve_promotion_request(&request.id)
            .await
            .unwrap();
        assert_eq!(promoted.status, CandidateStatus::Promoted);
        assert!(promoted.promoted_at.is_some());
        let resolved = checkpoint_store
            .load_latest(&request.id)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(resolved.status, RunStatus::Completed));
    }
}

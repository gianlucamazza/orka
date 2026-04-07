//! Agent execution budget management.
//!
//! Tracks tool-turn consumption with weighted costs, enforces soft/hard limits,
//! detects repetitive loops, and supports dynamic extension when the agent is
//! making measurable plan progress.
//!
//! # Design
//!
//! The central abstraction is [`BudgetTracker`], which replaces the bare
//! `tool_turns` counter in the agent loop. Tool calls are assigned a floating
//! point cost (see [`tool_step_cost`]) so that cheap read-only operations do
//! not consume the same budget as expensive mutations or long-running shell
//! commands.
//!
//! The budget passes through four [`BudgetZone`]s:
//!
//! - **Normal** — below 80 % consumed.
//! - **Warning** — 80 %–99 % consumed.  A hint is injected asking the LLM to
//!   begin wrapping up.
//! - **Critical** — exactly at the configured limit.  All tools are stripped
//!   from the next iteration, forcing a text-only conclusion.
//! - **Exhausted** — the grace conclusion turn has been used.  Hard stop.

use std::collections::VecDeque;

/// Effective cost of a single tool call counted against the budget.
///
/// Callers may pass a weight explicitly or fall back to [`synthetic_tool_cost`]
/// for runtime-injected tools not backed by a `Skill`.
pub type StepCost = f32;

/// Returns the step cost for a synthetic/meta tool name.
///
/// Only covers tools injected by the agent runtime that are **not** backed by
/// a `Skill` in the registry (planning, handoff, progressive-disclosure).
/// For skill-backed tools the cost is provided by `Skill::budget_cost()` and
/// should be passed directly to [`BudgetTracker::record_batch`].
///
/// - Free (0.0): planning meta-tools and handoff/routing calls.
/// - Full-cost (1.0): anything unknown (conservative default).
pub fn synthetic_tool_cost(tool_name: &str) -> StepCost {
    match tool_name {
        // Planning meta-tools are free — they improve efficiency.
        "create_plan"
        | "update_plan_step"
        | "transfer_to_agent"
        | "delegate_to_agent"
        | "list_tool_categories"
        | "enable_tools" => 0.0,
        _ => 1.0,
    }
}

/// Current budget zone, in ascending order of pressure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum BudgetZone {
    /// Below 80 % consumed — normal operation.
    Normal,
    /// 80 %–99 % consumed — LLM should begin wrapping up.
    Warning,
    /// Exactly at the configured limit — strip tools and force conclusion.
    Critical,
    /// Grace turn exhausted — hard stop.
    Exhausted,
}

/// Manages the tool-turn budget for a single agent run.
///
/// The `consumed` value is a `f32` to support weighted costs; comparisons
/// against `effective_limit` use `consumed.ceil() as usize`.
#[derive(Debug)]
pub struct BudgetTracker {
    /// Original limit from agent config.
    base_limit: usize,
    /// Current effective limit (may be increased by successful plan progress).
    effective_limit: usize,
    /// Fractional steps consumed so far.
    consumed: f32,
    /// Whether the 80 % warning has been emitted.
    pub warned: bool,
    /// Whether the critical (100 %) conclude prompt has been emitted.
    pub concluded: bool,
    /// Budget extensions already granted.
    extensions_granted: usize,
    /// Maximum number of extensions allowed.
    max_extensions: usize,
    /// Each extension adds this many turns.
    extension_size: usize,
    /// Loop detector — tracks recent (`tool_name`, `input_hash`) pairs.
    loop_detector: LoopDetector,
}

impl BudgetTracker {
    /// Create a new tracker with the given base limit and extension config.
    pub fn new(base_limit: usize, max_extensions: usize, extension_size: usize) -> Self {
        Self {
            base_limit,
            effective_limit: base_limit,
            consumed: 0.0,
            warned: false,
            concluded: false,
            extensions_granted: 0,
            max_extensions,
            extension_size,
            loop_detector: LoopDetector::new(6),
        }
    }

    /// Record a batch of tool calls.
    ///
    /// `calls` is a slice of `(tool_name, cost)` pairs.  The cost for
    /// skill-backed tools should come from `Skill::budget_cost()`; for
    /// runtime-injected synthetic tools use [`synthetic_tool_cost`].
    ///
    /// Returns the resulting [`BudgetZone`] after consumption.
    pub fn record_batch(&mut self, calls: &[(&str, StepCost)]) -> BudgetZone {
        let batch_cost: f32 = calls.iter().map(|(_, cost)| cost).sum();
        // Free batches (e.g. only plan tools) do not advance the budget.
        if batch_cost > 0.0 {
            self.consumed += batch_cost;
        }

        self.zone()
    }

    /// Feed the loop detector with a batch of tool calls.
    ///
    /// Should be called with the raw `(tool_name, input)` pairs so the
    /// detector can fingerprint by input content.  Separate from
    /// [`record_batch`] because skill-registry costs and loop-detection inputs
    /// are available at different points in the dispatch pipeline.
    pub fn observe_calls(&mut self, calls: &[(&str, &serde_json::Value)]) {
        for (name, input) in calls {
            self.loop_detector.record(name, input);
        }
    }

    /// Return the current [`BudgetZone`] without advancing consumption.
    pub fn zone(&self) -> BudgetZone {
        let used = self.consumed.ceil() as usize;
        if used > self.effective_limit {
            BudgetZone::Exhausted
        } else if used >= self.effective_limit {
            BudgetZone::Critical
        } else if used * 5 >= self.effective_limit * 4 {
            // >= 80 %
            BudgetZone::Warning
        } else {
            BudgetZone::Normal
        }
    }

    /// Integer steps consumed (ceiling of fractional value).
    pub fn consumed_steps(&self) -> usize {
        self.consumed.ceil() as usize
    }

    /// Remaining integer steps.
    pub fn remaining_steps(&self) -> usize {
        self.effective_limit.saturating_sub(self.consumed_steps())
    }

    /// The current effective limit (may differ from `base_limit` after
    /// extensions).
    pub fn effective_limit(&self) -> usize {
        self.effective_limit
    }

    /// Notify the tracker that a plan step completed (or failed).
    ///
    /// On success: if the budget is under pressure and extensions remain,
    /// grant one extension.  On failure: shrink the effective limit slightly
    /// to encourage the agent to conclude sooner.
    pub fn record_plan_step(&mut self, success: bool) {
        if success {
            let pressure = self.consumed / self.effective_limit as f32;
            if pressure >= 0.6 && self.extensions_granted < self.max_extensions {
                self.effective_limit += self.extension_size;
                self.extensions_granted += 1;
                tracing::debug!(
                    new_limit = self.effective_limit,
                    extensions_granted = self.extensions_granted,
                    "budget extended due to plan progress"
                );
            }
        } else {
            // Shrink by 2, but never below consumed + 1.
            let floor = self.consumed_steps() + 1;
            self.effective_limit = self.effective_limit.saturating_sub(2).max(floor);
        }
    }

    /// Returns `true` if a repetitive loop has been detected in the most
    /// recent tool calls.
    pub fn loop_detected(&self) -> bool {
        self.loop_detector.is_looping()
    }

    /// Format a compact budget line for injection into LLM context.
    ///
    /// Example: `"Steps: 18/25 (7 remaining)"` or, when extended,
    /// `"Steps: 18/30 (12 remaining, +5 extension)"`
    pub fn status_line(&self) -> String {
        let extension = self.effective_limit.saturating_sub(self.base_limit);
        if extension > 0 {
            format!(
                "Steps: {}/{} ({} remaining, +{extension} extension)",
                self.consumed_steps(),
                self.effective_limit,
                self.remaining_steps()
            )
        } else {
            format!(
                "Steps: {}/{} ({} remaining)",
                self.consumed_steps(),
                self.effective_limit,
                self.remaining_steps()
            )
        }
    }

    /// Calculate per-step budget for a plan with `n` steps.
    pub fn budget_per_plan_step(&self, n: usize) -> usize {
        if n == 0 {
            self.remaining_steps()
        } else {
            (self.remaining_steps() / n).max(1)
        }
    }
}

// ---------------------------------------------------------------------------
// Loop detection
// ---------------------------------------------------------------------------

/// Detects when the agent repeatedly invokes the same tool with identical
/// (or near-identical) inputs, indicating it is stuck in a loop.
#[derive(Debug)]
struct LoopDetector {
    recent: VecDeque<(String, u64)>,
    window: usize,
}

impl LoopDetector {
    fn new(window: usize) -> Self {
        Self {
            recent: VecDeque::with_capacity(window),
            window,
        }
    }

    fn record(&mut self, tool_name: &str, input: &serde_json::Value) {
        use std::hash::Hash;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        tool_name.hash(&mut hasher);
        input.to_string().hash(&mut hasher);
        let fingerprint = std::hash::Hasher::finish(&hasher);

        if self.recent.len() >= self.window {
            self.recent.pop_front();
        }
        self.recent.push_back((tool_name.to_string(), fingerprint));
    }

    /// Returns `true` if any (tool, input) pair appears 3 or more times
    /// within the recent window.
    fn is_looping(&self) -> bool {
        if self.recent.len() < 3 {
            return false;
        }
        let mut counts: std::collections::HashMap<(String, u64), u8> =
            std::collections::HashMap::new();
        for entry in &self.recent {
            let c = counts.entry(entry.clone()).or_insert(0);
            *c += 1;
            if *c >= 3 {
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn val(s: &str) -> serde_json::Value {
        serde_json::json!({"cmd": s})
    }

    #[test]
    fn zone_transitions() {
        let mut b = BudgetTracker::new(10, 2, 5);
        assert_eq!(b.zone(), BudgetZone::Normal);

        // Consume 7 full steps (70 %) -> still Normal
        for _ in 0..7 {
            b.record_batch(&[("shell_exec", 1.0)]);
        }
        assert_eq!(b.zone(), BudgetZone::Normal);

        // 8th step -> 80 % -> Warning
        b.record_batch(&[("shell_exec", 1.0)]);
        assert_eq!(b.zone(), BudgetZone::Warning);

        // 10th step -> Critical
        b.record_batch(&[("shell_exec", 1.0)]);
        b.record_batch(&[("shell_exec", 1.0)]);
        assert_eq!(b.zone(), BudgetZone::Critical);

        // 11th step -> Exhausted
        b.record_batch(&[("shell_exec", 1.0)]);
        assert_eq!(b.zone(), BudgetZone::Exhausted);
    }

    #[test]
    fn free_plan_tools_do_not_consume_budget() {
        let mut b = BudgetTracker::new(5, 2, 5);
        // synthetic_tool_cost returns 0.0 for planning tools
        b.record_batch(&[("create_plan", synthetic_tool_cost("create_plan"))]);
        b.record_batch(&[("update_plan_step", synthetic_tool_cost("update_plan_step"))]);
        assert_eq!(b.consumed_steps(), 0);
        assert_eq!(b.zone(), BudgetZone::Normal);
    }

    #[test]
    fn read_only_tools_cost_half() {
        let mut b = BudgetTracker::new(10, 2, 5);
        // 4 reads at 0.5 each = 2.0 fractional steps
        for _ in 0..4 {
            b.record_batch(&[("fs_read", 0.5)]);
        }
        assert_eq!(b.consumed_steps(), 2);
    }

    #[test]
    fn loop_detection() {
        let mut b = BudgetTracker::new(20, 2, 5);
        let input = val("same");
        for _ in 0..2 {
            b.record_batch(&[("shell_exec", 1.0)]);
            b.observe_calls(&[("shell_exec", &input)]);
            assert!(!b.loop_detected());
        }
        b.record_batch(&[("shell_exec", 1.0)]);
        b.observe_calls(&[("shell_exec", &input)]);
        assert!(b.loop_detected());
    }

    #[test]
    fn plan_progress_extends_budget() {
        let mut b = BudgetTracker::new(10, 2, 5);
        // Consume 70 % (7 steps)
        for _ in 0..7 {
            b.record_batch(&[("shell_exec", 1.0)]);
        }
        assert_eq!(b.extensions_granted, 0);
        b.record_plan_step(true);
        // 70 % >= 60 % threshold -> extension granted
        assert_eq!(b.extensions_granted, 1);
        assert_eq!(b.effective_limit(), 15);
    }

    #[test]
    fn plan_failure_shrinks_budget() {
        let mut b = BudgetTracker::new(10, 2, 5);
        b.record_plan_step(false);
        // 10 - 2 = 8, floored at consumed+1=1
        assert_eq!(b.effective_limit(), 8);
    }

    #[test]
    fn budget_per_plan_step() {
        let b = BudgetTracker::new(10, 2, 5);
        // 10 remaining / 3 steps = 3
        assert_eq!(b.budget_per_plan_step(3), 3);
        assert_eq!(b.budget_per_plan_step(0), 10);
    }
}

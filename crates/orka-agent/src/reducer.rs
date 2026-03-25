//! State reducer strategies for concurrent fan-out writes.
//!
//! In a fan-out node multiple agent branches execute in parallel and may write
//! to the same [`SlotKey`].  Without a reducer the outcome is
//! **last-write-wins** which is non-deterministic.  Reducers make the merge
//! semantics explicit and deterministic.
//!
//! ## Key format
//!
//! Reducers are registered on an [`AgentGraph`] with a string key in
//! `"namespace::name"` format (e.g. `"__shared::results"`).  The same
//! encoding is used by [`SerializableSlotKey`] in checkpoints.
//!
//! [`SlotKey`]: crate::context::SlotKey
//! [`AgentGraph`]: crate::graph::AgentGraph
//! [`SerializableSlotKey`]: orka_checkpoint::SerializableSlotKey

use serde_json::Value;

/// How concurrent writes to the same slot are merged.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ReducerStrategy {
    /// The last write wins (current default behaviour).
    #[default]
    LastWriteWins,
    /// Collect all written values into a JSON array.
    ///
    /// `old` is expected to be an array or absent; each new value is appended.
    Append,
    /// Merge JSON objects with shallow key-merge semantics.
    ///
    /// Keys in `new` overwrite keys in `old`.  Non-object values fall back to
    /// [`LastWriteWins`](Self::LastWriteWins).
    MergeObject,
    /// Sum numeric values.  Non-numeric or absent `old` is treated as `0`.
    Sum,
    /// Keep the larger of the two numeric values.
    Max,
    /// Keep the smaller of the two numeric values.
    Min,
}

/// Apply `strategy` to produce the merged value given an optional previous
/// value and a new incoming value.
///
/// Returns the value that should be stored in the slot.
pub fn apply_reducer(strategy: ReducerStrategy, old: Option<&Value>, new: &Value) -> Value {
    match strategy {
        ReducerStrategy::LastWriteWins => new.clone(),

        ReducerStrategy::Append => {
            let mut arr = match old {
                Some(Value::Array(v)) => v.clone(),
                Some(other) => vec![other.clone()], // coerce scalar to array
                None => vec![],
            };
            arr.push(new.clone());
            Value::Array(arr)
        }

        ReducerStrategy::MergeObject => {
            match (old, new) {
                (Some(Value::Object(base)), Value::Object(incoming)) => {
                    let mut merged = base.clone();
                    for (k, v) in incoming {
                        merged.insert(k.clone(), v.clone());
                    }
                    Value::Object(merged)
                }
                // Non-objects: fall back to last-write-wins.
                _ => new.clone(),
            }
        }

        ReducerStrategy::Sum => {
            let a = old.and_then(Value::as_f64).unwrap_or(0.0);
            let b = new.as_f64().unwrap_or(0.0);
            serde_json::json!(a + b)
        }

        ReducerStrategy::Max => {
            let a = old.and_then(Value::as_f64).unwrap_or(f64::NEG_INFINITY);
            let b = new.as_f64().unwrap_or(f64::NEG_INFINITY);
            serde_json::json!(f64::max(a, b))
        }

        ReducerStrategy::Min => {
            let a = old.and_then(Value::as_f64).unwrap_or(f64::INFINITY);
            let b = new.as_f64().unwrap_or(f64::INFINITY);
            serde_json::json!(f64::min(a, b))
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn last_write_wins() {
        let result = apply_reducer(ReducerStrategy::LastWriteWins, Some(&json!(1)), &json!(2));
        assert_eq!(result, json!(2));
    }

    #[test]
    fn append_to_existing_array() {
        let old = json!([1, 2]);
        let result = apply_reducer(ReducerStrategy::Append, Some(&old), &json!(3));
        assert_eq!(result, json!([1, 2, 3]));
    }

    #[test]
    fn append_from_none() {
        let result = apply_reducer(ReducerStrategy::Append, None, &json!("hello"));
        assert_eq!(result, json!(["hello"]));
    }

    #[test]
    fn append_coerces_scalar_old() {
        let result = apply_reducer(ReducerStrategy::Append, Some(&json!("a")), &json!("b"));
        assert_eq!(result, json!(["a", "b"]));
    }

    #[test]
    fn merge_object_shallow() {
        let old = json!({"a": 1, "b": 2});
        let new = json!({"b": 99, "c": 3});
        let result = apply_reducer(ReducerStrategy::MergeObject, Some(&old), &new);
        assert_eq!(result, json!({"a": 1, "b": 99, "c": 3}));
    }

    #[test]
    fn merge_object_fallback_for_non_objects() {
        let result = apply_reducer(ReducerStrategy::MergeObject, Some(&json!(1)), &json!(2));
        assert_eq!(result, json!(2));
    }

    #[test]
    fn sum_numbers() {
        let result = apply_reducer(ReducerStrategy::Sum, Some(&json!(10.0)), &json!(5.0));
        assert_eq!(result.as_f64().unwrap(), 15.0);
    }

    #[test]
    fn sum_from_none() {
        let result = apply_reducer(ReducerStrategy::Sum, None, &json!(7.0));
        assert_eq!(result.as_f64().unwrap(), 7.0);
    }

    #[test]
    fn max_keeps_larger() {
        assert_eq!(
            apply_reducer(ReducerStrategy::Max, Some(&json!(3.0)), &json!(5.0)).as_f64(),
            Some(5.0)
        );
        assert_eq!(
            apply_reducer(ReducerStrategy::Max, Some(&json!(7.0)), &json!(2.0)).as_f64(),
            Some(7.0)
        );
    }

    #[test]
    fn min_keeps_smaller() {
        assert_eq!(
            apply_reducer(ReducerStrategy::Min, Some(&json!(3.0)), &json!(5.0)).as_f64(),
            Some(3.0)
        );
        assert_eq!(
            apply_reducer(ReducerStrategy::Min, Some(&json!(7.0)), &json!(2.0)).as_f64(),
            Some(2.0)
        );
    }
}

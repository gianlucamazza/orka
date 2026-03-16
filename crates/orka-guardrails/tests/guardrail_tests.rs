use std::sync::Arc;

use orka_core::config::{GuardrailsConfig, RedactPattern};
use orka_core::traits::{Guardrail, GuardrailDecision};
use orka_core::Session;
use orka_guardrails::{create_guardrail, GuardrailChain, KeywordGuardrail, RegexGuardrail};

fn session() -> Session {
    Session::new("test", "user1")
}

// --- create_guardrail factory tests ---

#[tokio::test]
async fn create_guardrail_returns_none_when_empty() {
    let config = GuardrailsConfig {
        blocked_keywords: vec![],
        block_patterns: vec![],
        redact_patterns: vec![],
        pii_filter: false,
    };
    assert!(create_guardrail(&config).is_none());
}

#[tokio::test]
async fn create_guardrail_returns_none_for_empty_keyword_list() {
    let config = GuardrailsConfig {
        blocked_keywords: vec![],
        block_patterns: vec![],
        redact_patterns: vec![],
        pii_filter: false,
    };
    assert!(create_guardrail(&config).is_none());
}

#[tokio::test]
async fn create_guardrail_with_keywords_only() {
    let config = GuardrailsConfig {
        blocked_keywords: vec!["spam".into(), "scam".into()],
        block_patterns: vec![],
        redact_patterns: vec![],
        pii_filter: false,
    };
    let guard = create_guardrail(&config).expect("should return Some");
    let decision = guard.check_input("this is spam", &session()).await.unwrap();
    assert!(matches!(decision, GuardrailDecision::Block(_)));

    let decision = guard.check_input("hello world", &session()).await.unwrap();
    assert!(matches!(decision, GuardrailDecision::Allow));
}

#[tokio::test]
async fn create_guardrail_with_pii_filter() {
    let config = GuardrailsConfig {
        blocked_keywords: vec![],
        block_patterns: vec![],
        redact_patterns: vec![],
        pii_filter: true,
    };
    let guard = create_guardrail(&config).expect("should return Some");
    let decision = guard
        .check_output("reach me at alice@test.org", &session())
        .await
        .unwrap();
    match decision {
        GuardrailDecision::Modify(text) => {
            assert!(text.contains("[EMAIL]"));
            assert!(!text.contains("alice@test.org"));
        }
        other => panic!("expected Modify, got {:?}", other),
    }
}

#[tokio::test]
async fn create_guardrail_with_block_and_redact_patterns() {
    let config = GuardrailsConfig {
        blocked_keywords: vec![],
        block_patterns: vec![r"(?i)secret_key\s*=".into()],
        redact_patterns: vec![RedactPattern {
            pattern: r"\d{4}-\d{4}-\d{4}-\d{4}".into(),
            replacement: "[CARD]".into(),
        }],
        pii_filter: false,
    };
    let guard = create_guardrail(&config).expect("should return Some");

    // Block pattern triggers
    let decision = guard
        .check_input("secret_key = abc123", &session())
        .await
        .unwrap();
    assert!(matches!(decision, GuardrailDecision::Block(_)));

    // Redact pattern triggers
    let decision = guard
        .check_output("card: 1234-5678-9012-3456", &session())
        .await
        .unwrap();
    match decision {
        GuardrailDecision::Modify(text) => {
            assert!(text.contains("[CARD]"));
            assert!(!text.contains("1234-5678-9012-3456"));
        }
        other => panic!("expected Modify, got {:?}", other),
    }
}

// --- PII tests ---

#[tokio::test]
async fn multiple_pii_types_in_one_message() {
    let guard = RegexGuardrail::new().with_pii_filter();
    let input = "Email: bob@example.com, Phone: 555-867-5309, SSN: 123-45-6789";
    let decision = guard.check_output(input, &session()).await.unwrap();
    match decision {
        GuardrailDecision::Modify(text) => {
            assert!(text.contains("[EMAIL]"), "missing [EMAIL] in: {text}");
            assert!(text.contains("[PHONE]"), "missing [PHONE] in: {text}");
            assert!(text.contains("[SSN]"), "missing [SSN] in: {text}");
            assert!(!text.contains("bob@example.com"));
            assert!(!text.contains("555-867-5309"));
            assert!(!text.contains("123-45-6789"));
        }
        other => panic!("expected Modify, got {:?}", other),
    }
}

#[tokio::test]
async fn ssn_redaction() {
    let guard = RegexGuardrail::new().with_pii_filter();
    let decision = guard
        .check_output("SSN is 999-88-7777", &session())
        .await
        .unwrap();
    match decision {
        GuardrailDecision::Modify(text) => {
            assert_eq!(text, "SSN is [SSN]");
        }
        other => panic!("expected Modify, got {:?}", other),
    }
}

// --- Chain behavior tests ---

#[tokio::test]
async fn chain_block_takes_priority_over_modify() {
    // PII filter would modify, but keyword blocker should block first
    // Chain processes in order: PII filter runs first (modifies), then keyword blocker blocks.
    // But if the keyword is still present after modification, it blocks.
    let chain = GuardrailChain::new()
        .add(Arc::new(RegexGuardrail::new().with_pii_filter()))
        .add(Arc::new(KeywordGuardrail::new(vec!["attack".into()])));

    let decision = chain
        .check_input("plan an attack on test@example.com", &session())
        .await
        .unwrap();
    assert!(matches!(decision, GuardrailDecision::Block(_)));
}

#[tokio::test]
async fn chain_multiple_modifiers_accumulate() {
    let redactor1 = RegexGuardrail::new().add_redact_pattern(r"foo", "[X]");
    let redactor2 = RegexGuardrail::new().add_redact_pattern(r"bar", "[Y]");

    let chain = GuardrailChain::new()
        .add(Arc::new(redactor1))
        .add(Arc::new(redactor2));

    let decision = chain.check_input("foo and bar", &session()).await.unwrap();
    match decision {
        GuardrailDecision::Modify(text) => {
            assert_eq!(text, "[X] and [Y]");
        }
        other => panic!("expected Modify, got {:?}", other),
    }
}

// --- Input vs output both checked ---

#[tokio::test]
async fn check_input_and_output_both_work() {
    let guard = KeywordGuardrail::new(vec!["banned".into()]);
    let s = session();

    let input_decision = guard.check_input("this is banned", &s).await.unwrap();
    assert!(matches!(input_decision, GuardrailDecision::Block(_)));

    let output_decision = guard.check_output("this is banned", &s).await.unwrap();
    assert!(matches!(output_decision, GuardrailDecision::Block(_)));

    let input_ok = guard.check_input("this is fine", &s).await.unwrap();
    assert!(matches!(input_ok, GuardrailDecision::Allow));

    let output_ok = guard.check_output("this is fine", &s).await.unwrap();
    assert!(matches!(output_ok, GuardrailDecision::Allow));
}

// --- Unicode ---

#[tokio::test]
async fn unicode_blocked_keywords() {
    let guard = KeywordGuardrail::new(vec!["verboten".into(), "\u{1F4A3}".into()]);

    let decision = guard
        .check_input("this has a \u{1F4A3} in it", &session())
        .await
        .unwrap();
    assert!(matches!(decision, GuardrailDecision::Block(_)));

    let decision = guard
        .check_input("VERBOTEN content here", &session())
        .await
        .unwrap();
    assert!(matches!(decision, GuardrailDecision::Block(_)));
}

// --- Large input ---

#[tokio::test]
async fn large_input_does_not_panic() {
    let guard = RegexGuardrail::new()
        .with_pii_filter()
        .add_block_pattern(r"(?i)danger");

    let large = "a".repeat(1_000_000);
    let decision = guard.check_input(&large, &session()).await.unwrap();
    assert!(matches!(decision, GuardrailDecision::Allow));

    // Large input with a match buried inside
    let mut with_match = "b".repeat(500_000);
    with_match.push_str(" danger ");
    with_match.push_str(&"c".repeat(500_000));
    let decision = guard.check_input(&with_match, &session()).await.unwrap();
    assert!(matches!(decision, GuardrailDecision::Block(_)));
}

//! Phase-8 REPLAY + REPEATS (ignored; requires network + key).
//!
//! Run AFTER `live_advisory_phase8` (reads its `shadow-phase8.jsonl`):
//! `TYPESAFE_API_KEY=... cargo test -p codebro-mcp-server
//!  --test jev_advisory_phase8_replay -- --ignored --nocapture`
//!
//! - Stratified replay: 10 per workspace block = 30 (sanitized state +
//!   questions re-submitted; no CodeBro execution).
//! - Repeated-decision probe: 10 boundary-interest states x3 runs (direct
//!   client calls, unlogged from headline N).
//! - Writes `/tmp/opencode/jev-phase8/replay.json`. Log-only throughout.

use codebro_jev_shadow::{
    adapter::JevClient, config::JevShadowConfig, logging::read_records,
    replay::replay_records, shadow::ShadowRecord,
};
use std::path::Path;
use std::time::Duration;

const OUT_DIR: &str = "/tmp/opencode/jev-phase8";

fn spread(xs: &[f64]) -> f64 {
    let mut v: Vec<f64> = xs.iter().copied().filter(|x| x.is_finite()).collect();
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() - 1] - v[0]
}

#[tokio::test]
#[ignore]
async fn advisory_replay_phase8() {
    assert!(
        std::env::var("TYPESAFE_API_KEY").ok().filter(|s| !s.trim().is_empty()).is_some(),
        "replay needs TYPESAFE_API_KEY in env (never printed)"
    );
    let shadow_log = Path::new(OUT_DIR).join("shadow-phase8.jsonl");
    let (records, skipped) = read_records(&shadow_log);
    assert!(!records.is_empty(), "run live_advisory_phase8 first");
    println!("READ shadow records={} skipped_lines={}", records.len(), skipped);

    let cfg = JevShadowConfig {
        enabled: true,
        advisory_enabled: true,
        endpoint: std::env::var("JEV_SHADOW_ENDPOINT")
            .unwrap_or_else(|_| "https://api.typesafe.ai/v1/systemone".to_string()),
        model: std::env::var("JEV_SHADOW_MODEL").unwrap_or_else(|_| "jev-1.13.0".to_string()),
        timeout: Duration::from_millis(8000),
        max_retries: 1,
        log_path_override: None,
        advisory_log_path_override: None,
    };

    // Advisory events carry provenance; shadow records are in submission
    // order (codebro 0..100, conductor 100..200, mycontext 200..300).
    let adv: Vec<serde_json::Value> = std::fs::read_to_string(Path::new(OUT_DIR).join("advisory-phase8.jsonl"))
        .unwrap()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let prov_of = |idx: usize| -> String {
        adv.get(idx)
            .and_then(|a| a.get("provenance"))
            .and_then(|p| p.as_str())
            .unwrap_or("unknown")
            .to_string()
    };

    // ── Stratified replay: 10 per workspace block ──
    // Prefer: DISAGREE first, then UNCERTAIN, then AGREE (boundary-heavy).
    let mut replay_set: Vec<ShadowRecord> = Vec::new();
    for block in [0usize, 100usize, 200usize] {
        let mut dis: Vec<ShadowRecord> = Vec::new();
        let mut unc: Vec<ShadowRecord> = Vec::new();
        let mut agr: Vec<ShadowRecord> = Vec::new();
        for r in records.iter().skip(block).take(100) {
            match r.agreement.as_str() {
                "DISAGREE" => dis.push(r.clone()),
                "JEV_UNCERTAIN" => unc.push(r.clone()),
                _ => agr.push(r.clone()),
            }
        }
        let mut take: Vec<ShadowRecord> = Vec::new();
        take.extend(dis.into_iter().take(4));
        take.extend(unc.into_iter().take(3));
        take.extend(agr.into_iter().take(10 - take.len().min(10)));
        if take.len() < 10 {
            for r in records.iter().skip(block).take(100) {
                if take.len() >= 10 {
                    break;
                }
                if !take.iter().any(|t: &ShadowRecord| t.decision.state_hash == r.decision.state_hash) {
                    take.push(r.clone());
                }
            }
        }
        replay_set.extend(take.into_iter().take(10));
    }
    println!("REPLAY_SET size={} (target 30)", replay_set.len());
    let replay_outcomes = replay_records(&cfg, &replay_set).await;
    let same = replay_outcomes.iter().filter(|o| o.reproducible == Some(true)).count();
    let diff = replay_outcomes.iter().filter(|o| o.reproducible == Some(false)).count();
    let na = replay_outcomes.iter().filter(|o| o.reproducible.is_none()).count();
    println!("REPLAY same={same} different={diff} unavailable={na}");

    // ── Repeats: 10 states x3 (boundary-interest first) ──
    let ok_idx: Vec<usize> = records
        .iter()
        .enumerate()
        .filter(|(_, r)| r.decision.request_status == codebro_jev_shadow::types::RequestStatus::Ok)
        .map(|(i, _)| i)
        .collect();
    let mut targets: Vec<usize> = Vec::new();
    for i in &ok_idx {
        let p = prov_of(*i);
        if (p.contains("propose-flag-trap") || p.contains("s16-boundary")) && targets.len() < 10 {
            targets.push(*i);
        }
    }
    for i in &ok_idx {
        if targets.len() >= 10 {
            break;
        }
        let p = prov_of(*i);
        if (p.contains("qualifier-true") || p.contains("skill-approve-true"))
            && !targets.contains(i)
        {
            targets.push(*i);
        }
    }
    for i in &ok_idx {
        if targets.len() >= 10 {
            break;
        }
        if !targets.contains(i) {
            targets.push(*i);
        }
    }
    targets.truncate(10);
    println!("REPEAT_TARGETS n={}", targets.len());
    let client = JevClient::new(&cfg);
    let mut repeats: Vec<serde_json::Value> = Vec::new();
    for t in &targets {
        let rec = &records[*t];
        let mut run_results = Vec::new();
        let mut run_probs = Vec::new();
        let mut run_confs = Vec::new();
        let mut run_lat = Vec::new();
        let mut run_in = Vec::new();
        let mut run_out = Vec::new();
        for _ in 0..3 {
            let call = client.evaluate(&rec.state, &rec.questions).await;
            run_results.push(
                call.answers
                    .get(&rec.question_id)
                    .map(|a| a.result_string())
                    .unwrap_or_else(|| "<unavailable>".to_string()),
            );
            run_probs.push(call.answers.get(&rec.question_id).and_then(|a| a.probabilities.clone()));
            run_confs.push(call.answers.get(&rec.question_id).and_then(|a| {
                a.confidence.or_else(|| a.noul.map(|p| (p - 0.5).abs() * 2.0))
            }));
            run_lat.push(call.latency_ms);
            run_in.push(call.input_tokens);
            run_out.push(call.output_tokens);
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        let all_same = run_results.iter().all(|r| r == &run_results[0]);
        let exact_prob_eq = run_probs.iter().all(|p| p == &run_probs[0]);
        let conf_spread =
            spread(&run_confs.iter().map(|c| c.unwrap_or(f64::NAN)).collect::<Vec<_>>());
        repeats.push(serde_json::json!({
            "question_id": rec.question_id,
            "question_set_version": rec.question_set_version,
            "provenance": prov_of(*t),
            "original_result": rec.decision.result,
            "original_confidence": rec.decision.confidence,
            "runs": run_results,
            "answer_agreement": all_same,
            "exact_probability_equality": exact_prob_eq,
            "confidence_spread": conf_spread,
            "latencies_ms": run_lat,
            "input_tokens": run_in,
            "output_tokens": run_out,
            "model": rec.decision.model,
        }));
        println!(
            "REPEAT {} agree={all_same} spread={conf_spread:.3}",
            prov_of(*t).chars().take(80).collect::<String>()
        );
    }

    let out = serde_json::json!({
        "replayed": replay_outcomes.len(),
        "same": same,
        "different": diff,
        "unavailable": na,
        "outcomes": replay_outcomes,
        "replay_set_provenance": replay_set.iter().map(|r| {
            records.iter().position(|x| x.decision.state_hash == r.decision.state_hash)
                .map(prov_of).unwrap_or_else(|| "unknown".to_string())
        }).collect::<Vec<_>>(),
        "repeat_targets": repeats.len(),
        "repeats": repeats,
    });
    std::fs::write(Path::new(OUT_DIR).join("replay.json"), serde_json::to_string_pretty(&out).unwrap())
        .unwrap();
    println!("WROTE replay.json");
}

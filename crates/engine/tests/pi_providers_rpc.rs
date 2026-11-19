//! Pi custom providers over the RPC wire: save → list → remove against a
//! whole engine. Its own test binary so `PI_CODING_AGENT_DIR` can point Pi's
//! `models.json` at a tempdir before the engine reads it — never the real
//! `~/.pi/agent`.

use std::sync::Arc;

use zeron_engine::{EngineCore, HarnessRegistry};
use zeron_harness::mock::MockHarness;
use zeron_proto::HarnessId;
use zeron_rpc::methods;

#[tokio::test]
async fn pi_providers_round_trip_over_rpc_without_echoing_the_key() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let pi_dir = tmp.path().join("pi-agent");
    // SAFETY: the only test in this binary, set before the engine (or any
    // other thread that reads the environment) starts.
    unsafe { std::env::set_var("PI_CODING_AGENT_DIR", &pi_dir) };
    let data = tmp.path().join("data");
    std::fs::create_dir_all(&data).expect("data dir");
    let registry = HarnessRegistry::new();
    registry.register(Arc::new(MockHarness { script: Vec::new() }));
    let core = EngineCore::assemble(&data, Arc::new(registry), HarnessId::Mock, None)
        .expect("engine assembles");
    let client = zeron_rpc::memory_client(core.rpc_service());

    let empty = client
        .call(methods::LIST_PI_PROVIDERS, serde_json::json!({}))
        .await
        .expect("ListPiProviders");
    assert_eq!(empty, serde_json::json!([]));

    let saved = client
        .call(
            methods::SAVE_PI_PROVIDER,
            serde_json::json!({
                "name": "omni",
                "baseUrl": "https://omni.example.com/v1",
                "api": "openai-completions",
                "apiKey": "sk-wire-secret-9876",
                "models": [{ "id": "dashscope-intl/deepseek-v4.1-flash-1017" }],
            }),
        )
        .await
        .expect("SavePiProvider");
    assert_eq!(saved[0]["name"], "omni");
    assert_eq!(saved[0]["keySource"], "literal");
    assert_eq!(saved[0]["keyTail"], "9876");
    let stored: serde_json::Value =
        serde_json::from_slice(&std::fs::read(pi_dir.join("models.json")).expect("models.json"))
            .expect("json");
    assert_eq!(stored["providers"]["omni"]["apiKey"], "sk-wire-secret-9876");

    let listed = client
        .call(methods::LIST_PI_PROVIDERS, serde_json::json!({}))
        .await
        .expect("ListPiProviders");
    assert_eq!(listed, saved);
    assert!(!format!("{saved}{listed}").contains("sk-wire-secret"));

    let refused = client
        .call(
            methods::SAVE_PI_PROVIDER,
            serde_json::json!({
                "name": "openai", "baseUrl": "https://x.example/v1",
                "api": "openai-completions", "apiKey": "k", "models": [{ "id": "m" }],
            }),
        )
        .await
        .expect_err("built-in names are refused");
    assert!(refused.to_string().contains("built-in"), "{refused}");

    let removed = client
        .call(
            methods::REMOVE_PI_PROVIDER,
            serde_json::json!({ "name": "omni" }),
        )
        .await
        .expect("RemovePiProvider");
    assert_eq!(removed, serde_json::json!([]));
}

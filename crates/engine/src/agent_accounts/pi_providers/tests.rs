//! Pi `models.json` editing against temp dirs, and model discovery against
//! a local one-request-at-a-time HTTP stub — never a real endpoint.

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use zeron_proto::{PiKeySource, PiProviderModel, SavePiProvider};

use super::super::{AgentAccounts, AgentAccountsConfig};

fn accounts(root: &std::path::Path) -> (AgentAccounts, std::path::PathBuf) {
    let config = AgentAccountsConfig::isolated(root);
    let file = config.pi_agent_dir.join("models.json");
    (AgentAccounts::new(config), file)
}

fn read(file: &std::path::Path) -> Value {
    serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap()
}

fn write(file: &std::path::Path, value: &Value) {
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

fn model(id: &str) -> PiProviderModel {
    PiProviderModel {
        id: id.into(),
        name: None,
    }
}

fn omni(key: Option<&str>) -> SavePiProvider {
    SavePiProvider {
        name: "omni".into(),
        previous_name: None,
        base_url: "https://omni.example.com/v1/".into(),
        api: "openai-completions".into(),
        api_key: key.map(str::to_string),
        models: vec![model("dashscope-intl/deepseek-v4.1-flash-1017")],
    }
}

#[test]
fn creates_models_json_with_a_pi_provider_entry() {
    let dir = tempfile::tempdir().unwrap();
    let (accounts, file) = accounts(dir.path());
    assert!(accounts.list_pi_providers().unwrap().is_empty());

    let list = accounts.save_pi_provider(omni(Some("sk-test-123456"))).unwrap();

    assert_eq!(
        read(&file),
        json!({ "providers": { "omni": {
            "baseUrl": "https://omni.example.com/v1",
            "api": "openai-completions",
            "apiKey": "sk-test-123456",
            "models": [{ "id": "dashscope-intl/deepseek-v4.1-flash-1017" }],
        }}})
    );
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].key_source, PiKeySource::Literal);
    assert_eq!(list[0].key_tail.as_deref(), Some("3456"));
    // The key itself is never part of what clients see.
    assert!(!serde_json::to_string(&list).unwrap().contains("sk-test"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}

#[test]
fn edits_keep_other_providers_unknown_fields_and_the_stored_key() {
    let dir = tempfile::tempdir().unwrap();
    let (accounts, file) = accounts(dir.path());
    let other = json!({
        "baseUrl": "https://other.example/api", "api": "anthropic-messages",
        "apiKey": "!pass show other", "models": [{ "id": "x" }],
    });
    write(
        &file,
        &json!({
            "theme": "kept",
            "providers": {
                "other": other,
                "omni": {
                    "baseUrl": "https://old.example/v1", "api": "openai-completions",
                    "apiKey": "sk-old-abcdefgh", "headers": { "X-Team": "a" },
                    "models": [{ "id": "a", "contextWindow": 64000 }, { "id": "b" }],
                },
            },
        }),
    );

    let mut edit = omni(None);
    edit.previous_name = Some("omni".into());
    edit.models = vec![
        PiProviderModel {
            id: "a".into(),
            name: Some("Model A".into()),
        },
        model("c"),
    ];
    accounts.save_pi_provider(edit).unwrap();

    let stored = read(&file);
    assert_eq!(stored["theme"], "kept");
    assert_eq!(stored["providers"]["other"], other);
    assert_eq!(
        stored["providers"]["omni"],
        json!({
            "baseUrl": "https://omni.example.com/v1", "api": "openai-completions",
            "apiKey": "sk-old-abcdefgh", "headers": { "X-Team": "a" },
            "models": [{ "id": "a", "name": "Model A", "contextWindow": 64000 }, { "id": "c" }],
        })
    );
    let list = accounts.list_pi_providers().unwrap();
    let other = list.iter().find(|p| p.name == "other").unwrap();
    assert_eq!(other.key_source, PiKeySource::Command);
    assert_eq!(other.key_tail, None);
}

#[test]
fn renames_and_removes() {
    let dir = tempfile::tempdir().unwrap();
    let (accounts, file) = accounts(dir.path());
    accounts.save_pi_provider(omni(Some("sk-one-12345678"))).unwrap();

    let mut rename = omni(None);
    rename.previous_name = Some("omni".into());
    rename.name = "omni-router".into();
    let list = accounts.save_pi_provider(rename).unwrap();
    assert_eq!(list.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["omni-router"]);
    assert_eq!(read(&file)["providers"]["omni-router"]["apiKey"], "sk-one-12345678");

    assert!(accounts.remove_pi_provider("omni-router").unwrap().is_empty());
    assert_eq!(read(&file), json!({ "providers": {} }));
    // Already gone: fine, and nothing is written.
    assert!(accounts.remove_pi_provider("omni-router").unwrap().is_empty());
}

#[test]
fn rejects_bad_input_without_touching_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let (accounts, file) = accounts(dir.path());
    accounts.save_pi_provider(omni(Some("sk-one-12345678"))).unwrap();
    let before = std::fs::read(&file).unwrap();

    let cases: Vec<(SavePiProvider, &str)> = vec![
        (omni(Some("sk-two")), "already exists"),
        (SavePiProvider { name: "openai".into(), ..omni(Some("k")) }, "built-in"),
        (SavePiProvider { name: "Omni Router".into(), ..omni(Some("k")) }, "lowercase"),
        (SavePiProvider { name: "new".into(), ..omni(None) }, "API key"),
        (SavePiProvider { name: "new".into(), ..omni(Some("  ")) }, "empty"),
        (
            SavePiProvider { name: "new".into(), base_url: "ftp://x.example".into(), ..omni(Some("k")) },
            "base URL",
        ),
        (
            SavePiProvider {
                name: "new".into(),
                base_url: "https://user:pw@x.example/v1".into(),
                ..omni(Some("k"))
            },
            "base URL",
        ),
        (
            SavePiProvider { name: "new".into(), api: "made-up".into(), ..omni(Some("k")) },
            "Unsupported API",
        ),
        (
            SavePiProvider { name: "new".into(), models: vec![model(" ")], ..omni(Some("k")) },
            "at least one model",
        ),
        (
            SavePiProvider {
                previous_name: Some("ghost".into()),
                ..omni(None)
            },
            "no longer exists",
        ),
    ];
    for (input, expected) in cases {
        let name = input.name.clone();
        let err = accounts.save_pi_provider(input).unwrap_err().to_string();
        assert!(err.contains(expected), "{name}: {err}");
    }
    assert_eq!(std::fs::read(&file).unwrap(), before);
}

#[test]
fn local_http_endpoints_are_allowed() {
    let dir = tempfile::tempdir().unwrap();
    let (accounts, _) = accounts(dir.path());
    let input = SavePiProvider {
        name: "lan".into(),
        base_url: "http://192.168.1.20:8080/v1".into(),
        ..omni(Some("k"))
    };
    let list = accounts.save_pi_provider(input).unwrap();
    assert_eq!(list[0].base_url, "http://192.168.1.20:8080/v1");
}

#[test]
fn an_unparseable_models_json_is_never_rewritten() {
    let dir = tempfile::tempdir().unwrap();
    let (accounts, file) = accounts(dir.path());
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "{ not json").unwrap();

    let err = accounts.list_pi_providers().unwrap_err().to_string();
    assert!(err.contains("isn't valid JSON"), "{err}");
    assert!(accounts.save_pi_provider(omni(Some("k"))).is_err());
    assert!(accounts.remove_pi_provider("omni").is_err());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "{ not json");
}

#[test]
fn writes_wait_for_another_zeron_writer() {
    let dir = tempfile::tempdir().unwrap();
    let (accounts, file) = accounts(dir.path());
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    let lock = file.with_file_name("models.json.zeron-lock");
    let _held = super::super::stores::FileLock::acquire(&lock).unwrap();

    let err = accounts
        .save_pi_provider(omni(Some("k")))
        .unwrap_err()
        .to_string();
    assert!(err.to_lowercase().contains("lock"), "{err}");
    assert!(!file.exists());
}

/// Serves `reply` to every request, recording each request's
/// `Authorization` header and path.
async fn stub(status: u16, reply: &'static str) -> (String, Arc<Mutex<Vec<(String, String)>>>) {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
    let task_seen = seen.clone();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut raw = Vec::new();
            let mut chunk = [0u8; 4096];
            while !raw.windows(4).any(|w| w == b"\r\n\r\n") {
                match socket.read(&mut chunk).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => raw.extend_from_slice(&chunk[..n]),
                }
            }
            let head = String::from_utf8_lossy(&raw).to_string();
            let path = head.split_whitespace().nth(1).unwrap_or("").to_string();
            let auth = head
                .lines()
                .find_map(|l| {
                    l.to_ascii_lowercase()
                        .starts_with("authorization:")
                        .then(|| l["authorization:".len()..].trim().to_string())
                })
                .unwrap_or_default();
            task_seen.lock().unwrap().push((path, auth));
            let response = format!(
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                reply.len()
            );
            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.shutdown().await;
        }
    });
    (base, seen)
}

#[tokio::test]
async fn discovers_openai_style_model_lists() {
    let dir = tempfile::tempdir().unwrap();
    let (accounts, _) = accounts(dir.path());
    let (base, seen) = stub(
        200,
        r#"{"object":"list","data":[{"id":"zeta"},{"id":"dashscope-intl/deepseek-v4.1-flash-1017"},{"id":"zeta"}]}"#,
    )
    .await;

    let ids = accounts
        .discover_pi_models(&format!("{base}/v1/"), Some("sk-fresh"), None)
        .await
        .unwrap();

    assert_eq!(ids, ["dashscope-intl/deepseek-v4.1-flash-1017", "zeta"]);
    assert_eq!(
        seen.lock().unwrap().as_slice(),
        [("/v1/models".to_string(), "Bearer sk-fresh".to_string())]
    );
}

#[tokio::test]
async fn discovery_reuses_a_stored_key_only_for_its_own_url() {
    let dir = tempfile::tempdir().unwrap();
    let (accounts, _) = accounts(dir.path());
    let (base, seen) = stub(200, r#"{"data":[{"id":"m"}]}"#).await;
    let input = SavePiProvider {
        base_url: format!("{base}/v1"),
        ..omni(Some("sk-stored-1234"))
    };
    accounts.save_pi_provider(input).unwrap();

    let ids = accounts
        .discover_pi_models(&format!("{base}/v1"), None, Some("omni"))
        .await
        .unwrap();
    assert_eq!(ids, ["m"]);
    assert_eq!(seen.lock().unwrap()[0].1, "Bearer sk-stored-1234");

    // A different URL never receives the stored key.
    let (elsewhere, elsewhere_seen) = stub(200, r#"{"data":[]}"#).await;
    let err = accounts
        .discover_pi_models(&format!("{elsewhere}/v1"), None, Some("omni"))
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("new URL"), "{err}");
    assert!(elsewhere_seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn discovery_explains_failures() {
    let dir = tempfile::tempdir().unwrap();
    let (accounts, _) = accounts(dir.path());

    let (base, _) = stub(401, r#"{"error":"bad key"}"#).await;
    let err = accounts.discover_pi_models(&base, Some("k"), None).await.unwrap_err();
    assert!(err.to_string().contains("rejected the API key"), "{err}");

    let (base, _) = stub(404, "not found").await;
    let err = accounts.discover_pi_models(&base, Some("k"), None).await.unwrap_err();
    assert!(err.to_string().contains("by hand"), "{err}");

    let (base, _) = stub(200, r#"{"unexpected":true}"#).await;
    let err = accounts.discover_pi_models(&base, Some("k"), None).await.unwrap_err();
    assert!(err.to_string().contains("OpenAI-style"), "{err}");

    let err = accounts.discover_pi_models(&base, None, None).await.unwrap_err();
    assert!(err.to_string().contains("Enter the API key"), "{err}");
}

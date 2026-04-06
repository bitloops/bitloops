use super::handlers::{
    handle_api_agents, handle_api_branches, handle_api_check_bundle_version, handle_api_checkpoint,
    handle_api_commits, handle_api_db_health, handle_api_fetch_bundle, handle_api_git_blob,
    handle_api_kpis, handle_api_not_found, handle_api_openapi, handle_api_repositories,
    handle_api_root, handle_api_users,
};
use super::{
    DASHBOARD_FALLBACK_INSTALL_HTML, DashboardState, ServeMode, content_type_for_path,
    has_bundle_index, request_path_looks_like_asset, resolve_bundle_file,
};
use crate::graphql::{
    global_graphql_handler, global_graphql_playground_handler, global_graphql_sdl_handler,
    global_graphql_ws_handler, slim_graphql_handler, slim_graphql_playground_handler,
    slim_graphql_sdl_handler, slim_graphql_ws_handler,
};
use axum::{
    Router,
    body::Body,
    extract::{Path as AxumPath, State},
    http::{HeaderValue, Method, StatusCode, header},
    response::Response,
    routing::{any, get, post},
};
use serde_json::Value;
use std::collections::HashMap;
use std::time::Instant;

const BUNDLE_UPDATE_PROMPT_SCRIPT: &str = r##"<script id="bitloops-bundle-update-prompt-script">
(function () {
  if (window.__bitloopsBundleUpdatePromptMounted) return;
  window.__bitloopsBundleUpdatePromptMounted = true;

  var messages = {
    manifest_fetch_failed: "Could not check for dashboard updates. Please retry.",
    bundle_download_failed: "Failed to download dashboard update. Please retry.",
    checksum_mismatch: "Downloaded update failed integrity checks. Please retry.",
    bundle_install_failed: "Dashboard update failed to install. Please retry.",
    no_compatible_version: "No compatible dashboard update is available.",
    internal: "Unexpected dashboard update error. Please retry."
  };

  function parseError(payload) {
    var code = payload && payload.error && payload.error.code ? payload.error.code : "internal";
    var message = payload && payload.error && payload.error.message ? payload.error.message : (messages[code] || messages.internal);
    return { code: code, message: message };
  }

  function requestJson(path, options) {
    return fetch(path, options || {}).then(function (res) {
      return res.json().catch(function () { return {}; }).then(function (payload) {
        if (!res.ok) throw parseError(payload);
        return payload;
      });
    });
  }

  function mountPrompt(state) {
    var overlay = document.createElement("div");
    overlay.id = "bitloops-update-prompt-overlay";
    overlay.style.position = "fixed";
    overlay.style.inset = "0";
    overlay.style.zIndex = "2147483647";
    overlay.style.background = "rgba(2, 6, 23, 0.64)";
    overlay.style.display = "grid";
    overlay.style.placeItems = "center";
    overlay.style.padding = "12px";

    var container = document.createElement("aside");
    container.id = "bitloops-update-prompt";
    container.setAttribute("role", "status");
    container.style.width = "min(680px, 92vw)";
    container.style.background = "#111827";
    container.style.color = "#f8fafc";
    container.style.border = "1px solid #1f2937";
    container.style.borderRadius = "14px";
    container.style.boxShadow = "0 18px 40px rgba(0,0,0,0.35)";
    container.style.padding = "24px";
    container.style.fontFamily = "-apple-system, BlinkMacSystemFont, \"Segoe UI\", sans-serif";
    container.style.fontSize = "15px";
    container.style.lineHeight = "1.5";

    var title = document.createElement("div");
    title.textContent = "Dashboard update available";
    title.style.fontWeight = "700";
    title.style.fontSize = "1.2rem";
    title.style.marginBottom = "10px";
    container.appendChild(title);

    var details = document.createElement("div");
    details.textContent = "Current: " + (state.currentVersion || "unknown") + " -> Latest: " + (state.latestApplicableVersion || "unknown");
    details.style.opacity = "0.9";
    details.style.marginBottom = "14px";
    container.appendChild(details);

    var status = document.createElement("div");
    status.style.minHeight = "22px";
    status.style.marginBottom = "14px";
    status.textContent = "Update dashboard bundle to apply the latest compatible UI.";
    container.appendChild(status);

    var actions = document.createElement("div");
    actions.style.display = "flex";
    actions.style.gap = "8px";
    actions.style.flexWrap = "wrap";

    var updateButton = document.createElement("button");
    updateButton.id = "bitloops-update-bundle-btn";
    updateButton.type = "button";
    updateButton.textContent = "Update dashboard bundle";
    updateButton.style.background = "#7404e4";
    updateButton.style.color = "#f8fafc";
    updateButton.style.border = "0";
    updateButton.style.padding = "8px 12px";
    updateButton.style.borderRadius = "8px";
    updateButton.style.cursor = "pointer";
    updateButton.style.fontWeight = "600";

    var dismissButton = document.createElement("button");
    dismissButton.type = "button";
    dismissButton.textContent = "Dismiss";
    dismissButton.style.background = "#334155";
    dismissButton.style.color = "#f8fafc";
    dismissButton.style.border = "0";
    dismissButton.style.padding = "8px 12px";
    dismissButton.style.borderRadius = "8px";
    dismissButton.style.cursor = "pointer";

    actions.appendChild(updateButton);
    actions.appendChild(dismissButton);
    container.appendChild(actions);

    function setInstalling(installing) {
      updateButton.disabled = installing;
      dismissButton.disabled = installing;
      updateButton.style.opacity = installing ? "0.6" : "1";
      dismissButton.style.opacity = installing ? "0.6" : "1";
      updateButton.style.cursor = installing ? "not-allowed" : "pointer";
      dismissButton.style.cursor = installing ? "not-allowed" : "pointer";
      updateButton.textContent = installing ? "Installing..." : "Update dashboard bundle";
    }

    updateButton.addEventListener("click", function () {
      setInstalling(true);
      status.style.color = "#f8fafc";
      status.textContent = "Installing dashboard update...";
      requestJson("/api/fetch_bundle", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: "{}"
      })
        .then(function () {
          status.style.color = "#86efac";
          status.textContent = "Update installed. Reloading...";
          setTimeout(function () { window.location.reload(); }, 250);
        })
        .catch(function (error) {
          status.style.color = "#fca5a5";
          status.textContent = error && error.message ? error.message : messages.internal;
          setInstalling(false);
        });
    });

    dismissButton.addEventListener("click", function () {
      overlay.remove();
    });

    overlay.appendChild(container);
    document.body.appendChild(overlay);
  }

  requestJson("/api/check_bundle_version")
    .then(function (payload) {
      if (!payload || payload.installAvailable !== true || payload.reason !== "update_available") return;
      mountPrompt(payload);
    })
    .catch(function () {
      return;
    });
})();
</script>"##;

pub(super) fn build_dashboard_router(state: DashboardState) -> Router {
    Router::new()
        .route("/api/", get(handle_api_root))
        .nest("/api", build_dashboard_api_router())
        .route(
            "/devql",
            post(slim_graphql_handler).get(slim_graphql_playground_handler),
        )
        .route("/devql/playground", get(slim_graphql_playground_handler))
        .route("/devql/sdl", get(slim_graphql_sdl_handler))
        .route("/devql/ws", get(slim_graphql_ws_handler))
        .route(
            "/devql/global",
            post(global_graphql_handler).get(global_graphql_playground_handler),
        )
        .route(
            "/devql/global/playground",
            get(global_graphql_playground_handler),
        )
        .route("/devql/global/sdl", get(global_graphql_sdl_handler))
        .route("/devql/global/ws", get(global_graphql_ws_handler))
        .route("/", any(handle_dashboard_root))
        .route("/{*path}", any(handle_dashboard_path))
        .with_state(state)
}

fn build_dashboard_api_router() -> Router<DashboardState> {
    Router::new()
        .route("/", get(handle_api_root))
        .route("/kpis", get(handle_api_kpis))
        .route("/commits", get(handle_api_commits))
        .route("/branches", get(handle_api_branches))
        .route("/repositories", get(handle_api_repositories))
        .route("/users", get(handle_api_users))
        .route("/agents", get(handle_api_agents))
        .route("/db/health", get(handle_api_db_health))
        .route(
            "/checkpoints/{repo_id}/{checkpoint_id}",
            get(handle_api_checkpoint),
        )
        .route("/blobs/{repo_id}/{blob_sha}", get(handle_api_git_blob))
        .route(
            "/check_bundle_version",
            get(handle_api_check_bundle_version),
        )
        .route("/fetch_bundle", post(handle_api_fetch_bundle))
        .route("/openapi.json", get(handle_api_openapi))
        .fallback(handle_api_not_found)
}

async fn handle_dashboard_root(State(state): State<DashboardState>, method: Method) -> Response {
    let started = Instant::now();
    let response = serve_dashboard_request(&state, "/", method.clone()).await;
    track_dashboard_page_event(&state, "/", &method, response.status(), started.elapsed());
    response
}

async fn handle_dashboard_path(
    State(state): State<DashboardState>,
    AxumPath(path): AxumPath<String>,
    method: Method,
) -> Response {
    let request_path = format!("/{path}");
    let started = Instant::now();
    let response = serve_dashboard_request(&state, &request_path, method.clone()).await;
    track_dashboard_page_event(
        &state,
        &request_path,
        &method,
        response.status(),
        started.elapsed(),
    );
    response
}

async fn serve_dashboard_request(
    state: &DashboardState,
    request_path: &str,
    method: Method,
) -> Response {
    let is_head = method == Method::HEAD;
    if method != Method::GET && !is_head {
        return response_with_bytes(
            StatusCode::METHOD_NOT_ALLOWED,
            "text/plain; charset=utf-8",
            b"method not allowed\n".to_vec(),
            false,
            None,
        );
    }

    let active_mode = match &state.mode {
        ServeMode::Bundle(bundle_dir) => ServeMode::Bundle(bundle_dir.clone()),
        ServeMode::HelloWorld => {
            if has_bundle_index(&state.bundle_dir) {
                ServeMode::Bundle(state.bundle_dir.clone())
            } else {
                ServeMode::HelloWorld
            }
        }
    };

    match active_mode {
        ServeMode::HelloWorld => response_with_bytes(
            StatusCode::OK,
            "text/html; charset=utf-8",
            DASHBOARD_FALLBACK_INSTALL_HTML.as_bytes().to_vec(),
            is_head,
            Some("no-store"),
        ),
        ServeMode::Bundle(bundle_dir) => {
            serve_bundle_request(&bundle_dir, request_path, is_head).await
        }
    }
}

async fn serve_bundle_request(
    bundle_dir: &std::path::Path,
    request_path: &str,
    is_head: bool,
) -> Response {
    let Ok(canonical_bundle_dir) = tokio::fs::canonicalize(bundle_dir).await else {
        return response_with_bytes(
            StatusCode::NOT_FOUND,
            "text/plain; charset=utf-8",
            b"Bundle not found.\n".to_vec(),
            is_head,
            None,
        );
    };

    if let Some(file_path) = resolve_bundle_file(bundle_dir, request_path)
        && let Some(bytes) = read_bundle_file_within_dir(&canonical_bundle_dir, &file_path).await
    {
        let content_type = content_type_for_path(&file_path);
        let body = maybe_inject_update_prompt(content_type, bytes);
        let cache_control = if content_type.starts_with("text/html") {
            Some("no-store")
        } else {
            None
        };
        return response_with_bytes(StatusCode::OK, content_type, body, is_head, cache_control);
    }

    if request_path_looks_like_asset(request_path) {
        return response_with_bytes(
            StatusCode::NOT_FOUND,
            "text/plain; charset=utf-8",
            b"Bundle asset not found.\n".to_vec(),
            is_head,
            None,
        );
    }

    let index_path = bundle_dir.join("index.html");
    if let Some(bytes) = read_bundle_file_within_dir(&canonical_bundle_dir, &index_path).await {
        let body = maybe_inject_update_prompt("text/html; charset=utf-8", bytes);
        return response_with_bytes(
            StatusCode::OK,
            "text/html; charset=utf-8",
            body,
            is_head,
            Some("no-store"),
        );
    }

    response_with_bytes(
        StatusCode::NOT_FOUND,
        "text/plain; charset=utf-8",
        b"Bundle not found.\n".to_vec(),
        is_head,
        None,
    )
}

fn maybe_inject_update_prompt(content_type: &str, body: Vec<u8>) -> Vec<u8> {
    if !content_type.starts_with("text/html") {
        return body;
    }

    let source = match String::from_utf8(body) {
        Ok(source) => source,
        Err(err) => return err.into_bytes(),
    };

    if source.contains("bitloops-bundle-update-prompt-script") {
        return source.into_bytes();
    }

    if let Some(index) = source.rfind("</body>") {
        let mut output = String::with_capacity(source.len() + BUNDLE_UPDATE_PROMPT_SCRIPT.len());
        output.push_str(&source[..index]);
        output.push_str(BUNDLE_UPDATE_PROMPT_SCRIPT);
        output.push_str(&source[index..]);
        return output.into_bytes();
    }

    let mut output = String::with_capacity(source.len() + BUNDLE_UPDATE_PROMPT_SCRIPT.len());
    output.push_str(&source);
    output.push_str(BUNDLE_UPDATE_PROMPT_SCRIPT);
    output.into_bytes()
}

async fn read_bundle_file_within_dir(
    canonical_bundle_dir: &std::path::Path,
    candidate_path: &std::path::Path,
) -> Option<Vec<u8>> {
    let canonical_candidate = tokio::fs::canonicalize(candidate_path).await.ok()?;
    if !canonical_candidate.starts_with(canonical_bundle_dir) {
        return None;
    }
    tokio::fs::read(canonical_candidate).await.ok()
}

fn response_with_bytes(
    status: StatusCode,
    content_type: &'static str,
    body: Vec<u8>,
    is_head: bool,
    cache_control: Option<&'static str>,
) -> Response {
    let mut response = if is_head {
        Response::new(Body::empty())
    } else {
        Response::new(Body::from(body))
    };
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    if let Some(cache_control) = cache_control {
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static(cache_control),
        );
    }
    response
}

fn track_dashboard_page_event(
    state: &DashboardState,
    request_path: &str,
    method: &Method,
    status: StatusCode,
    duration: std::time::Duration,
) {
    if *method != Method::GET && *method != Method::HEAD {
        return;
    }
    if request_path_looks_like_asset(request_path) {
        return;
    }

    let mut properties = HashMap::new();
    properties.insert(
        "http_method".to_string(),
        Value::String(method.as_str().to_string()),
    );
    properties.insert(
        "status_code_class".to_string(),
        Value::String(super::status_code_class(status).to_string()),
    );

    super::track_repo_action(
        &state.repo_root,
        crate::telemetry::analytics::ActionDescriptor {
            event: "bitloops dashboard page".to_string(),
            surface: "dashboard",
            properties,
        },
        status.is_success(),
        duration,
    );
}

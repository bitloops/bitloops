mod parsing;

use super::*;
use parsing::*;

#[allow(dead_code)]
pub(super) fn group_entry_point_artefacts_by_path(
    artefacts: &[CurrentCanonicalArtefactRecord],
) -> BTreeMap<String, Vec<LanguageEntryPointArtefact>> {
    let mut grouped: BTreeMap<String, Vec<LanguageEntryPointArtefact>> = BTreeMap::new();
    for artefact in artefacts {
        grouped
            .entry(artefact.path.clone())
            .or_default()
            .push(entry_point_artefact_from_current(artefact));
    }
    grouped
}

pub(super) fn entry_point_artefact_from_current(
    artefact: &CurrentCanonicalArtefactRecord,
) -> LanguageEntryPointArtefact {
    LanguageEntryPointArtefact {
        artefact_id: artefact.artefact_id.clone(),
        symbol_id: artefact.symbol_id.clone(),
        path: artefact.path.clone(),
        name: artefact_name(artefact),
        canonical_kind: artefact.canonical_kind.clone(),
        language_kind: artefact.language_kind.clone(),
        symbol_fqn: artefact.symbol_fqn.clone(),
        signature: artefact.signature.clone(),
        modifiers: parse_modifiers(&artefact.modifiers),
        start_line: artefact.start_line,
        end_line: artefact.end_line,
    }
}

#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn dependency_adjacency(
    edges: &[CurrentCanonicalEdgeRecord],
) -> BTreeMap<String, BTreeSet<String>> {
    let mut adjacency = BTreeMap::new();
    for edge in edges {
        insert_dependency_adjacency(&mut adjacency, edge);
    }
    adjacency
}

pub(super) fn insert_dependency_adjacency(
    adjacency: &mut BTreeMap<String, BTreeSet<String>>,
    edge: &CurrentCanonicalEdgeRecord,
) {
    let Some(to) = edge.to_artefact_id.as_ref() else {
        return;
    };
    adjacency
        .entry(edge.from_artefact_id.clone())
        .or_default()
        .insert(to.clone());
}

pub(super) fn infer_container_kind(candidate: &LanguageEntryPointCandidate) -> &'static str {
    match candidate.entry_kind.as_str() {
        "cargo_bin" if candidate.name == "xtask" => "dev_tool",
        "cargo_bin" | "npm_bin" | "python_console_script" | "rust_cli_dispatch" => "cli",
        "docusaurus_site" => "documentation_site",
        "vscode_extension" => "editor_extension",
        "npm_script" if candidate.name == "worker" => "worker",
        "npm_script" | "python_web_app" | "go_http_handler" | "next_route_handler" => "service",
        _ => "runtime",
    }
}

pub(super) fn is_deployable_config_candidate(candidate: &LanguageEntryPointCandidate) -> bool {
    matches!(
        candidate.entry_kind.as_str(),
        "cargo_bin"
            | "npm_bin"
            | "npm_script"
            | "python_console_script"
            | "docusaurus_site"
            | "vscode_extension"
    )
}

pub(super) fn deployment_binding_for_candidate<'a>(
    candidate: &LanguageEntryPointCandidate,
    deployment_by_path: &'a BTreeMap<String, DeploymentBinding>,
    deployment_by_root: &'a BTreeMap<String, DeploymentBinding>,
) -> Option<&'a DeploymentBinding> {
    deployment_by_path
        .get(&candidate.path)
        .or_else(|| deployment_by_root.get(&deployment_root_from_candidate(candidate)))
}

pub(super) fn deployment_root_from_candidate(candidate: &LanguageEntryPointCandidate) -> String {
    candidate
        .evidence
        .iter()
        .find(|path| {
            path.ends_with("Cargo.toml")
                || path.ends_with("package.json")
                || path.ends_with("pyproject.toml")
        })
        .and_then(|path| Path::new(path).parent())
        .map(normalise_repo_path)
        .filter(|path| !path.is_empty())
        .unwrap_or_else(|| {
            Path::new(&candidate.path)
                .parent()
                .map(normalise_repo_path)
                .filter(|path| !path.is_empty())
                .unwrap_or_else(|| ".".to_string())
        })
}

pub(super) fn path_in_root(path: &str, root: &str) -> bool {
    root == "."
        || path == root
        || path
            .strip_prefix(root)
            .is_some_and(|rest| rest.starts_with('/'))
}

pub(super) fn component_key_for_path(root: &str, path: &str) -> Option<String> {
    let relative = if root == "." {
        path
    } else {
        path.strip_prefix(root)?.trim_start_matches('/')
    };
    let parts = relative
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.is_empty() {
        return None;
    }
    if parts[0] == "src" && parts.len() > 1 {
        return Some(format!("src/{}", component_segment(parts[1])));
    }
    Some(component_segment(parts[0]))
}

pub(super) fn component_segment(path_segment: &str) -> String {
    Path::new(path_segment)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .unwrap_or(path_segment)
        .to_string()
}

pub(super) fn component_label(component_key: &str) -> String {
    component_key
        .rsplit('/')
        .next()
        .unwrap_or(component_key)
        .replace(['_', '-'], " ")
}

pub(super) fn component_path(root: &str, component_key: &str) -> String {
    if root == "." {
        component_key.to_string()
    } else {
        format!("{root}/{component_key}")
    }
}

pub(super) fn detect_config_entry_points(
    repo_root: &Path,
    files: &[CurrentCanonicalFileRecord],
    artefacts_by_path: &BTreeMap<String, Vec<LanguageEntryPointArtefact>>,
) -> Vec<LanguageEntryPointCandidate> {
    let file_paths = files
        .iter()
        .map(|file| file.path.clone())
        .collect::<BTreeSet<_>>();
    let mut candidates = Vec::new();
    for file in files {
        let Some(basename) = Path::new(&file.path)
            .file_name()
            .and_then(|name| name.to_str())
        else {
            continue;
        };
        match basename {
            "Cargo.toml" => detect_cargo_entry_points(
                repo_root,
                &file.path,
                &file_paths,
                artefacts_by_path,
                &mut candidates,
            ),
            "package.json" => detect_package_json_entry_points(
                repo_root,
                &file.path,
                &file_paths,
                artefacts_by_path,
                &mut candidates,
            ),
            "pyproject.toml" => detect_pyproject_entry_points(
                repo_root,
                &file.path,
                &file_paths,
                artefacts_by_path,
                &mut candidates,
            ),
            _ => {}
        }
    }
    candidates
}

pub(super) fn detect_cargo_entry_points(
    repo_root: &Path,
    config_path: &str,
    file_paths: &BTreeSet<String>,
    artefacts_by_path: &BTreeMap<String, Vec<LanguageEntryPointArtefact>>,
    candidates: &mut Vec<LanguageEntryPointCandidate>,
) {
    let Some(content) = read_repo_file(repo_root, config_path) else {
        return;
    };
    let Ok(document) = content.parse::<toml_edit::DocumentMut>() else {
        return;
    };
    let package_name = document
        .get("package")
        .and_then(|package| package.get("name"))
        .and_then(|name| name.as_str())
        .map(ToString::to_string);
    let Some(package_name) = package_name else {
        return;
    };

    let default_main = repo_relative_join(config_path, "src/main.rs");
    if file_paths.contains(&default_main) {
        candidates.push(config_candidate_for_path(
            &default_main,
            artefacts_by_path,
            "cargo_bin",
            &package_name,
            0.92,
            "Cargo package default binary target",
            vec![config_path.to_string(), default_main.clone()],
        ));
    }

    if let Some(bins) = document
        .get("bin")
        .and_then(|item| item.as_array_of_tables())
    {
        for bin in bins {
            let name = bin
                .get("name")
                .and_then(|value| value.as_str())
                .unwrap_or(package_name.as_str());
            let explicit_path = bin
                .get("path")
                .and_then(|value| value.as_str())
                .map(|path| repo_relative_join(config_path, path));
            let inferred_path = repo_relative_join(config_path, &format!("src/bin/{name}.rs"));
            let path = explicit_path
                .filter(|path| file_paths.contains(path))
                .or_else(|| file_paths.contains(&inferred_path).then_some(inferred_path));
            if let Some(path) = path {
                candidates.push(config_candidate_for_path(
                    &path,
                    artefacts_by_path,
                    "cargo_bin",
                    name,
                    0.94,
                    "Cargo explicit binary target",
                    vec![config_path.to_string(), path.clone()],
                ));
            }
        }
    }

    detect_rust_clap_entry_points(
        repo_root,
        config_path,
        &package_name,
        file_paths,
        candidates,
    );
    detect_http_route_entry_points(repo_root, config_path, file_paths, candidates);
}

pub(super) fn detect_package_json_entry_points(
    repo_root: &Path,
    config_path: &str,
    file_paths: &BTreeSet<String>,
    artefacts_by_path: &BTreeMap<String, Vec<LanguageEntryPointArtefact>>,
    candidates: &mut Vec<LanguageEntryPointCandidate>,
) {
    let Some(content) = read_repo_file(repo_root, config_path) else {
        return;
    };
    let Ok(document) = serde_json::from_str::<Value>(&content) else {
        return;
    };
    let package_name = document
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("package");
    let docusaurus_package = is_docusaurus_package(&document)
        || docusaurus_config_path(config_path, file_paths).is_some();
    let vscode_extension_package = is_vscode_extension_package(&document);

    if docusaurus_package {
        detect_docusaurus_package_entry_points(
            &document,
            config_path,
            package_name,
            file_paths,
            artefacts_by_path,
            candidates,
        );
    }

    if vscode_extension_package {
        detect_vscode_extension_entry_points(
            &document,
            config_path,
            package_name,
            file_paths,
            artefacts_by_path,
            candidates,
        );
    }

    match document.get("bin") {
        Some(Value::String(path)) => {
            let path = repo_relative_join(config_path, path);
            if file_paths.contains(&path) {
                candidates.push(config_candidate_for_path(
                    &path,
                    artefacts_by_path,
                    "npm_bin",
                    package_name,
                    0.90,
                    "package.json binary target",
                    vec![config_path.to_string(), path.clone()],
                ));
            }
        }
        Some(Value::Object(bins)) => {
            for (name, path) in bins {
                let Some(path) = path.as_str() else {
                    continue;
                };
                let path = repo_relative_join(config_path, path);
                if file_paths.contains(&path) {
                    candidates.push(config_candidate_for_path(
                        &path,
                        artefacts_by_path,
                        "npm_bin",
                        name,
                        0.90,
                        "package.json binary target",
                        vec![config_path.to_string(), path.clone()],
                    ));
                }
            }
        }
        _ => {}
    }

    if !docusaurus_package
        && !vscode_extension_package
        && let Some(Value::Object(scripts)) = document.get("scripts")
    {
        for script_name in ["start", "dev", "serve", "worker", "cli"] {
            let Some(script) = scripts.get(script_name).and_then(Value::as_str) else {
                continue;
            };
            if let Some(path) = script_entry_path(config_path, script, file_paths) {
                candidates.push(config_candidate_for_path(
                    &path,
                    artefacts_by_path,
                    "npm_script",
                    script_name,
                    0.76,
                    "package.json runtime script",
                    vec![config_path.to_string(), script.to_string(), path.clone()],
                ));
            }
        }
    }
}

pub(super) fn detect_docusaurus_package_entry_points(
    document: &Value,
    config_path: &str,
    package_name: &str,
    file_paths: &BTreeSet<String>,
    artefacts_by_path: &BTreeMap<String, Vec<LanguageEntryPointArtefact>>,
    candidates: &mut Vec<LanguageEntryPointCandidate>,
) {
    let entry_path = docusaurus_config_path(config_path, file_paths)
        .or_else(|| first_existing_package_path(config_path, file_paths, &["src/pages/index.tsx"]))
        .unwrap_or_else(|| config_path.to_string());
    let config_label = Path::new(&entry_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("docusaurus.config");
    candidates.push(config_candidate_for_path(
        &entry_path,
        artefacts_by_path,
        "docusaurus_site",
        package_name,
        0.92,
        "Docusaurus documentation site package",
        vec![
            config_path.to_string(),
            entry_path.clone(),
            "@docusaurus/core".to_string(),
            config_label.to_string(),
        ],
    ));

    let Some(Value::Object(scripts)) = document.get("scripts") else {
        return;
    };
    for script_name in ["start", "build", "serve", "deploy"] {
        let Some(script) = scripts.get(script_name).and_then(Value::as_str) else {
            continue;
        };
        candidates.push(config_candidate_for_path(
            &entry_path,
            artefacts_by_path,
            "docusaurus_script",
            &format!("{package_name} {script_name}"),
            0.78,
            "Docusaurus package lifecycle script",
            vec![config_path.to_string(), format!("{script_name}: {script}")],
        ));
    }
}

pub(super) fn detect_vscode_extension_entry_points(
    document: &Value,
    config_path: &str,
    package_name: &str,
    file_paths: &BTreeSet<String>,
    artefacts_by_path: &BTreeMap<String, Vec<LanguageEntryPointArtefact>>,
    candidates: &mut Vec<LanguageEntryPointCandidate>,
) {
    let main = document
        .get("main")
        .and_then(Value::as_str)
        .unwrap_or("./out/extension.js");
    let entry_path = package_main_source_path(config_path, main, file_paths)
        .or_else(|| first_existing_package_path(config_path, file_paths, &["src/extension.ts"]))
        .unwrap_or_else(|| config_path.to_string());
    let display_name = document
        .get("displayName")
        .and_then(Value::as_str)
        .unwrap_or(package_name);
    let extension_label = if display_name.to_lowercase().contains("extension") {
        display_name.to_string()
    } else {
        format!("{display_name} VS Code extension")
    };

    candidates.push(config_candidate_for_path(
        &entry_path,
        artefacts_by_path,
        "vscode_extension",
        &extension_label,
        0.93,
        "VS Code extension package",
        vec![
            config_path.to_string(),
            format!("main: {main}"),
            entry_path.clone(),
        ],
    ));

    let Some(commands) = document
        .get("contributes")
        .and_then(|contributes| contributes.get("commands"))
        .and_then(Value::as_array)
    else {
        return;
    };
    for command in commands {
        let Some(command_id) = command.get("command").and_then(Value::as_str) else {
            continue;
        };
        let title = command
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or(command_id);
        candidates.push(config_candidate_for_path(
            &entry_path,
            artefacts_by_path,
            "vscode_command",
            command_id,
            0.80,
            "VS Code contributed command",
            vec![
                config_path.to_string(),
                format!("command: {command_id}"),
                format!("title: {title}"),
            ],
        ));
    }
}

pub(super) fn detect_rust_clap_entry_points(
    repo_root: &Path,
    config_path: &str,
    package_name: &str,
    file_paths: &BTreeSet<String>,
    candidates: &mut Vec<LanguageEntryPointCandidate>,
) {
    let mut parsed_paths = BTreeSet::new();
    let cli_paths = [
        "src/cli.rs",
        "src/cli/mod.rs",
        "src/args.rs",
        "src/commands.rs",
        "src/commands/mod.rs",
    ];
    for cli_path in cli_paths {
        let path = repo_relative_join(config_path, cli_path);
        if !file_paths.contains(&path) {
            continue;
        }
        let Some(content) = read_repo_file(repo_root, &path) else {
            continue;
        };
        parsed_paths.insert(path.clone());
        for command in extract_clap_enum_commands(&content, "Commands") {
            candidates.push(LanguageEntryPointCandidate {
                path: path.clone(),
                artefact_id: None,
                symbol_id: None,
                name: format!("{package_name} {}", command.name),
                entry_kind: "rust_clap_command".to_string(),
                confidence: 0.82,
                reason: "Rust Clap subcommand".to_string(),
                evidence: vec![
                    config_path.to_string(),
                    path.clone(),
                    format!("command: {}", command.name),
                    command.about,
                ],
            });
        }
        break;
    }

    for path in rust_files_under_config_root(config_path, file_paths) {
        if parsed_paths.contains(&path) || path.contains("/tests/") || path.ends_with("_test.rs") {
            continue;
        }
        let Some(prefix) = clap_command_prefix_from_path(config_path, &path) else {
            continue;
        };
        let Some(content) = read_repo_file(repo_root, &path) else {
            continue;
        };
        for (_enum_name, commands) in extract_clap_subcommand_enums(&content) {
            for command in commands {
                candidates.push(LanguageEntryPointCandidate {
                    path: path.clone(),
                    artefact_id: None,
                    symbol_id: None,
                    name: format!("{package_name} {prefix} {}", command.name),
                    entry_kind: "rust_clap_command".to_string(),
                    confidence: 0.76,
                    reason: "Nested Rust Clap subcommand".to_string(),
                    evidence: vec![
                        config_path.to_string(),
                        path.clone(),
                        format!("command: {prefix} {}", command.name),
                        command.about,
                    ],
                });
            }
        }
    }
}

pub(super) fn detect_http_route_entry_points(
    repo_root: &Path,
    config_path: &str,
    file_paths: &BTreeSet<String>,
    candidates: &mut Vec<LanguageEntryPointCandidate>,
) {
    for router_path in http_route_files(config_path, file_paths) {
        let Some(content) = read_repo_file(repo_root, &router_path) else {
            continue;
        };
        for route in extract_axum_route_paths(&content)
            .into_iter()
            .filter(|route| is_primary_http_route(route))
        {
            candidates.push(LanguageEntryPointCandidate {
                path: router_path.clone(),
                artefact_id: None,
                symbol_id: None,
                name: route.clone(),
                entry_kind: "http_route".to_string(),
                confidence: 0.82,
                reason: "HTTP route endpoint".to_string(),
                evidence: vec![
                    config_path.to_string(),
                    router_path.clone(),
                    format!("route: {route}"),
                ],
            });
        }
    }
}

pub(super) fn is_docusaurus_package(document: &Value) -> bool {
    package_has_dependency(document, "@docusaurus/core")
        || package_has_dependency(document, "@docusaurus/preset-classic")
        || package_scripts_contain(document, "docusaurus")
}

pub(super) fn is_vscode_extension_package(document: &Value) -> bool {
    document
        .get("engines")
        .and_then(|engines| engines.get("vscode"))
        .and_then(Value::as_str)
        .is_some()
        || document
            .get("contributes")
            .and_then(|contributes| contributes.get("commands"))
            .is_some()
}

pub(super) fn package_has_dependency(document: &Value, dependency_name: &str) -> bool {
    ["dependencies", "devDependencies", "peerDependencies"]
        .into_iter()
        .any(|section| {
            document
                .get(section)
                .and_then(Value::as_object)
                .is_some_and(|dependencies| dependencies.contains_key(dependency_name))
        })
}

pub(super) fn package_scripts_contain(document: &Value, needle: &str) -> bool {
    document
        .get("scripts")
        .and_then(Value::as_object)
        .is_some_and(|scripts| {
            scripts
                .values()
                .filter_map(Value::as_str)
                .any(|script| script.contains(needle))
        })
}

pub(super) fn docusaurus_config_path(
    config_path: &str,
    file_paths: &BTreeSet<String>,
) -> Option<String> {
    first_existing_package_path(
        config_path,
        file_paths,
        &[
            "docusaurus.config.ts",
            "docusaurus.config.js",
            "docusaurus.config.mjs",
            "docusaurus.config.cjs",
        ],
    )
}

pub(super) fn package_main_source_path(
    config_path: &str,
    main: &str,
    file_paths: &BTreeSet<String>,
) -> Option<String> {
    let exact = repo_relative_join(config_path, main);
    if file_paths.contains(&exact) {
        return Some(exact);
    }

    let stem = Path::new(main)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())?;
    let candidates = [
        format!("src/{stem}.ts"),
        format!("src/{stem}.tsx"),
        format!("src/{stem}.js"),
        format!("src/{stem}.jsx"),
        format!("{stem}.ts"),
        format!("{stem}.js"),
    ];
    candidates
        .iter()
        .map(|path| repo_relative_join(config_path, path))
        .find(|path| file_paths.contains(path))
}

pub(super) fn first_existing_package_path(
    config_path: &str,
    file_paths: &BTreeSet<String>,
    candidates: &[&str],
) -> Option<String> {
    candidates
        .iter()
        .map(|path| repo_relative_join(config_path, path))
        .find(|path| file_paths.contains(path))
}

pub(super) fn rust_files_under_config_root(
    config_path: &str,
    file_paths: &BTreeSet<String>,
) -> Vec<String> {
    let root = config_root(config_path);
    file_paths
        .iter()
        .filter(|path| path.ends_with(".rs") && path_in_root(path, &root))
        .cloned()
        .collect()
}

pub(super) fn http_route_files(config_path: &str, file_paths: &BTreeSet<String>) -> Vec<String> {
    let conventional_paths = [
        "src/api/router.rs",
        "src/router.rs",
        "src/routes.rs",
        "src/http/router.rs",
        "src/server.rs",
        "src/main.rs",
    ];
    conventional_paths
        .into_iter()
        .map(|path| repo_relative_join(config_path, path))
        .filter(|path| file_paths.contains(path))
        .collect()
}

pub(super) fn clap_command_prefix_from_path(config_path: &str, path: &str) -> Option<String> {
    let root = config_root(config_path);
    let relative = relative_to_root(&root, path)?;
    let parts = relative
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let command_parts = match parts.as_slice() {
        ["src", "cli", rest @ ..] | ["src", "commands", rest @ ..] => rest,
        _ => return None,
    };
    if command_parts.is_empty() {
        return None;
    }

    let mut prefix_parts = command_parts
        .iter()
        .map(|part| part.to_string())
        .collect::<Vec<_>>();
    let last = prefix_parts.pop()?;
    let stem = Path::new(&last)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(&last);
    if !matches!(stem, "args" | "mod" | "commands") {
        prefix_parts.push(stem.to_string());
    }

    let prefix = prefix_parts
        .into_iter()
        .map(|part| variant_to_kebab(&part.replace('_', "-")))
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    (!prefix.is_empty()).then_some(prefix)
}

pub(super) fn config_root(config_path: &str) -> String {
    Path::new(config_path)
        .parent()
        .map(normalise_repo_path)
        .filter(|path| !path.is_empty())
        .unwrap_or_else(|| ".".to_string())
}

pub(super) fn relative_to_root<'a>(root: &str, path: &'a str) -> Option<&'a str> {
    if root == "." {
        Some(path)
    } else {
        path.strip_prefix(root)?.strip_prefix('/')
    }
}

pub(super) fn config_candidate_for_path(
    path: &str,
    artefacts_by_path: &BTreeMap<String, Vec<LanguageEntryPointArtefact>>,
    entry_kind: &str,
    name: &str,
    confidence: f64,
    reason: &str,
    evidence: Vec<String>,
) -> LanguageEntryPointCandidate {
    let artefact = artefacts_by_path.get(path).and_then(|artefacts| {
        artefacts
            .iter()
            .find(|artefact| artefact.name == "main")
            .or_else(|| {
                artefacts
                    .iter()
                    .find(|artefact| is_entry_candidate_artefact(artefact))
            })
            .or_else(|| artefacts.first())
    });
    LanguageEntryPointCandidate {
        path: path.to_string(),
        artefact_id: artefact.map(|artefact| artefact.artefact_id.clone()),
        symbol_id: artefact.map(|artefact| artefact.symbol_id.clone()),
        name: if name.is_empty() {
            artefact
                .map(|artefact| artefact.name.clone())
                .unwrap_or_else(|| path.to_string())
        } else {
            name.to_string()
        },
        entry_kind: entry_kind.to_string(),
        confidence,
        reason: reason.to_string(),
        evidence,
    }
}

pub(super) fn is_entry_candidate_artefact(artefact: &LanguageEntryPointArtefact) -> bool {
    matches!(
        artefact.canonical_kind.as_deref(),
        Some("function" | "method" | "callable" | "value" | "variable")
    )
}

pub(super) fn script_entry_path(
    config_path: &str,
    script: &str,
    file_paths: &BTreeSet<String>,
) -> Option<String> {
    for token in script.split_whitespace() {
        let token = token
            .trim_matches(|ch| matches!(ch, '"' | '\'' | '(' | ')' | ','))
            .trim_start_matches("./");
        if !matches!(
            Path::new(token).extension().and_then(|ext| ext.to_str()),
            Some("js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs")
        ) {
            continue;
        }
        let path = repo_relative_join(config_path, token);
        if file_paths.contains(&path) {
            return Some(path);
        }
    }

    [
        "src/index.ts",
        "src/index.js",
        "src/server.ts",
        "src/server.js",
        "server.ts",
        "server.js",
        "app.ts",
        "app.js",
    ]
    .into_iter()
    .map(|path| repo_relative_join(config_path, path))
    .find(|path| file_paths.contains(path))
}

pub(super) fn read_repo_file(repo_root: &Path, relative_path: &str) -> Option<String> {
    std::fs::read_to_string(repo_root.join(relative_path)).ok()
}

pub(super) fn repo_relative_join(config_path: &str, child_path: &str) -> String {
    let mut path = PathBuf::new();
    if let Some(parent) = Path::new(config_path).parent()
        && parent != Path::new("")
    {
        path.push(parent);
    }
    path.push(child_path.trim_start_matches("./"));
    normalise_repo_path(&path)
}

pub(super) fn normalise_repo_path(path: &Path) -> String {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                parts.pop();
            }
            std::path::Component::Normal(value) => {
                if let Some(value) = value.to_str() {
                    parts.push(value);
                }
            }
            _ => {}
        }
    }
    parts.join("/")
}

pub(super) fn artefact_display_name(artefact: &CurrentCanonicalArtefactRecord) -> String {
    artefact
        .symbol_fqn
        .as_deref()
        .map(last_symbol_segment)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| artefact.artefact_id.clone())
}

pub(super) fn artefact_name(artefact: &CurrentCanonicalArtefactRecord) -> String {
    artefact_display_name(artefact)
}

pub(super) fn last_symbol_segment(value: &str) -> String {
    value
        .rsplit([':', '.', '#'])
        .find(|segment| !segment.is_empty())
        .unwrap_or(value)
        .to_string()
}

pub(super) fn parse_modifiers(raw: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(raw).unwrap_or_default()
}

pub(super) fn string_field(row: &Value, key: &str) -> Option<String> {
    row.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

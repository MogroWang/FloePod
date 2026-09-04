//! 可选机构策略、审计导出和本地诊断包。
//!
//! 策略来自 `%PROGRAMDATA%\FloePod\organization-policy.json`，普通用户没有该
//! 文件时保持完全不受管理。策略不联网，也不读取暂存文件内容。

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager};
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

use crate::db;
use crate::operations;
use crate::settings::{self, Hotkeys, Pod, Settings};
use crate::state::AppState;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct OrganizationPolicy {
    pub organization_name: String,
    pub disable_move: bool,
    pub require_copy_default: bool,
    pub require_privacy_scan: bool,
    pub lock_rules: bool,
    pub disable_fulltext_index: bool,
    pub allowed_data_roots: Vec<String>,
    pub maximum_history_days: u32,
    pub mandatory_retention_days: u32,
    pub diagnostic_include_paths: bool,
    pub support_contact: String,
    pub managed_hotkeys: Option<Hotkeys>,
    pub managed_pods: Vec<Pod>,
}

impl Default for OrganizationPolicy {
    fn default() -> Self {
        Self {
            organization_name: String::new(),
            disable_move: false,
            require_copy_default: false,
            require_privacy_scan: false,
            lock_rules: false,
            disable_fulltext_index: false,
            allowed_data_roots: Vec::new(),
            maximum_history_days: 90,
            mandatory_retention_days: 0,
            diagnostic_include_paths: false,
            support_contact: String::new(),
            managed_hotkeys: None,
            managed_pods: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PolicyStatus {
    pub managed: bool,
    pub source: Option<String>,
    pub policy: OrganizationPolicy,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportedArtifact {
    pub path: String,
    pub records: usize,
}

fn policy_path() -> Option<PathBuf> {
    std::env::var_os("PROGRAMDATA")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .map(|path| path.join("FloePod").join("organization-policy.json"))
}

fn validate(policy: &OrganizationPolicy) -> Result<(), String> {
    if policy.organization_name.chars().count() > 120
        || policy.support_contact.chars().count() > 300
    {
        return Err("机构名称或支持联系方式过长".into());
    }
    if policy.maximum_history_days > 3_650 || policy.mandatory_retention_days > 3_650 {
        return Err("机构保留期限不能超过 10 年".into());
    }
    if policy.allowed_data_roots.len() > 32 {
        return Err("机构数据目录白名单不能超过 32 项".into());
    }
    if policy.managed_pods.len() > 32 {
        return Err("机构预设匣不能超过 32 个".into());
    }
    for root in &policy.allowed_data_roots {
        if !Path::new(root).is_absolute() {
            return Err(format!("机构数据目录必须是绝对路径: {root}"));
        }
    }
    Ok(())
}

pub fn apply_to_settings(settings: &mut Settings) -> Result<(), String> {
    let status = load()?;
    if !status.managed {
        return Ok(());
    }
    if let Some(hotkeys) = status.policy.managed_hotkeys {
        settings.hotkeys = hotkeys;
    }
    for managed in status.policy.managed_pods {
        if let Some(existing) = settings.pods.iter_mut().find(|pod| pod.id == managed.id) {
            *existing = managed;
        } else {
            settings.pods.push(managed);
        }
    }
    if status.policy.require_copy_default {
        for pod in &mut settings.pods {
            pod.drop_action = "copy".into();
        }
    }
    if status.policy.mandatory_retention_days > 0 {
        for pod in settings.pods.iter_mut().filter(|pod| pod.security.enabled) {
            pod.security.retention_days = match pod.security.retention_days {
                0 => status.policy.mandatory_retention_days,
                configured => configured.min(status.policy.mandatory_retention_days),
            };
        }
    }
    Ok(())
}

pub fn load() -> Result<PolicyStatus, String> {
    let Some(path) = policy_path() else {
        return Ok(PolicyStatus {
            managed: false,
            source: None,
            policy: OrganizationPolicy::default(),
        });
    };
    if !path.is_file() {
        return Ok(PolicyStatus {
            managed: false,
            source: Some(path.to_string_lossy().to_string()),
            policy: OrganizationPolicy::default(),
        });
    }
    let text = fs::read_to_string(&path)
        .map_err(|error| format!("无法读取机构策略 {}: {error}", path.display()))?;
    let policy: OrganizationPolicy =
        serde_json::from_str(&text).map_err(|error| format!("机构策略格式无效: {error}"))?;
    validate(&policy)?;
    Ok(PolicyStatus {
        managed: true,
        source: Some(path.to_string_lossy().to_string()),
        policy,
    })
}

pub fn enforce_pod(app: &AppHandle, pod: &Pod, changing_rules: bool) -> Result<(), String> {
    let status = load()?;
    if !status.managed {
        return Ok(());
    }
    if status.policy.lock_rules && changing_rules {
        return Err("机构策略已锁定规则匣设置".into());
    }
    if !status.policy.allowed_data_roots.is_empty() && !pod.staging_folder.trim().is_empty() {
        let folder = settings::resolve_path(Path::new(&pod.staging_folder))?;
        let allowed = status
            .policy
            .allowed_data_roots
            .iter()
            .filter_map(|root| settings::resolve_path(Path::new(root)).ok())
            .any(|root| settings::path_is_within(&folder, &root));
        if !allowed {
            return Err("暂存目录不在机构允许的数据目录白名单中".into());
        }
    }
    let _ = app;
    Ok(())
}

pub fn enforce_stage(action: &str) -> Result<(), String> {
    let status = load()?;
    if status.managed && status.policy.disable_move && action == "move" {
        return Err("机构策略禁止移动源文件；请使用复制".into());
    }
    Ok(())
}

pub fn enforce_export(mode: &str, privacy_checked: bool) -> Result<(), String> {
    let status = load()?;
    if !status.managed {
        return Ok(());
    }
    if status.policy.disable_move && mode == "move" {
        return Err("机构策略禁止移动导出；请使用复制或可信交接包".into());
    }
    if status.policy.require_privacy_scan && !privacy_checked {
        return Err("机构策略要求先使用“安全导出”或“可信交接包”完成本地隐私检查".into());
    }
    Ok(())
}

pub fn allow_index() -> Result<bool, String> {
    let status = load()?;
    Ok(!status.managed || !status.policy.disable_fulltext_index)
}

fn redact_path(value: &str) -> String {
    static PATH: OnceLock<Regex> = OnceLock::new();
    let path = PATH.get_or_init(|| {
        Regex::new(r#"(?i)(?:\\\\\?\\)?(?:[a-z]:\\|\\\\)[^\r\n\"';，。]*"#).unwrap()
    });
    path.replace_all(value, "<本地路径已隐藏>").to_string()
}

fn redact_value(value: &mut Value) {
    match value {
        Value::String(text) => *text = redact_path(text),
        Value::Array(values) => values.iter_mut().for_each(redact_value),
        Value::Object(values) => values.values_mut().for_each(redact_value),
        _ => {}
    }
}

fn redacted_operation(operation: &operations::OperationEntry, include_paths: bool) -> Value {
    let mut value = serde_json::to_value(operation).unwrap_or_else(|_| serde_json::json!({}));
    if !include_paths {
        if let Some(items) = value.get_mut("items").and_then(Value::as_array_mut) {
            for item in items {
                if let Some(item) = item.as_object_mut() {
                    item.insert("sourcePath".into(), Value::Null);
                    item.insert("targetPath".into(), Value::Null);
                }
            }
        }
        redact_value(&mut value);
    }
    value
}

pub fn export_audit(
    app: &AppHandle,
    destination: String,
    format: String,
) -> Result<ExportedArtifact, String> {
    let status = load()?;
    let hours = status.policy.maximum_history_days.max(1).saturating_mul(24);
    let operations = operations::list(app, hours, 500)?;
    let destination = PathBuf::from(destination);
    if !destination.is_absolute() {
        return Err("审计导出目标必须是绝对路径".into());
    }
    fs::create_dir_all(&destination).map_err(|error| error.to_string())?;
    let extension = if format == "csv" { "csv" } else { "json" };
    let path = destination.join(format!("FloePod-audit-{}.{}", db::now_ms(), extension));
    if extension == "json" {
        let records = operations
            .iter()
            .map(|operation| redacted_operation(operation, status.policy.diagnostic_include_paths))
            .collect::<Vec<_>>();
        fs::write(
            &path,
            serde_json::to_vec_pretty(&serde_json::json!({
                "organization": status.policy.organization_name,
                "exportedAt": db::now_ms(),
                "records": records,
                "localOnly": true,
            }))
            .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
    } else {
        let mut file = fs::File::create(&path).map_err(|error| error.to_string())?;
        writeln!(file, "时间,类型,状态,摘要,项目数").map_err(|error| error.to_string())?;
        for operation in &operations {
            let summary = if status.policy.diagnostic_include_paths {
                operation.summary.clone()
            } else {
                redact_path(&operation.summary)
            };
            writeln!(
                file,
                "{},\"{}\",\"{}\",\"{}\",{}",
                operation.created_at,
                operation.kind.replace('"', "\"\""),
                operation.status.replace('"', "\"\""),
                summary.replace('"', "\"\""),
                operation.items.len()
            )
            .map_err(|error| error.to_string())?;
        }
    }
    Ok(ExportedArtifact {
        path: path.to_string_lossy().to_string(),
        records: operations.len(),
    })
}

fn add_zip_text(writer: &mut ZipWriter<fs::File>, name: &str, value: &str) -> Result<(), String> {
    writer
        .start_file(
            name,
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated),
        )
        .map_err(|error| error.to_string())?;
    writer
        .write_all(value.as_bytes())
        .map_err(|error| error.to_string())
}

pub fn diagnostic_bundle(app: &AppHandle, destination: String) -> Result<ExportedArtifact, String> {
    let state = app.state::<AppState>();
    let status = load()?;
    let destination = PathBuf::from(destination);
    if !destination.is_absolute() {
        return Err("诊断包目标必须是绝对路径".into());
    }
    fs::create_dir_all(&destination).map_err(|error| error.to_string())?;
    let path = destination.join(format!("FloePod-diagnostics-{}.zip", db::now_ms()));
    let file = fs::File::create(&path).map_err(|error| error.to_string())?;
    let mut writer = ZipWriter::new(file);
    let mut settings_value = serde_json::to_value(crate::manager::current_settings(app))
        .map_err(|error| error.to_string())?;
    if !status.policy.diagnostic_include_paths {
        if let Some(settings) = settings_value.as_object_mut() {
            settings.insert("dataDir".into(), Value::String("<已隐藏>".into()));
        }
        if let Some(pods) = settings_value.get_mut("pods").and_then(Value::as_array_mut) {
            for pod in pods {
                if let Some(pod) = pod.as_object_mut() {
                    pod.insert("stagingFolder".into(), Value::String("<已隐藏>".into()));
                    if let Some(rules) = pod.get_mut("rules").and_then(Value::as_object_mut) {
                        rules.insert("sourceFolder".into(), Value::String("<已隐藏>".into()));
                    }
                }
            }
        }
        redact_value(&mut settings_value);
    }
    add_zip_text(
        &mut writer,
        "settings-redacted.json",
        &serde_json::to_string_pretty(&settings_value).map_err(|error| error.to_string())?,
    )?;
    let mut policy_value =
        serde_json::to_value(&status.policy).map_err(|error| error.to_string())?;
    if !status.policy.diagnostic_include_paths {
        redact_value(&mut policy_value);
    }
    add_zip_text(
        &mut writer,
        "organization-policy.json",
        &serde_json::to_string_pretty(&policy_value).map_err(|error| error.to_string())?,
    )?;
    let operations = operations::list(app, 24, 100)?;
    let operation_values = operations
        .iter()
        .map(|operation| redacted_operation(operation, status.policy.diagnostic_include_paths))
        .collect::<Vec<_>>();
    add_zip_text(
        &mut writer,
        "recent-operations-redacted.json",
        &serde_json::to_string_pretty(&operation_values).map_err(|error| error.to_string())?,
    )?;
    let debug_path = state.data_dir.join("debug.log");
    if let Ok(log) = fs::read_to_string(debug_path) {
        let lines = log.lines().rev().take(300).collect::<Vec<_>>();
        let log = lines.into_iter().rev().collect::<Vec<_>>().join("\n");
        add_zip_text(
            &mut writer,
            "debug-redacted.log",
            &if status.policy.diagnostic_include_paths {
                log
            } else {
                redact_path(&log)
            },
        )?;
    }
    add_zip_text(
        &mut writer,
        "system.txt",
        &format!(
            "FloePod {}\nOS {}\narch {}\nlocal-only diagnostic bundle\n",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH,
        ),
    )?;
    writer.finish().map_err(|error| error.to_string())?;
    Ok(ExportedArtifact {
        path: path.to_string_lossy().to_string(),
        records: operations.len(),
    })
}

pub fn export_settings(app: &AppHandle, destination: String) -> Result<ExportedArtifact, String> {
    let destination = PathBuf::from(destination);
    if !destination.is_absolute() {
        return Err("设置导出目标必须是绝对路径".into());
    }
    fs::create_dir_all(&destination).map_err(|error| error.to_string())?;
    let path = destination.join(format!("FloePod-settings-{}.json", db::now_ms()));
    let value = serde_json::to_vec_pretty(&crate::manager::current_settings(app))
        .map_err(|error| error.to_string())?;
    fs::write(&path, value).map_err(|error| error.to_string())?;
    Ok(ExportedArtifact {
        path: path.to_string_lossy().to_string(),
        records: 1,
    })
}

pub fn import_settings(
    app: &AppHandle,
    source: String,
) -> Result<crate::settings::Settings, String> {
    let path = PathBuf::from(source);
    if !path.is_absolute() || !path.is_file() {
        return Err("请选择有效的 FloePod 设置 JSON".into());
    }
    let value: Value = serde_json::from_slice(&fs::read(&path).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    let mut candidate: crate::settings::Settings =
        serde_json::from_value(value).map_err(|error| error.to_string())?;
    let state = app.state::<AppState>();
    candidate.version = env!("CARGO_PKG_VERSION").into();
    candidate.data_dir = state.data_dir.to_string_lossy().to_string();
    apply_to_settings(&mut candidate)?;
    settings::validate(&candidate, &candidate.data_dir)?;
    for pod in &candidate.pods {
        enforce_pod(app, pod, true)?;
        if pod.security.enabled {
            crate::security::ensure_efs(Path::new(&pod.staging_folder))?;
        }
    }
    settings::persist(&state.db.lock().unwrap(), &candidate)?;
    crate::manager::apply_settings(app, &candidate);
    crate::watcher::restart_all(app);
    crate::hotkeys::register(app, &candidate)?;
    let _ = app.emit(crate::events::SETTINGS_CHANGED, candidate.clone());
    let _ = app.emit(crate::events::PODS_CHANGED, ());
    Ok(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_policy_is_unmanaged_and_permissive() {
        let policy = OrganizationPolicy::default();
        assert!(!policy.disable_move);
        assert!(!policy.require_privacy_scan);
        assert!(validate(&policy).is_ok());
    }

    #[test]
    fn path_redaction_removes_drive_and_unc_paths() {
        let value = redact_path(
            "copy C:\\Users\\A\\My Secret\\secret.txt\nthen \\\\server\\share\\file.pdf",
        );
        assert!(!value.contains("secret.txt"));
        assert!(!value.contains("My Secret"));
        assert!(!value.contains("server"));
    }

    #[test]
    fn recursive_redaction_covers_policy_and_retry_metadata() {
        let mut value = serde_json::json!({
            "allowedDataRoots": [r"C:\Sensitive Root"],
            "retry": { "paths": [r"D:\Client Files\case.pdf"] }
        });
        redact_value(&mut value);
        let rendered = value.to_string();
        assert!(!rendered.contains("Sensitive Root"));
        assert!(!rendered.contains("case.pdf"));
    }
}

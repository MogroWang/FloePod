use super::model::AutoBlock;
use super::*;
use crate::db;
use rusqlite::Connection;
use std::path::Path;

fn conn() -> Connection {
    let c = Connection::open_in_memory().unwrap();
    db::migrate(&c).unwrap();
    c
}

#[test]
fn saving_general_settings_migrates_old_width_without_touching_the_original_on_failure() {
    let c = conn();
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("data").to_string_lossy().to_string();
    let old = serde_json::json!({"pods": [{"id": 1, "panelWidth": 380,
        "stagingFolder": tmp.path().join("stage").to_string_lossy()}]});
    db::kv_set(&c, KEY, &old.to_string()).unwrap();
    assert!(merge_persist(
        &c,
        serde_json::json!({"theme": "invalid"}),
        &data_dir,
        "test"
    )
    .is_err());
    assert_eq!(db::kv_get(&c, KEY).unwrap().unwrap(), old.to_string());
    let saved = merge_persist(&c, serde_json::json!({"theme": "dark"}), &data_dir, "test").unwrap();
    assert_eq!(saved.pods[0].panel_width, 410);
    assert_eq!(saved.theme, "dark");
    assert_eq!(
        load(&c, &data_dir, "test").unwrap().pods[0].panel_width,
        410
    );
}

#[test]
fn rejected_pod_update_leaves_database_and_accepted_memory_unchanged() {
    let c = conn();
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("data").to_string_lossy().to_string();
    let mut current = Settings {
        pods: vec![pod(1, &tmp.path().join("stage"))],
        ..Settings::default()
    };
    persist(&c, &current).unwrap();
    let original = serde_json::to_value(&current).unwrap();
    let stored = db::kv_get(&c, KEY).unwrap();
    let broken = Pod {
        panel_width: 380,
        ..current.pods[0].clone()
    };
    assert!(upsert_pod_from(&c, &mut current, &broken, &data_dir).is_err());
    assert_eq!(serde_json::to_value(&current).unwrap(), original);
    assert_eq!(db::kv_get(&c, KEY).unwrap(), stored);
    c.execute_batch("PRAGMA query_only=ON").unwrap();
    let valid = Pod {
        name: "磁盘拒绝写入".into(),
        ..current.pods[0].clone()
    };
    assert!(upsert_pod_from(&c, &mut current, &valid, &data_dir).is_err());
    assert_eq!(serde_json::to_value(&current).unwrap(), original);
}

fn pod(id: u64, folder: &Path) -> Pod {
    Pod {
        id,
        name: format!("匣 {id}"),
        edge: "left".into(),
        monitor: String::new(),
        offset: 0.5,
        staging_folder: folder.to_string_lossy().to_string(),
        opacity: 1.0,
        material: "acrylic".into(),
        panel_width: 440,
        hover_delay_ms: 120,
        drop_action: "ask".into(),
        enabled: true,
        ..Pod::default()
    }
}

#[test]
fn legacy_settings_migrates_to_pod() {
    let c = conn();
    db::kv_set(
        &c,
        KEY,
        r#"{"stagingFolder":"D:\\暂存","edge":"right","firstRunDone":true}"#,
    )
    .unwrap();
    let s = load(&c, "DATA", "0.4.0").unwrap();
    assert!(s.first_run_done);
    assert_eq!(s.pods.len(), 1);
    assert_eq!(s.pods[0].staging_folder, "D:\\暂存");
    assert_eq!(s.pods[0].edge, "right");
    assert_eq!(s.pods[0].id, 1);
}

#[test]
fn merge_ignores_pods_and_version() {
    let c = conn();
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("data").to_string_lossy().to_string();
    db::kv_set(&c, KEY, r#"{"theme":"system","pods":[]}"#).unwrap();
    let s = merge_persist(
        &c,
        serde_json::json!({"theme":"dark","pods":[{"id":99}],"version":"9.9"}),
        &data_dir,
        "0.4.0",
    )
    .unwrap();
    assert_eq!(s.theme, "dark");
    assert!(s.pods.is_empty());
    assert_eq!(s.version, "0.4.0");
}

#[test]
fn runtime_metadata_is_serialized_for_ipc_but_not_persisted() {
    let c = conn();
    let s = Settings {
        version: "1.0.0".into(),
        data_dir: r"C:\Users\tester\AppData\Roaming\FloePod".into(),
        ..Settings::default()
    };

    let wire = serde_json::to_value(&s).unwrap();
    assert_eq!(wire["version"], "1.0.0");
    assert_eq!(wire["dataDir"], s.data_dir);

    persist(&c, &s).unwrap();
    let stored: serde_json::Value =
        serde_json::from_str(&db::kv_get(&c, KEY).unwrap().unwrap()).unwrap();
    assert!(stored.get("version").is_none());
    assert!(stored.get("dataDir").is_none());
}

#[test]
fn existing_v040_settings_round_trip_without_losing_fields() {
    let c = conn();
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("data").to_string_lossy().to_string();
    let stage_one = tmp.path().join("stage-one").to_string_lossy().to_string();
    let stage_two = tmp.path().join("stage-two").to_string_lossy().to_string();
    let fixture = serde_json::json!({
        "theme": "dark",
        "firstRunDone": true,
        "autostart": true,
        "hotkeys": {
            "toggleBar": "Ctrl+Alt+KeyF",
            "collectClipboard": "Ctrl+Alt+KeyS",
            "openPanel": "Ctrl+Alt+KeyP"
        },
        "autoBlock": {
            "enabled": true,
            "apps": ["game.exe", "C:\\Games\\Racer.exe"]
        },
        "pods": [
            {
                "id": 7,
                "name": "左侧工作匣",
                "edge": "left",
                "monitor": "DISPLAY-A",
                "offset": 0.25,
                "stagingFolder": stage_one,
                "opacity": 0.72,
                "material": "plain",
                "panelMaterial": "acrylic",
                "panelOpacity": 0.9,
                "panelColor": "#aabbcc",
                "panelWidth": 512,
                "hoverDelayMs": 600,
                "autoHide": false,
                "autoHideDelayMs": 480,
                "stealth": true,
                "stealthDelayMs": 5000,
                "dropAction": "move",
                "enabled": true,
                "barWidth": 56,
                "cornerRadius": 12,
                "borderColor": "#80ffaa",
                "borderOpacity": 0.4
            },
            {
                "id": 19,
                "name": "离线便携匣",
                "edge": "bottom",
                "monitor": "",
                "offset": 1.0,
                "stagingFolder": stage_two,
                "opacity": 1.0,
                "material": "plain",
                "panelMaterial": "acrylic",
                "panelOpacity": 1.0,
                "panelColor": "",
                "panelWidth": 480,
                "hoverDelayMs": 0,
                "autoHide": true,
                "autoHideDelayMs": 320,
                "stealth": false,
                "stealthDelayMs": 3000,
                "dropAction": "shortcut",
                "enabled": false,
                "barWidth": 36,
                "cornerRadius": 0,
                "borderColor": "",
                "borderOpacity": 1.0
            }
        ]
    });
    db::kv_set(&c, KEY, &fixture.to_string()).unwrap();

    let loaded = load(&c, &data_dir, "1.0.0").unwrap();
    assert_eq!(loaded.pods.len(), 2);
    assert_eq!(loaded.pods[0].id, 7);
    assert_eq!(loaded.pods[0].monitor, "DISPLAY-A");
    assert_eq!(loaded.pods[0].panel_width, 512);
    assert_eq!(loaded.pods[0].bar_width, 56);
    assert_eq!(loaded.pods[0].corner_radius, 12);
    assert_eq!(loaded.pods[0].border_color, "#80ffaa");
    assert_eq!(loaded.pods[0].border_opacity, 0.4);
    assert_eq!(loaded.pods[0].panel_material, "acrylic");
    assert_eq!(loaded.pods[0].panel_opacity, 0.9);
    assert_eq!(loaded.pods[0].panel_color, "#aabbcc");
    assert!(!loaded.pods[0].auto_hide);
    assert_eq!(loaded.pods[0].auto_hide_delay_ms, 480);
    assert!(loaded.pods[0].stealth);
    assert_eq!(loaded.pods[0].stealth_delay_ms, 5000);
    assert_eq!(loaded.pods[1].drop_action, "shortcut");
    assert_eq!(loaded.pods[1].panel_material, "acrylic");
    assert!(loaded.pods[1].auto_hide);
    assert_eq!(loaded.pods[1].bar_width, 36);
    assert!(!loaded.pods[1].enabled);
    assert!(loaded.auto_block.enabled);
    assert_eq!(
        loaded.auto_block.apps,
        vec!["game.exe".to_string(), "C:\\Games\\Racer.exe".to_string()]
    );

    persist(&c, &loaded).unwrap();
    let stored: serde_json::Value =
        serde_json::from_str(&db::kv_get(&c, KEY).unwrap().unwrap()).unwrap();
    // 1.5.0 起辅助功能不再有总开关：旧 enabled 字段不得被写回存储。
    assert!(stored["accessibility"].get("enabled").is_none());
    assert_eq!(stored["accessibility"]["confirmDangerous"], true);
    assert_eq!(stored["pods"][0]["hoverOpen"], true);
    assert_eq!(stored["pods"][0]["barLength"], 190);
    assert_eq!(stored["pods"][0]["barColor"], "");
    assert_eq!(stored["pods"][0]["rules"]["enabled"], false);
    let mut legacy_view = stored;
    legacy_view.as_object_mut().unwrap().remove("accessibility");
    legacy_view["hotkeys"]
        .as_object_mut()
        .unwrap()
        .remove("lockSensitive");
    for pod in legacy_view["pods"].as_array_mut().unwrap() {
        pod.as_object_mut().unwrap().remove("hoverOpen");
        pod.as_object_mut().unwrap().remove("barLength");
        pod.as_object_mut().unwrap().remove("barColor");
        pod.as_object_mut().unwrap().remove("rules");
        pod.as_object_mut().unwrap().remove("security");
    }
    assert_eq!(legacy_view, fixture);
}

#[test]
fn panel_appearance_migrates_from_bar_fields() {
    let c = conn();
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("data").to_string_lossy().to_string();
    let stage = tmp.path().join("stage").to_string_lossy().to_string();
    // 1.2 版存储：pods 里没有 panelMaterial / panelOpacity。
    db::kv_set(
        &c,
        KEY,
        &serde_json::json!({
            "theme": "system",
            "pods": [{
                "id": 1, "name": "匣", "edge": "left",
                "stagingFolder": stage, "opacity": 0.8, "material": "acrylic"
            }]
        })
        .to_string(),
    )
    .unwrap();
    let s = load(&c, &data_dir, "1.3.0").unwrap();
    assert_eq!(s.pods[0].panel_material, "acrylic");
    assert!((s.pods[0].panel_opacity - 0.8).abs() < 1e-9);
    assert!(s.pods[0].auto_hide);
    assert_eq!(s.pods[0].auto_hide_delay_ms, 320);
    assert!(!s.auto_block.enabled);
    assert!(s.auto_block.apps.is_empty());

    // 重新持久化后字段已补齐，再次加载不再触发回填。
    persist(&c, &s).unwrap();
    let reloaded = load(&c, &data_dir, "1.3.0").unwrap();
    assert_eq!(reloaded.pods[0].panel_material, "acrylic");
    assert!((reloaded.pods[0].panel_opacity - 0.8).abs() < 1e-9);
}

#[test]
fn legacy_panel_width_migrates_into_new_range() {
    let c = conn();
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("data").to_string_lossy().to_string();
    let stage = tmp.path().join("stage").to_string_lossy().to_string();
    // 1.4 及更早允许 300-520：存量下限值 300 在 1.5.0 收紧后必须收敛，
    // 否则严格校验会让保存路径全部报错。
    db::kv_set(
        &c,
        KEY,
        &serde_json::json!({
            "theme": "system",
            "pods": [{
                "id": 1, "name": "匣", "edge": "left", "stagingFolder": stage,
                "opacity": 1.0, "material": "plain", "panelWidth": 300
            }]
        })
        .to_string(),
    )
    .unwrap();
    let s = load(&c, &data_dir, "1.5.0").unwrap();
    assert_eq!(s.pods[0].panel_width, 410);
    assert!(validate(&s, &data_dir).is_ok());
}

#[test]
fn panel_and_auto_block_fields_validate() {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("data").to_string_lossy().to_string();

    for good in ["acrylic", "plain"] {
        let mut candidate = pod(1, &tmp.path().join("stage"));
        candidate.material = good.into();
        candidate.panel_material = good.into();
        let settings = Settings {
            pods: vec![candidate],
            ..Settings::default()
        };
        assert!(validate(&settings, &data_dir).is_ok(), "材质 {good} 应有效");
    }
    for bad in ["blur", "mica", "glass", "frosted", "MICA", ""] {
        let mut candidate = pod(1, &tmp.path().join("stage"));
        candidate.material = bad.into();
        let settings = Settings {
            pods: vec![candidate],
            ..Settings::default()
        };
        assert!(validate(&settings, &data_dir).is_err(), "材质 {bad} 应无效");
    }

    let mut broken = pod(1, &tmp.path().join("stage"));
    broken.panel_opacity = 0.05;
    let settings = Settings {
        pods: vec![broken],
        ..Settings::default()
    };
    assert!(validate(&settings, &data_dir).is_err());

    let mut broken = pod(1, &tmp.path().join("stage"));
    broken.auto_hide_delay_ms = 5001;
    let settings = Settings {
        pods: vec![broken],
        ..Settings::default()
    };
    assert!(validate(&settings, &data_dir).is_err());

    let blocked = Settings {
        auto_block: AutoBlock {
            enabled: true,
            apps: (0..65).map(|i| format!("app{i}.exe")).collect(),
        },
        ..Default::default()
    };
    assert!(validate(&blocked, &data_dir).is_err());

    let blank = Settings {
        auto_block: AutoBlock {
            enabled: true,
            apps: vec!["   ".into()],
        },
        ..Default::default()
    };
    assert!(validate(&blank, &data_dir).is_err());
}

#[test]
fn merge_accepts_auto_block_patch() {
    let c = conn();
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("data").to_string_lossy().to_string();
    db::kv_set(&c, KEY, r#"{"theme":"system","pods":[]}"#).unwrap();
    let s = merge_persist(
        &c,
        serde_json::json!({"autoBlock":{"enabled":true,"apps":["Game.exe"]}}),
        &data_dir,
        "1.3.0",
    )
    .unwrap();
    assert!(s.auto_block.enabled);
    assert_eq!(s.auto_block.apps, vec!["Game.exe".to_string()]);
}

#[test]
fn legacy_materials_migrate_to_acrylic() {
    let c = conn();
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("data").to_string_lossy().to_string();
    // 两个匣必须使用不同的暂存文件夹，否则 validate 会因文件夹重复而失败。
    let stage = tmp.path().join("stage").to_string_lossy().to_string();
    let stage_two = tmp.path().join("stage-two").to_string_lossy().to_string();
    db::kv_set(
        &c,
        KEY,
        &serde_json::json!({
            "theme": "system",
            "pods": [
                {
                    "id": 1, "name": "匣", "edge": "left", "stagingFolder": stage,
                    "material": "blur", "panelMaterial": "blur"
                },
                {
                    "id": 2, "name": "匣二", "edge": "right", "stagingFolder": stage_two,
                    "material": "mica", "panelMaterial": "mica"
                }
            ]
        })
        .to_string(),
    )
    .unwrap();
    let s = load(&c, &data_dir, "1.3.0").unwrap();
    // 边缘浮动条材质已废弃（固定普通）；浮动面板的 blur 与 mica 都迁移为亚克力。
    assert_eq!(s.pods[0].material, "plain");
    assert_eq!(s.pods[0].panel_material, "acrylic");
    assert_eq!(s.pods[1].panel_material, "acrylic");
    // 迁移结果合法，可直接通过校验。
    assert!(validate(&s, &data_dir).is_ok());
}

#[test]
fn pod_upsert_delete() {
    let c = conn();
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("data").to_string_lossy().to_string();
    let pod = pod(1, &tmp.path().join("stage"));
    let mut current = load(&c, &data_dir, "0.4.0").unwrap();
    upsert_pod_from(&c, &mut current, &pod, &data_dir).unwrap();
    assert_eq!(load(&c, &data_dir, "0.4.0").unwrap().pods.len(), 1);
    delete_pod(&c, 1, &data_dir, "0.4.0").unwrap();
    assert!(load(&c, &data_dir, "0.4.0").unwrap().pods.is_empty());
}

#[test]
fn validate_rejects_data_dir_overlap_both_directions() {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("data");
    let mut settings = Settings::default();
    settings.pods.push(pod(1, &data_dir.join("stage")));
    assert!(validate(&settings, &data_dir.to_string_lossy()).is_err());

    settings.pods[0] = pod(1, tmp.path());
    assert!(validate(&settings, &data_dir.to_string_lossy()).is_err());
}

#[test]
fn validate_rejects_equal_or_nested_pod_folders() {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("data");
    let stage = tmp.path().join("stage");
    let settings = Settings {
        pods: vec![pod(1, &stage), pod(2, &stage.join("nested"))],
        ..Settings::default()
    };
    assert!(validate(&settings, &data_dir.to_string_lossy()).is_err());
}

#[test]
fn validate_allows_legacy_disabled_pod_without_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let mut disabled = pod(1, &tmp.path().join("unused"));
    disabled.enabled = false;
    disabled.staging_folder.clear();
    let settings = Settings {
        pods: vec![disabled],
        ..Settings::default()
    };
    assert!(validate(&settings, &tmp.path().join("data").to_string_lossy()).is_ok());
}

#[test]
fn pod_ids_are_monotonic_even_before_persisting_pod() {
    let c = conn();
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("data").to_string_lossy().to_string();
    let current = load(&c, &data_dir, "0.4.0").unwrap();
    assert_eq!(next_pod_id_from(&c, &current).unwrap(), 1);
    assert_eq!(next_pod_id_from(&c, &current).unwrap(), 2);
}

#[test]
fn pod_deserializes_without_id() {
    // 前端创建匣时不携带 id（由后端分配），缺省字段应成功反序列化
    let v = serde_json::json!({
        "name": "我的匣",
        "edge": "right",
        "stagingFolder": "D:\\暂存",
    });
    let pod: Pod = serde_json::from_value(v).unwrap();
    assert_eq!(pod.id, 0);
    assert_eq!(pod.name, "我的匣");
    assert_eq!(pod.edge, "right");
    assert_eq!(pod.staging_folder, "D:\\暂存");
    assert_eq!(pod.offset, 0.5); // 来自 Default
    assert!(pod.enabled);
}

#[test]
fn bar_appearance_fields_validate() {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("data").to_string_lossy().to_string();

    let mut pod = pod(1, &tmp.path().join("stage"));
    pod.bar_width = 44;
    pod.corner_radius = 22;
    pod.border_color = String::new();
    pod.border_opacity = 1.0;
    let settings = Settings {
        pods: vec![pod.clone()],
        ..Settings::default()
    };
    assert!(validate(&settings, &data_dir).is_ok());

    for (field, value) in [
        ("bar_width", serde_json::json!(27u64)),
        ("bar_width", serde_json::json!(97u64)),
        ("bar_length", serde_json::json!(99u64)),
        ("bar_length", serde_json::json!(501u64)),
        ("panel_width", serde_json::json!(409u64)),
        ("panel_width", serde_json::json!(601u64)),
        ("corner_radius", serde_json::json!(65u64)),
        ("border_opacity", serde_json::json!(1.5)),
        ("border_opacity", serde_json::json!(-0.1)),
    ] {
        let mut broken = pod.clone();
        match field {
            "bar_width" => broken.bar_width = value.as_u64().unwrap() as u32,
            "bar_length" => broken.bar_length = value.as_u64().unwrap() as u32,
            "panel_width" => broken.panel_width = value.as_u64().unwrap() as u32,
            "corner_radius" => broken.corner_radius = value.as_u64().unwrap() as u32,
            "border_opacity" => broken.border_opacity = value.as_f64().unwrap(),
            _ => unreachable!(),
        }
        let settings = Settings {
            pods: vec![broken],
            ..Settings::default()
        };
        assert!(
            validate(&settings, &data_dir).is_err(),
            "{field} = {value} 应无效"
        );
    }

    for good in ["#fff", "#80FFaa", "#11223344", ""] {
        let mut candidate = pod.clone();
        candidate.border_color = good.into();
        let settings = Settings {
            pods: vec![candidate],
            ..Settings::default()
        };
        assert!(
            validate(&settings, &data_dir).is_ok(),
            "边框色 {good} 应有效"
        );
    }
    for bad in ["red", "#12", "#12345", "#1234567", "80ffaa", "#80 ffaa"] {
        let mut candidate = pod.clone();
        candidate.border_color = bad.into();
        let settings = Settings {
            pods: vec![candidate],
            ..Settings::default()
        };
        assert!(
            validate(&settings, &data_dir).is_err(),
            "边框色 {bad} 应无效"
        );
    }

    // 浮动条填充色：合法 hex 与空串（跟随主题）均可，非法值被拒绝。
    for good in ["#fff", "#80FFaa", "#11223344", ""] {
        let mut candidate = pod.clone();
        candidate.bar_color = good.into();
        let settings = Settings {
            pods: vec![candidate],
            ..Settings::default()
        };
        assert!(
            validate(&settings, &data_dir).is_ok(),
            "浮动条填充色 {good} 应有效"
        );
    }
    let mut candidate = pod.clone();
    candidate.bar_color = "red".into();
    let settings = Settings {
        pods: vec![candidate],
        ..Settings::default()
    };
    assert!(validate(&settings, &data_dir).is_err(), "非法填充色应无效");
}

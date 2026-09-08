use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Hotkeys {
    #[serde(default = "d_toggle_bar")]
    pub toggle_bar: String,
    #[serde(default = "d_collect_clipboard")]
    pub collect_clipboard: String,
    #[serde(default = "d_open_panel")]
    pub open_panel: String,
    #[serde(default = "d_lock_sensitive")]
    pub lock_sensitive: String,
}

fn d_toggle_bar() -> String {
    "Alt+Shift+F".into()
}
fn d_collect_clipboard() -> String {
    "Alt+Shift+S".into()
}
fn d_open_panel() -> String {
    "Alt+Shift+P".into()
}
fn d_lock_sensitive() -> String {
    "Alt+Shift+L".into()
}

impl Hotkeys {
    pub fn with_defaults() -> Self {
        Self {
            toggle_bar: d_toggle_bar(),
            collect_clipboard: d_collect_clipboard(),
            open_panel: d_open_panel(),
            lock_sensitive: d_lock_sensitive(),
        }
    }
}

/// 自动屏蔽：配置的应用位于前台时暂时隐藏全部匣，离开前台后自动恢复。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AutoBlock {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub apps: Vec<String>,
}

/// 免费的「辅助功能」设置：提高可读性、提供非拖拽替代并减少认知负担。
/// 各选项相互独立、直接生效；1.5.0 起不再有「启用辅助功能」总开关
/// （旧配置中的 enabled 字段被 serde 忽略）。
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default, rename_all = "camelCase")]
pub struct Accessibility {
    /// WebView 内容缩放，范围 1.0 - 2.0。
    pub scale: f64,
    pub high_contrast: bool,
    pub reduce_transparency: bool,
    pub reduce_motion: bool,
    pub simple_language: bool,
    pub confirm_dangerous: bool,
    pub send_to_menu: bool,
}

/// 单个匣的本地规则。规则只做可解释的过滤、命名、归档和校验，不执行任意脚本。
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default, rename_all = "camelCase")]
pub struct PodRules {
    pub enabled: bool,
    pub template: String,
    pub allowed_extensions: Vec<String>,
    pub name_contains: String,
    pub source_folder: String,
    pub max_size_mb: u64,
    /// 支持 {name}、{stem}、{ext}、{date}、{year}、{month}、{day}。
    pub rename_pattern: String,
    /// 支持日期令牌；必须是相对目录且不能含 ..。
    pub subfolder_pattern: String,
    /// allow / reject
    #[schemars(extend("enum" = ["allow","reject"]))]
    pub duplicate_policy: String,
    pub checksum_sidecar: bool,
    pub expire_days: u32,
    pub remove_after_export: bool,
}

impl Default for PodRules {
    fn default() -> Self {
        Self {
            enabled: false,
            template: "manual".into(),
            allowed_extensions: Vec::new(),
            name_contains: String::new(),
            source_folder: String::new(),
            max_size_mb: 0,
            rename_pattern: "{name}".into(),
            subfolder_pattern: String::new(),
            duplicate_policy: "allow".into(),
            checksum_sidecar: false,
            expire_days: 0,
            remove_after_export: false,
        }
    }
}

/// 敏感匣使用 Windows EFS 加密目录，并以 Windows Hello 控制应用内解锁。
/// 不保存自制密码或密钥；不支持 EFS 的卷会拒绝启用而不是假装已加密。
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default, rename_all = "camelCase")]
pub struct PodSecurity {
    pub enabled: bool,
    pub require_windows_hello: bool,
    pub auto_lock_minutes: u32,
    pub retention_days: u32,
    pub cleanup_after_export: bool,
    pub suppress_thumbnails: bool,
    pub suppress_index: bool,
}

impl Default for PodSecurity {
    fn default() -> Self {
        Self {
            enabled: false,
            require_windows_hello: true,
            auto_lock_minutes: 10,
            retention_days: 0,
            cleanup_after_export: false,
            suppress_thumbnails: true,
            suppress_index: true,
        }
    }
}

impl Default for Accessibility {
    fn default() -> Self {
        Self {
            scale: 1.0,
            high_contrast: false,
            reduce_transparency: false,
            reduce_motion: false,
            simple_language: false,
            confirm_dangerous: true,
            send_to_menu: false,
        }
    }
}

/// 窗口材质取值:亚克力 / 普通无材质。
/// 早期版本的「模糊」与亚克力观感一致、云母因系统材质失焦不可靠已移除,
/// 存量配置统一迁移(见 normalize_materials)。
pub const MATERIALS: [&str; 2] = ["acrylic", "plain"];

pub(super) fn valid_material(material: &str) -> bool {
    MATERIALS.contains(&material)
}

/// 一个「匣」：贴在屏幕边缘的独立暂存点，拥有自己的保存文件夹与外观。
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default, rename_all = "camelCase")]
pub struct Pod {
    pub id: u64,
    pub name: String,
    /// top / right / bottom / left
    #[schemars(extend("enum" = ["top","right","bottom","left"]))]
    pub edge: String,
    /// 显示器名；空串 = 主显示器
    pub monitor: String,
    /// 沿边缘的位置 0.0 - 1.0
    pub offset: f64,
    pub staging_folder: String,
    pub opacity: f64,
    /// 边缘浮动条材质；1.3.0 起废弃，normalize 时固定为 "plain"（普通半透明）。
    /// 保留字段仅为兼容旧存储与 IPC 结构，应用层不再读取。
    #[schemars(extend("enum" = ["plain","acrylic"]))]
    pub material: String,
    /// 浮动面板材质；与边缘浮动条材质独立设置。
    #[schemars(extend("enum" = ["plain","acrylic"]))]
    pub panel_material: String,
    /// 浮动面板不透明度 0.1 - 1.0；与边缘浮动条不透明度独立设置。
    pub panel_opacity: f64,
    /// 浮动面板填充色（#RGB/#RRGGBB/#RRGGBBAA）；空串 = 跟随主题表面色。
    pub panel_color: String,
    pub panel_width: u32,
    pub hover_delay_ms: u64,
    /// 是否允许悬停自动弹出；关闭后仍可单击或用键盘打开浮动面板。
    pub hover_open: bool,
    /// 鼠标离开后自动隐藏浮动面板（淡出；重新悬停时淡入）。
    pub auto_hide: bool,
    /// 鼠标离开后到自动隐藏的延迟（毫秒）。
    pub auto_hide_delay_ms: u64,
    /// 隐匿模式：无交互超过延迟后边缘浮动条淡化隐去，指针靠近时再淡入。
    pub stealth: bool,
    /// 隐匿模式下无交互到淡化隐去的延迟（毫秒）。
    pub stealth_delay_ms: u64,
    #[schemars(extend("enum" = ["ask","copy","move","shortcut"]))]
    pub drop_action: String,
    pub enabled: bool,
    /// 边缘浮动条短边宽度（CSS 逻辑像素）；浮动面板宽度由 panel_width 控制。
    pub bar_width: u32,
    /// 边缘浮动条长度，即沿屏幕边缘方向的长边（CSS 逻辑像素）。
    pub bar_length: u32,
    /// 边缘浮动条填充色（#RGB/#RRGGBB/#RRGGBBAA）；空串 = 跟随主题表面色。
    pub bar_color: String,
    /// 边缘浮动条外角圆角半径；0 为直角，CSS 会自动把超过半宽的值收敛。
    pub corner_radius: u32,
    /// 边缘浮动条边框颜色（#RGB/#RRGGBB/#RRGGBBAA）；空串 = 跟随主题。
    pub border_color: String,
    /// 边缘浮动条边框不透明度 0.0 - 1.0，作用于 border_color 或主题默认边框色。
    pub border_opacity: f64,
    #[serde(default)]
    pub rules: PodRules,
    #[serde(default)]
    pub security: PodSecurity,
}

impl Default for Pod {
    fn default() -> Self {
        Pod {
            id: 0,
            name: "新匣".into(),
            edge: "left".into(),
            monitor: String::new(),
            offset: 0.5,
            staging_folder: String::new(),
            opacity: 1.0,
            material: "plain".into(),
            panel_material: "acrylic".into(),
            panel_opacity: 1.0,
            panel_color: String::new(),
            panel_width: 440,
            hover_delay_ms: 120,
            hover_open: true,
            auto_hide: true,
            auto_hide_delay_ms: 320,
            stealth: false,
            stealth_delay_ms: 3000,
            drop_action: "ask".into(),
            enabled: true,
            bar_width: 44,
            bar_length: 190,
            bar_color: String::new(),
            corner_radius: 22,
            border_color: String::new(),
            border_opacity: 1.0,
            rules: PodRules::default(),
            security: PodSecurity::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(default = "d_theme")]
    #[schemars(extend("enum" = ["system","light","dark"]))]
    pub theme: String,
    #[serde(default)]
    pub first_run_done: bool,
    #[serde(default)]
    pub autostart: bool,
    #[serde(default = "Hotkeys::with_defaults")]
    pub hotkeys: Hotkeys,
    #[serde(default)]
    pub auto_block: AutoBlock,
    #[serde(default)]
    pub accessibility: Accessibility,
    #[serde(default)]
    pub pods: Vec<Pod>,
    /// 只读：由应用在读取时注入并返回前端，但不接受数据库中的旧值。
    /// `persist` 会在写库前显式剔除这两个运行时字段。
    #[serde(skip_deserializing, default)]
    pub version: String,
    #[serde(skip_deserializing, default)]
    pub data_dir: String,
}

fn d_theme() -> String {
    "system".into()
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: d_theme(),
            first_run_done: false,
            autostart: false,
            hotkeys: Hotkeys::with_defaults(),
            auto_block: AutoBlock::default(),
            accessibility: Accessibility::default(),
            pods: Vec::new(),
            version: String::new(),
            data_dir: String::new(),
        }
    }
}

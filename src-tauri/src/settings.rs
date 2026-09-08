//! 设置领域：模型/默认值、兼容迁移、验证与持久化各自拥有单一职责。
mod migration;
mod model;
mod patch;
mod store;
mod validation;

pub use model::{Hotkeys, Pod, Settings};
pub use patch::{PodPatch, SettingsPatch};
#[cfg(test)]
pub use store::KEY;
pub use store::{delete_pod, load, merge_persist, next_pod_id_from, persist, upsert_pod_from};
pub use validation::{validate, validate_pod_for_io};

#[cfg(test)]
mod tests;

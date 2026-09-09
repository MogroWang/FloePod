#[path = "contract/build.rs"]
mod contract;

fn main() {
    contract::generate();
    tauri_build::build()
}

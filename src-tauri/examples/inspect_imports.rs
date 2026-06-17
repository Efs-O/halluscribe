use app_lib::{scanner, settings};
use std::env;
use std::path::PathBuf;

fn main() {
    let home = env::var("USERPROFILE")
        .or_else(|_| env::var("HOME"))
        .unwrap();
    let dir = PathBuf::from(home).join(".halluscribe");
    let settings = settings::load_settings(&dir);
    println!("chatgpt_import_path={}", settings.chatgpt_import_path);
    println!("claudeai_import_path={}", settings.claudeai_import_path);
    println!("gemini_import_path={}", settings.gemini_import_path);
    let targets = scanner::scan_chat_imports(&settings, u64::MAX);
    println!("targets={}", targets.len());
    for target in targets {
        println!("path={}", target.path.display());
    }
}

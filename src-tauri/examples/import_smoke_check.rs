// HalluScribe - dev example: scan configured imports and report what each reader parses.
use app_lib::{
    readers,
    scanner::{self, ScanTargetKind},
    settings::HalluScribeSettings,
};
use std::env;

fn main() {
    let settings = HalluScribeSettings {
        chatgpt_import_path: env::var("CHATGPT_IMPORT_PATH").unwrap_or_default(),
        claudeai_import_path: env::var("CLAUDEAI_IMPORT_PATH").unwrap_or_default(),
        gemini_import_path: env::var("GEMINI_IMPORT_PATH").unwrap_or_default(),
        ..HalluScribeSettings::default()
    };

    let targets = scanner::scan_chat_imports(&settings, u64::MAX);
    println!("=== HalluScribe Import Smoke Check ===");
    println!("targets: {}", targets.len());

    if targets.is_empty() {
        println!("No import targets found. Set CHATGPT_IMPORT_PATH / CLAUDEAI_IMPORT_PATH / GEMINI_IMPORT_PATH.");
        return;
    }

    for target in targets {
        let provider = match &target.kind {
            ScanTargetKind::Import(provider) => provider.display_name(),
            ScanTargetKind::Coding(_) => "coding",
        };
        println!("\n--- {} ---", provider);
        println!("source: {}", target.path.display());

        match readers::read_target(&target, None) {
            Ok(sessions) => {
                println!("parsed sessions: {}", sessions.len());
                for session in sessions.iter().take(3) {
                    println!(
                        "- {} | {} | fill={}{:.1}% | messages={}",
                        session.id,
                        session.title,
                        if session.fill_estimated { "~" } else { "" },
                        session.fill_pct,
                        session.messages.len()
                    );
                }
                if sessions.len() > 3 {
                    println!("... {} more", sessions.len() - 3);
                }
            }
            Err(error) => {
                println!("parse error: {error}");
            }
        }
    }
}

//! Probes the routing preferences Copilot offers its `auto` Model, Fusion among them.
//!
//! Prints every Model `models.list` reports with its policy state and billing multiplier, then the
//! `auto` metadata the account was offered: its default preference and each preference's ID, kind
//! (`auto` or `fusion`), Fusion tier, availability and billing. For each preference ID named, it
//! also opens an `auto` Session staged on that preference, runs one short Turn, and prints the
//! model, Auto-tier and Fusion events the Turn produced beside what `session.model.getCurrent`
//! answered after it. `default` names no preference, leaving the routing to Copilot:
//!
//! ```sh
//! cargo run --example copilot_auto_tier_probe                    # catalog and preferences only
//! cargo run --example copilot_auto_tier_probe -- default balance <fusion-id>
//! ```
//!
//! An SDK Session receives none of the ExP assignments the interactive CLI fetches, so the flags
//! that gate dynamic Auto tiers are the host's to supply: `PROBE_FEATURE_FLAGS` names feature flags
//! to enable on every Session (comma-separated, e.g. `HYDRAFUSION`), and `PROBE_EXPERIMENTAL=1`
//! enables experimental mode, and `PROBE_EXP_ASSIGNMENTS` names a JSON file holding ExP assignments
//! in the shape the CLI caches them (the `response` of an entry in its `exp-cache.json`).
//! `SURU_COPILOT_PATH` names the CLI (default `copilot`).
#[path = "support/copilot_cli.rs"]
mod copilot_cli;

use std::{collections::HashMap, time::Duration};

use github_copilot_sdk::{
    CapiSessionOptions, MessageOptions, session_events::AutoTier, types::SessionConfig,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let tiers: Vec<String> = std::env::args().skip(1).collect();
    let client = copilot_cli::start().await?;
    let list = client.rpc().models().list().await?;
    for model in &list.models {
        println!(
            "[PROBE] model {:<32} policy={} multiplier={}",
            model.id,
            serde_json::to_string(&model.policy.as_ref().map(|policy| &policy.state))?,
            serde_json::to_string(&model.billing.as_ref().and_then(|b| b.multiplier))?,
        );
    }
    match &list.auto {
        Some(auto) => println!("[PROBE] auto {}", serde_json::to_string_pretty(auto)?),
        None => println!("[PROBE] auto metadata absent from models.list"),
    }
    // A Session lists Models under its own auth and integration context, which may carry Auto
    // metadata the client-level list does not.
    let session = client.create_session(auto_session()).await?;
    match session.rpc().model().list().await?.auto {
        Some(auto) => println!(
            "[PROBE] session auto {}",
            serde_json::to_string_pretty(&auto)?
        ),
        None => println!("[PROBE] auto metadata absent from session.model.list"),
    }
    println!(
        "[PROBE] session current {}",
        serde_json::to_string(&session.rpc().model().get_current().await?)?
    );

    for tier in &tiers {
        println!("[PROBE] tier {tier}");
        let mut config = auto_session();
        if tier != "default" {
            let auto_tier: AutoTier = serde_json::from_value(tier.as_str().into())?;
            config = config.with_capi(CapiSessionOptions::new().with_auto_tier(auto_tier));
        }
        let session = match client.create_session(config).await {
            Ok(session) => session,
            Err(error) => {
                println!("[PROBE]   create Err({error})");
                continue;
            }
        };
        let mut events = session.subscribe();
        let watcher = tokio::spawn(async move {
            while let Ok(event) = events.recv().await {
                let kind = event.event_type.as_str();
                if kind.contains("fusion")
                    || kind.contains("auto_tier")
                    || kind == "session.model_change"
                    || kind == "session.error"
                {
                    println!("[PROBE]   {kind} {}", event.data);
                } else if kind == "assistant.usage" {
                    println!(
                        "[PROBE]   assistant.usage model={} cost={}",
                        event.data["model"], event.data["cost"]
                    );
                }
            }
        });
        let reply = session
            .send_and_wait(MessageOptions::new(
                "Reply with exactly the single word OK and nothing else.",
            ))
            .await;
        println!(
            "[PROBE]   turn -> {}",
            match &reply {
                Ok(_) => "Ok".to_owned(),
                Err(error) => format!("Err({error})"),
            }
        );
        // Ephemeral reports may trail the idle that ends the Turn.
        tokio::time::sleep(Duration::from_secs(2)).await;
        match session.rpc().model().get_current().await {
            Ok(current) => println!("[PROBE]   current {}", serde_json::to_string(&current)?),
            Err(error) => println!("[PROBE]   current Err({error})"),
        }
        watcher.abort();
    }
    client.stop().await?;
    Ok(())
}

/// An `auto` Session carrying whatever flags the environment names.
fn auto_session() -> SessionConfig {
    let mut config = SessionConfig::default().with_model("auto");
    if let Ok(flags) = std::env::var("PROBE_FEATURE_FLAGS") {
        let flags: HashMap<String, bool> = flags
            .split(',')
            .filter(|flag| !flag.is_empty())
            .map(|flag| (flag.trim().to_owned(), true))
            .collect();
        config = config.with_feature_flags(flags);
    }
    if let Some(path) = std::env::var_os("PROBE_EXP_ASSIGNMENTS") {
        let assignments = std::fs::read_to_string(path).expect("PROBE_EXP_ASSIGNMENTS is readable");
        config = config.with_exp_assignments(
            serde_json::from_str(&assignments)
                .expect("PROBE_EXP_ASSIGNMENTS holds ExP assignments"),
        );
    }
    if std::env::var("PROBE_EXPERIMENTAL").is_ok_and(|value| value == "1") {
        config = config.with_enable_experimental_mode(true);
    }
    config
}

use std::{collections::HashMap, fs, path::Path, sync::Arc};

use chrono::{Datelike, TimeZone};
use chrono_tz::Tz;
use poise::CreateReply;
use serenity::all::{ChannelId, ClientBuilder, GatewayIntents, RoleId};
use tokio::time::{Instant, sleep_until};
use tracing::{error, info};

use crate::state::Store;

mod env;
mod state;
mod words;

type Ctx<'a> = poise::Context<'a, AppState, anyhow::Error>;

const SAMPLE_ALPHA: f64 = 2.0;

#[derive(Clone)]
pub struct AppState {
    store: Arc<Store>,
    timezone: Tz,
    channel_id: ChannelId,
    role_id: RoleId,
    dictionary: Arc<HashMap<String, f64>>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env().add_directive("info".parse()?),
        )
        .init();
    info!("Starting bot");

    let cfg = env::EnvCfg::from_env()?;

    let state_path = Path::new(&cfg.state_path);
    if let Some(parent) = state_path.parent() {
        fs::create_dir_all(parent).ok();
    }
    let store = Arc::new(Store::new(cfg.state_path));
    store.load()?;

    let timezone: Tz = cfg.timezone.parse().expect("Invalid IANA timezone");

    let channel_id = ChannelId::new(cfg.announce_channel_id);
    let role_id = RoleId::new(cfg.role_id);

    let dictionary = Arc::new(words::build_dict(cfg.dict_path)?);

    let state = AppState {
        store,
        timezone,
        channel_id,
        role_id,
        dictionary,
    };

    let intents = GatewayIntents::GUILDS | GatewayIntents::GUILD_MESSAGES;

    let framework = poise::Framework::<AppState, anyhow::Error>::builder()
        .options(poise::FrameworkOptions {
            commands: vec![suggest(), history()],
            ..Default::default()
        })
        .setup(move |ctx, _ready, framework| {
            let state = state.clone();
            Box::pin(async move {
                poise::builtins::register_globally(ctx, &framework.options().commands).await?;
                spawn_scheduler(ctx.clone(), state.clone());
                Ok(state)
            })
        })
        .build();

    let mut client = ClientBuilder::new(cfg.discord_bot_token, intents)
        .framework(framework)
        .await?;

    if let Err(e) = client.start().await {
        error!("Client error: {:?}", e);
    }

    Ok(())
}

fn spawn_scheduler(ctx: poise::serenity_prelude::Context, state: AppState) {
    tokio::spawn(async move {
        loop {
            if let Err(e) = run_once(&ctx, &state).await {
                error!("scheduler error: {:?}", e);
            }
            let now_utc = chrono::Utc::now();
            let now_local = state.timezone.from_utc_datetime(&now_utc.naive_utc());
            let next_local = {
                let mut d = now_local.date_naive();
                // if already past 23:55 today, use tomorrow
                let today_target = state
                    .timezone
                    .with_ymd_and_hms(d.year(), d.month(), d.day(), 23, 55, 0)
                    .unwrap();
                if now_local >= today_target {
                    d = d.succ_opt().unwrap();
                }
                state
                    .timezone
                    .with_ymd_and_hms(d.year(), d.month(), d.day(), 23, 55, 0)
                    .unwrap()
            };
            let dur = (next_local - now_local).to_std().unwrap_or_default();
            sleep_until(Instant::now() + dur).await;
        }
    });
}

/// # Errors
/// Will error if get weighted fails
pub async fn run_once(ctx: &serenity::all::Context, state: &AppState) -> anyhow::Result<()> {
    use chrono::{Duration, Utc};

    let now_local = state.timezone.from_utc_datetime(&Utc::now().naive_utc());
    let target = now_local.date_naive() + Duration::days(1);

    // 1) Reuse
    if let Some((existing, sug)) = state.store.with(|s| {
        s.history.iter().rev().find(|e| e.date == target).map(|e| {
            (
                e.word.clone(),
                e.suggested_by.map(|user| format!("<@{user}>")),
            )
        })
    }) {
        return announce(ctx, state, target, &existing, sug.as_deref()).await;
    }

    // 2) Queue first: drop invalid/used; pick first valid
    let picked_from_queue: Option<(String, serenity::all::UserId)> = loop {
        let maybe = state.store.with_mut(|s| s.queue.pop_front());
        let Some((user_id, word)) = maybe else {
            break None;
        };
        let w = word.to_lowercase();
        let is_valid = state.dictionary.contains_key(&w);
        let is_used = state.store.with(|s| s.used.contains(&w));
        if is_valid && !is_used {
            state
                .store
                .with_mut(|s| s.mark_used(target, w.clone(), Some(user_id)));
            break Some((w, user_id));
        }
    };

    // 3) Fallback weighted pick
    let (word, mention): (String, Option<String>) = if let Some((w, uid)) = picked_from_queue {
        (w, Some(format!("<@{}>", uid.get())))
    } else {
        let used = state.store.with(|s| s.used.clone());
        let Some(w) = words::pick_weighted(&state.dictionary, Some(&used), Some(SAMPLE_ALPHA))
            .map(str::to_owned)
        else {
            error!("Failed to get next word");
            return Err(anyhow::Error::msg("Failed to get next word"));
        };
        state
            .store
            .with_mut(|s| s.mark_used(target, w.clone(), None));
        (w, None)
    };

    announce(ctx, state, target, &word, mention.as_deref()).await
}

async fn announce(
    ctx: &serenity::all::Context,
    state: &AppState,
    date: chrono::NaiveDate,
    word: &str,
    suggested_by: Option<&str>,
) -> anyhow::Result<()> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(m) = suggested_by {
        parts.push(format!("Suggested by {m}"));
    }

    let suffix = if parts.is_empty() {
        String::new()
    } else {
        parts.join("\n").to_string()
    };
    let msg = format!(
        "<@&{}>\nTomorrow’s Wordle starter ({date}) is: ||`{word}`||\n{suffix}",
        state.role_id
    );
    state.channel_id.say(&ctx.http, msg).await?;
    Ok(())
}

#[poise::command(slash_command)]
pub async fn suggest(
    ctx: Ctx<'_>,
    #[description = "5-letter word"] word: String,
) -> anyhow::Result<()> {
    let uid = ctx.author().id;
    let w = word.trim().to_lowercase();

    if w.len() != 5 || !w.chars().all(|c| c.is_ascii_lowercase()) {
        ctx.send(
            CreateReply::default()
                .content("Rejected: provide a 5-letter a–z word.")
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    }
    if !ctx.data().dictionary.contains_key(&w) {
        ctx.send(
            CreateReply::default()
                .content("Rejected: not in dictionary.")
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    }
    if ctx.data().store.with(|s| s.used.contains(&w)) {
        ctx.send(
            CreateReply::default()
                .content("Rejected: already used previously.")
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    }
    if ctx
        .data()
        .store
        .with(|s| s.queue.iter().any(|(_, q)| q == &w))
    {
        ctx.send(
            CreateReply::default()
                .content("Already queued.")
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    }

    ctx.data()
        .store
        .with_mut(|s| s.queue.push_back((uid, w.clone())));

    ctx.send(
        CreateReply::default()
            .content(format!("Queued `{w}`."))
            .ephemeral(true),
    )
    .await?;
    Ok(())
}

#[poise::command(slash_command)]
pub async fn history(
    ctx: Ctx<'_>,
    #[description = "How many days back (default 14)"] days_back: Option<i64>,
) -> anyhow::Result<()> {
    let days = days_back.unwrap_or(14).clamp(1, 3650);

    // compute cutoff in the bot's configured timezone
    let now_local = ctx
        .data()
        .timezone
        .from_utc_datetime(&chrono::Utc::now().naive_utc());
    let cutoff = now_local.date_naive() - chrono::Duration::days(days);

    // collect entries >= cutoff
    let mut rows = ctx.data().store.with(|s| {
        s.history
            .iter()
            .filter(|e| e.date >= cutoff)
            .cloned()
            .collect::<Vec<_>>()
    });

    // newest first; tie-break by word
    rows.sort_by(|a, b| b.date.cmp(&a.date).then_with(|| a.word.cmp(&b.word)));

    if rows.is_empty() {
        ctx.say(format!("No entries in the last {days} days."))
            .await?;
        return Ok(());
    }

    // build a message under ~1900 chars
    let mut out = String::with_capacity(1024);
    out.push_str(format!("Previous starting words for the last {days} days\n").as_str());
    for e in rows {
        let line = format!("{} — `{}`\n", e.date, e.word);
        if out.len() + line.len() > 1900 {
            break;
        }
        out.push_str(&line);
    }

    ctx.say(out).await?;
    Ok(())
}

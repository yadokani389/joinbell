use poise::{
    CreateReply,
    serenity_prelude::{self as serenity, RoleId},
};
use serde::Deserialize;
use serenity::Mentionable;
use tokio::time::{Duration, sleep};

type Error = Box<dyn std::error::Error + Send + Sync>;

const DELETE_DELAY_SECONDS: u64 = 3600;
const PARTICIPATION_EMOJI: &str = "✋";

#[derive(Debug, Deserialize)]
struct RecruitConfig {
    game_title: String,
    required_players: u64,
    mention_role: RoleId,
    #[serde(default)]
    notify_on_reaction: bool,
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    dotenvy::dotenv().ok();
    let token = std::env::var("DISCORD_TOKEN").expect("Missing DISCORD_TOKEN");

    let intents = serenity::GatewayIntents::non_privileged();

    let framework = poise::Framework::builder()
        .options(poise::FrameworkOptions {
            commands: vec![recruit()],
            event_handler: |ctx, event, framework, data| {
                Box::pin(event_handler(ctx, event, framework, data))
            },
            ..Default::default()
        })
        .setup(|ctx, _ready, framework| {
            Box::pin(async move {
                poise::builtins::register_globally(ctx, &framework.options().commands).await?;
                Ok(())
            })
        })
        .build();

    let mut client = serenity::ClientBuilder::new(token, intents)
        .framework(framework)
        .await?;

    client.start().await?;
    Ok(())
}

async fn event_handler(
    ctx: &serenity::Context,
    event: &serenity::FullEvent,
    _framework: poise::FrameworkContext<'_, (), Error>,
    _data: &(),
) -> Result<(), Error> {
    if let serenity::FullEvent::ReactionAdd { add_reaction } = event {
        handle_reaction_add(ctx, add_reaction).await?;
    }
    Ok(())
}

#[poise::command(slash_command, guild_only)]
async fn recruit(
    ctx: poise::Context<'_, (), Error>,
    #[description = "募集するゲーム名"] game_title: String,
    #[description = "開始に必要な人数"] required_players: u64,
    #[description = "開始時にメンションするロール"] mention_role: serenity::Role,
    #[description = "リアクション追加時に参加通知を送るかどうか"] notify_on_reaction: Option<bool>,
) -> Result<(), Error> {
    if required_players == 0 {
        ctx.say("required_players は 1 以上を指定してください。")
            .await?;
        return Ok(());
    }

    let notify_on_reaction = notify_on_reaction.unwrap_or(false);

    let role_mention = mention_role.id;
    let message_body = format!(
        r#"
このメッセージに :raised_hand: をつけると {game_title} に参加できます
人数が揃ったら開始通知が送られます
```toml
game_title = {game_title:?}
required_players = {required_players}
mention_role = {role_mention}
notify_on_reaction = {notify_on_reaction}
```"#,
    );

    let message = ctx.channel_id().say(ctx.http(), message_body).await?;
    let reaction_type = participation_reaction_type();
    message.react(ctx.http(), reaction_type).await?;

    ctx.send(
        CreateReply::default()
            .content("募集メッセージを投稿しました")
            .ephemeral(true),
    )
    .await?;
    Ok(())
}

async fn handle_reaction_add(
    ctx: &serenity::Context,
    reaction: &serenity::Reaction,
) -> Result<(), Error> {
    if !is_participation_reaction(&reaction.emoji) {
        return Ok(());
    }

    if reaction.user_id == Some(ctx.cache.current_user().id) {
        return Ok(());
    }

    let message = reaction.message(ctx.http.clone()).await?;
    if message.author.id != ctx.cache.current_user().id {
        return Ok(());
    }

    if !message.content.contains("```toml") {
        return Ok(());
    }

    let config = match parse_recruit_config(&message.content) {
        Ok(config) => config,
        Err(err) => {
            eprintln!("Failed to parse config: {err}");
            send_error_message(ctx, reaction).await?;
            return Ok(());
        }
    };

    if config.required_players == 0 {
        send_error_message(ctx, reaction).await?;
        return Ok(());
    }

    let current_count = message
        .reactions
        .iter()
        .find(|item| item.reaction_type == reaction.emoji)
        .map(|item| item.count - if item.me { 1 } else { 0 })
        .unwrap_or(0);

    if config.notify_on_reaction {
        send_participation_notification(ctx, &config, reaction).await?;
    }

    if current_count == config.required_players {
        send_start_notification(
            ctx,
            &config,
            &message,
            config.mention_role,
            reaction.emoji.clone(),
        )
        .await?;
    }

    Ok(())
}

fn participation_reaction_type() -> serenity::ReactionType {
    serenity::ReactionType::Unicode(PARTICIPATION_EMOJI.to_string())
}

fn is_participation_reaction(reaction: &serenity::ReactionType) -> bool {
    matches!(reaction, serenity::ReactionType::Unicode(value) if value == PARTICIPATION_EMOJI)
}

fn parse_recruit_config(content: &str) -> Result<RecruitConfig, String> {
    let block = extract_toml_block(content).ok_or("toml block not found")?;
    toml::from_str(block).map_err(|err| err.to_string())
}

fn extract_toml_block(content: &str) -> Option<&str> {
    let start_index = content.find("```toml")?;
    let rest = &content[start_index + "```toml".len()..];
    let end_index = rest.find("```")?;
    Some(rest[..end_index].trim())
}

async fn send_error_message(
    ctx: &serenity::Context,
    reaction: &serenity::Reaction,
) -> Result<(), Error> {
    let channel_id = reaction.channel_id;
    const ERROR_MESSAGE: &str =
        "募集設定の読み取りに失敗しました。募集メッセージを作り直してください。";
    let content = match reaction.user_id {
        Some(user_id) => format!("{} {}", user_id.mention(), ERROR_MESSAGE),
        None => ERROR_MESSAGE.to_string(),
    };
    channel_id.say(ctx.http.clone(), content).await?;
    Ok(())
}

async fn send_participation_notification(
    ctx: &serenity::Context,
    config: &RecruitConfig,
    reaction: &serenity::Reaction,
) -> Result<(), Error> {
    let user_id = match reaction.user_id {
        Some(user_id) => user_id,
        None => return Ok(()),
    };
    let channel_id = reaction.channel_id;
    let content = format!(
        "{} が {} に参加しました",
        user_id.mention(),
        config.game_title
    );
    let message = channel_id.say(ctx.http.clone(), content).await?;

    schedule_delete_message(ctx.http.clone(), channel_id, message.id);
    Ok(())
}

async fn send_start_notification(
    ctx: &serenity::Context,
    config: &RecruitConfig,
    message: &serenity::Message,
    role_id: serenity::RoleId,
    reaction_type: serenity::ReactionType,
) -> Result<(), Error> {
    let users = fetch_reaction_users(ctx, message, reaction_type.clone()).await?;
    let mentions: Vec<String> = users
        .into_iter()
        .filter(|user| !user.bot)
        .map(|user| user.mention().to_string())
        .collect();

    let content = format!(
        "{}\n{} が {} を開始します",
        role_id.mention(),
        mentions.join(" "),
        config.game_title
    );
    let channel_id = message.channel_id;
    let start_message = channel_id.say(ctx.http.clone(), content).await?;

    schedule_delete_message(ctx.http.clone(), channel_id, start_message.id);
    schedule_delete_reaction(ctx.http.clone(), channel_id, message.id, reaction_type);

    Ok(())
}

async fn fetch_reaction_users(
    ctx: &serenity::Context,
    message: &serenity::Message,
    reaction_type: serenity::ReactionType,
) -> Result<Vec<serenity::User>, Error> {
    let mut users = Vec::new();
    let mut after = None;

    loop {
        let chunk = message
            .reaction_users(ctx.http.clone(), reaction_type.clone(), Some(100), after)
            .await?;
        if chunk.is_empty() {
            break;
        }
        after = chunk.last().map(|user| user.id);
        let chunk_len = chunk.len();
        users.extend(chunk);
        if chunk_len < 100 {
            break;
        }
    }

    Ok(users)
}

fn schedule_delete_message(
    http: std::sync::Arc<serenity::Http>,
    channel_id: serenity::ChannelId,
    message_id: serenity::MessageId,
) {
    tokio::spawn(async move {
        sleep(Duration::from_secs(DELETE_DELAY_SECONDS)).await;
        let _ = channel_id.delete_message(&http, message_id).await;
    });
}

fn schedule_delete_reaction(
    http: std::sync::Arc<serenity::Http>,
    channel_id: serenity::ChannelId,
    recruit_message_id: serenity::MessageId,
    reaction_type: serenity::ReactionType,
) {
    tokio::spawn(async move {
        sleep(Duration::from_secs(DELETE_DELAY_SECONDS)).await;
        let _ = channel_id
            .delete_reaction_emoji(&http, recruit_message_id, reaction_type)
            .await;
        let _ = channel_id
            .create_reaction(&http, recruit_message_id, participation_reaction_type())
            .await;
    });
}

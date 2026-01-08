use std::collections::HashSet;

use poise::{CreateReply, serenity_prelude::*};
use serde::Deserialize;
use tokio::time::{Duration, sleep};

type Error = Box<dyn std::error::Error + Send + Sync>;

const DELETE_DELAY_SECONDS: u64 = 3600;
const PARTICIPATION_EMOJI: &str = "✋";
const SILENT_PARTICIPATION_EMOJI: &str = "🤚";

#[derive(Debug, Deserialize)]
struct RecruitConfig {
    game_title: String,
    required_players: usize,
    mention_role: Option<RoleId>,
    #[serde(default = "default_notify_on_reaction")]
    notify_on_reaction: bool,
    #[serde(default)]
    auto_assign_role_on_reaction: bool,
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    dotenvy::dotenv().ok();
    let token = std::env::var("DISCORD_TOKEN").expect("Missing DISCORD_TOKEN");

    let intents = GatewayIntents::non_privileged();

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

    let mut client = ClientBuilder::new(token, intents)
        .framework(framework)
        .await?;

    client.start().await?;
    Ok(())
}

async fn event_handler(
    ctx: &Context,
    event: &FullEvent,
    _framework: poise::FrameworkContext<'_, (), Error>,
    _data: &(),
) -> Result<(), Error> {
    if let FullEvent::ReactionAdd { add_reaction } = event {
        handle_reaction_add(ctx, add_reaction).await?;
    }
    Ok(())
}

/// 募集を作成します
#[poise::command(slash_command, guild_only)]
async fn recruit(
    ctx: poise::Context<'_, (), Error>,
    #[description = "募集するゲーム名"] game_title: String,
    #[description = "開始に必要な人数"] required_players: usize,
    #[description = "開始時にメンションするロール"] mention_role: Option<Role>,
    #[description = "ロールを作成するかどうか"] create_role: Option<bool>,
    #[description = "リアクション追加時にロールを自動付与するかどうか"]
    auto_assign_role_on_reaction: Option<bool>,
    #[description = "リアクション追加時に参加通知を送るかどうか"] notify_on_reaction: Option<bool>,
) -> Result<(), Error> {
    if required_players == 0 {
        ctx.say("required_players は 1 以上を指定してください。")
            .await?;
        return Ok(());
    }

    let create_role = create_role.unwrap_or(false);
    let mention_role_id = match mention_role {
        Some(ref role) => Some(role.id),
        None if create_role => {
            let guild_id = match ctx.guild_id() {
                Some(guild_id) => guild_id,
                None => {
                    ctx.send(
                        CreateReply::default()
                            .content("サーバー内でのみロールを作成できます。")
                            .ephemeral(true),
                    )
                    .await?;
                    return Ok(());
                }
            };
            let role = guild_id
                .create_role(ctx, EditRole::new().name(&game_title).mentionable(true))
                .await?;
            Some(role.id)
        }
        None => None,
    };

    let notify_on_reaction = notify_on_reaction.unwrap_or(true);
    let auto_assign_role_on_reaction =
        auto_assign_role_on_reaction.unwrap_or(create_role) && mention_role_id.is_some();

    let mut reaction_line = format!("{PARTICIPATION_EMOJI}: 参加");
    if notify_on_reaction {
        reaction_line += &format!("\n{SILENT_PARTICIPATION_EMOJI}: 参加通知なしで参加");
    }

    let mut config_lines = Vec::new();
    config_lines.push(format!("game_title = {game_title:?}"));
    config_lines.push(format!("required_players = {required_players}"));
    if let Some(role_id) = mention_role_id {
        config_lines.push(format!("mention_role = {role_id}"));
    }
    if !notify_on_reaction {
        config_lines.push(format!("notify_on_reaction = {notify_on_reaction}"));
    }
    if auto_assign_role_on_reaction {
        config_lines.push(format!(
            "auto_assign_role_on_reaction = {auto_assign_role_on_reaction}"
        ));
    }
    let config_block = config_lines.join("\n");

    let message_body = format!(
        r#"
このメッセージにリアクションをつけると {game_title} に参加できます
{reaction_line}
人数が揃ったら開始通知が送られます
```toml
{config_block}
```"#,
    );

    let message = ctx.channel_id().say(ctx.http(), message_body).await?;
    message
        .react(ctx.http(), participation_reaction_type())
        .await?;
    if notify_on_reaction {
        message
            .react(ctx.http(), silent_participation_reaction_type())
            .await?;
    }

    ctx.send(
        CreateReply::default()
            .content("募集メッセージを投稿しました")
            .ephemeral(true),
    )
    .await?;
    Ok(())
}

async fn handle_reaction_add(ctx: &Context, reaction: &Reaction) -> Result<(), Error> {
    if !is_supported_participation_reaction(&reaction.emoji) {
        return Ok(());
    }

    if reaction.user_id == Some(ctx.cache.current_user().id) {
        return Ok(());
    }

    let message = reaction.message(ctx).await?;
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

    if config.notify_on_reaction && is_participation_reaction(&reaction.emoji) {
        send_participation_notification(ctx, &config, reaction).await?;
    }

    if config.auto_assign_role_on_reaction
        && let Some(role_id) = config.mention_role
        && let Err(err) = assign_role_if_missing(ctx, reaction, role_id).await
    {
        eprintln!("Failed to assign role: {err}");
        send_role_assign_error(ctx, reaction).await?;
    }

    let mut user_ids = HashSet::new();
    user_ids.extend(fetch_reaction_users(ctx, &message, participation_reaction_type()).await?);
    user_ids
        .extend(fetch_reaction_users(ctx, &message, silent_participation_reaction_type()).await?);

    if config.required_players <= user_ids.len() {
        send_start_notification(ctx, &config, &message, config.mention_role, user_ids).await?;
    }

    Ok(())
}

fn participation_reaction_type() -> ReactionType {
    ReactionType::Unicode(PARTICIPATION_EMOJI.to_string())
}

fn silent_participation_reaction_type() -> ReactionType {
    ReactionType::Unicode(SILENT_PARTICIPATION_EMOJI.to_string())
}

fn is_participation_reaction(reaction: &ReactionType) -> bool {
    matches!(reaction, ReactionType::Unicode(value) if value == PARTICIPATION_EMOJI)
}

fn is_silent_participation_reaction(reaction: &ReactionType) -> bool {
    matches!(reaction, ReactionType::Unicode(value) if value == SILENT_PARTICIPATION_EMOJI)
}

fn is_supported_participation_reaction(reaction: &ReactionType) -> bool {
    is_participation_reaction(reaction) || is_silent_participation_reaction(reaction)
}

fn parse_recruit_config(content: &str) -> Result<RecruitConfig, String> {
    let block = extract_toml_block(content).ok_or("toml block not found")?;
    toml::from_str(block).map_err(|err| err.to_string())
}

fn default_notify_on_reaction() -> bool {
    true
}

fn extract_toml_block(content: &str) -> Option<&str> {
    let start_index = content.find("```toml")?;
    let rest = &content[start_index + "```toml".len()..];
    let end_index = rest.find("```")?;
    Some(rest[..end_index].trim())
}

async fn send_error_message(ctx: &Context, reaction: &Reaction) -> Result<(), Error> {
    let channel_id = reaction.channel_id;
    let content = reaction
        .user_id
        .map(|uid| uid.mention().to_string())
        .unwrap_or_default()
        + "募集設定の読み取りに失敗しました。募集メッセージを作り直してください。";
    channel_id.say(ctx, content).await?;
    Ok(())
}

async fn send_participation_notification(
    ctx: &Context,
    config: &RecruitConfig,
    reaction: &Reaction,
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
    let message = channel_id.say(ctx, content).await?;

    schedule_delete_message(ctx.http.clone(), channel_id, message.id);
    Ok(())
}

async fn send_start_notification(
    ctx: &Context,
    config: &RecruitConfig,
    message: &Message,
    role_id: Option<RoleId>,
    user_ids: HashSet<UserId>,
) -> Result<(), Error> {
    let mentions: Vec<String> = user_ids
        .into_iter()
        .map(|user_id| user_id.mention().to_string())
        .collect();

    let content = role_id
        .map(|rid| rid.mention().to_string() + "\n")
        .unwrap_or_default()
        + &format!(
            "{} が {} を開始します",
            mentions.join(" "),
            config.game_title
        );
    let channel_id = message.channel_id;
    let start_message = channel_id.say(ctx, content).await?;

    schedule_delete_message(ctx.http.clone(), channel_id, start_message.id);

    channel_id
        .delete_reaction_emoji(ctx, message.id, participation_reaction_type())
        .await?;
    if config.notify_on_reaction {
        channel_id
            .delete_reaction_emoji(ctx, message.id, silent_participation_reaction_type())
            .await?;
    }
    channel_id
        .create_reaction(ctx, message.id, participation_reaction_type())
        .await?;
    if config.notify_on_reaction {
        channel_id
            .create_reaction(ctx, message.id, silent_participation_reaction_type())
            .await?;
    }

    Ok(())
}

async fn assign_role_if_missing(
    ctx: &Context,
    reaction: &Reaction,
    role_id: RoleId,
) -> Result<(), Error> {
    let Some(user_id) = reaction.user_id else {
        return Ok(());
    };
    let Some(guild_id) = reaction.guild_id else {
        return Ok(());
    };
    let member = guild_id.member(ctx, user_id).await?;
    if member.roles.contains(&role_id) {
        return Ok(());
    }
    member.add_role(ctx, role_id).await?;
    Ok(())
}

async fn send_role_assign_error(ctx: &Context, reaction: &Reaction) -> Result<(), Error> {
    let channel_id = reaction.channel_id;
    const ERROR_MESSAGE: &str = "ロールの付与に失敗しました。権限を確認してください。";
    let content = match reaction.user_id {
        Some(user_id) => format!("{} {}", user_id.mention(), ERROR_MESSAGE),
        None => ERROR_MESSAGE.to_string(),
    };
    channel_id.say(ctx, content).await?;
    Ok(())
}

async fn fetch_reaction_users(
    ctx: &Context,
    message: &Message,
    reaction_type: ReactionType,
) -> Result<Vec<UserId>, Error> {
    let mut users = Vec::new();
    let mut after = None;

    loop {
        let chunk = message
            .reaction_users(ctx, reaction_type.clone(), Some(100), after)
            .await?
            .into_iter()
            .filter(|user| !user.bot)
            .map(|user| user.id)
            .collect::<Vec<_>>();
        if chunk.is_empty() {
            break;
        }
        after = chunk.last().copied();
        let chunk_len = chunk.len();
        users.extend(chunk);
        if chunk_len < 100 {
            break;
        }
    }

    Ok(users)
}

fn schedule_delete_message(
    http: std::sync::Arc<Http>,
    channel_id: ChannelId,
    message_id: MessageId,
) {
    tokio::spawn(async move {
        sleep(Duration::from_secs(DELETE_DELAY_SECONDS)).await;
        let _ = channel_id.delete_message(&http, message_id).await;
    });
}

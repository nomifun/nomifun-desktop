#[cfg(feature = "telegram")]
pub mod telegram;

#[cfg(feature = "lark")]
pub mod lark;

#[cfg(feature = "dingtalk")]
pub mod dingtalk;

#[cfg(feature = "weixin")]
pub mod weixin;

#[cfg(feature = "wecom")]
pub mod wecom;

#[cfg(feature = "discord")]
pub mod discord;

#[cfg(feature = "matrix")]
pub mod matrix;

#[cfg(feature = "mattermost")]
pub mod mattermost;

#[cfg(feature = "slack")]
pub mod slack;

#[cfg(feature = "twitch")]
pub mod twitch;

#[cfg(feature = "nostr")]
pub mod nostr;

#[cfg(feature = "qqbot")]
pub mod qqbot;

/// Shared callback-data encoding for interactive buttons (Telegram/Discord/Slack/...).
#[cfg(any(feature = "telegram", feature = "discord", feature = "slack", feature = "qqbot"))]
pub mod callback;

/// Small helpers shared by the plugins.
#[cfg(any(
    feature = "telegram",
    feature = "lark",
    feature = "dingtalk",
    feature = "wecom",
    feature = "discord",
    feature = "matrix",
    feature = "mattermost",
    feature = "slack",
    feature = "twitch",
    feature = "nostr",
    feature = "qqbot"
))]
pub(crate) mod util;

use crate::plugin::ChannelPlugin;
use crate::types::PluginType;

/// Create a platform-specific plugin instance from a `PluginType`.
///
/// Returns `None` if the platform feature is not compiled in.
pub fn create_plugin(plugin_type: PluginType) -> Option<Box<dyn ChannelPlugin>> {
    match plugin_type {
        #[cfg(feature = "telegram")]
        PluginType::Telegram => Some(Box::new(telegram::TelegramPlugin::new())),

        #[cfg(feature = "lark")]
        PluginType::Lark => Some(Box::new(lark::LarkPlugin::new())),

        #[cfg(feature = "dingtalk")]
        PluginType::Dingtalk => Some(Box::new(dingtalk::DingtalkPlugin::new())),

        #[cfg(feature = "weixin")]
        PluginType::Weixin => Some(Box::new(weixin::WeixinPlugin::new())),

        #[cfg(feature = "wecom")]
        PluginType::Wecom => Some(Box::new(wecom::WecomPlugin::new())),

        #[cfg(feature = "discord")]
        PluginType::Discord => Some(Box::new(discord::DiscordPlugin::new())),

        #[cfg(feature = "matrix")]
        PluginType::Matrix => Some(Box::new(matrix::MatrixPlugin::new())),

        #[cfg(feature = "mattermost")]
        PluginType::Mattermost => Some(Box::new(mattermost::MattermostPlugin::new())),

        #[cfg(feature = "slack")]
        PluginType::Slack => Some(Box::new(slack::SlackPlugin::new())),

        #[cfg(feature = "twitch")]
        PluginType::Twitch => Some(Box::new(twitch::TwitchPlugin::new())),

        #[cfg(feature = "nostr")]
        PluginType::Nostr => Some(Box::new(nostr::NostrPlugin::new())),

        #[cfg(feature = "qqbot")]
        PluginType::Qqbot => Some(Box::new(qqbot::QqbotPlugin::new())),

        #[allow(unreachable_patterns)]
        _ => None,
    }
}

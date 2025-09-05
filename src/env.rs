#[derive(Debug)]
pub struct EnvCfg {
    pub discord_bot_token: String,
    pub announce_channel_id: u64,
    pub role_id: u64,
    pub timezone: String,
    pub dict_path: String,
    pub state_path: String,
}

impl EnvCfg {
    pub fn from_env() -> anyhow::Result<Self> {
        dotenvy::dotenv().ok();
        let discord_bot_token = std::env::var("DISCORD_BOT_TOKEN")?;
        let announce_channel_id = std::env::var("ANNOUNCE_CHANNEL_ID")?.parse()?;
        let role_id = std::env::var("WORDLE_ROLE_ID")?.parse()?;
        let timezone = std::env::var("TIMEZONE")?;
        let dict_path = std::env::var("DICT_PATH")?;
        let state_path = std::env::var("STATE_PATH")?;
        Ok(Self {
            discord_bot_token,
            announce_channel_id,
            role_id,
            timezone,
            dict_path,
            state_path,
        })
    }
}

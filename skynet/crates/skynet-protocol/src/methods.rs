// Well-known WS method names — must match OpenClaw client expectations.

// chat
pub const CHAT_SEND: &str = "chat.send";
pub const CHAT_ABORT: &str = "chat.abort";

// sessions
pub const SESSIONS_LIST: &str = "sessions.list";
pub const SESSIONS_PREVIEW: &str = "sessions.preview";
pub const SESSIONS_RESOLVE: &str = "sessions.resolve";

// config
pub const CONFIG_GET: &str = "config.get";
pub const CONFIG_SET: &str = "config.set";

// agent
pub const AGENT_STATUS: &str = "agent.status";
pub const AGENT_LIST: &str = "agent.list";

// channels
pub const CHANNELS_STATUS: &str = "channels.status";
pub const CHANNELS_LOGOUT: &str = "channels.logout";

// cron / scheduler
pub const CRON_LIST: &str = "cron.list";
pub const CRON_ADD: &str = "cron.add";
pub const CRON_DELETE: &str = "cron.delete";

// node (multi-node, future)
pub const NODE_LIST: &str = "node.list";
pub const NODE_INVOKE: &str = "node.invoke";

// handshake
pub const CONNECT: &str = "connect";

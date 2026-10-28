//! What the battle is told about this machine's agent sessions: the types
//! the collectors fill in and the scene consumes.

/// What a session is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum State {
    Work,
    #[default]
    Idle,
    Wait,
}

/// Why a waiting session waits: a permission prompt ("!") or a question ("?").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Why {
    #[default]
    None,
    Perm,
    Ask,
}

/// What the card's last message is: something the agent said, the question
/// it waits on, or the tool call it waits to have approved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SaidKind {
    #[default]
    Msg,
    Ask,
    Tool,
}

/// A tool call, by kind: each is a spell for a mage (FIRE, BLIZZARD, SCAN,
/// GALE, the class special, PROTECT, THUNDER); a fighter answers every one
/// with its special.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Bash,
    Edit,
    Read,
    Web,
    Agent,
    Todo,
    Mcp,
}

impl Tool {
    pub const ALL: [Tool; 7] = [Tool::Bash, Tool::Edit, Tool::Read, Tool::Web, Tool::Agent, Tool::Todo, Tool::Mcp];

    /// "bash", "edit", ... as `--print` spells them.
    pub fn name(self) -> &'static str {
        ["bash", "edit", "read", "web", "agent", "todo", "mcp"][self as usize]
    }
}

/// One live session as the battle sees it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Session {
    /// A short stable id (8 hex characters for real sessions).
    pub id: String,
    /// The project: its git remote as host/owner/repo, else its directory
    /// with home as ~; "" when unknown. The battle shows the last part.
    pub proj: String,
    /// A Codex session is a fighter, a Claude Code one a mage.
    pub codex: bool,
    /// The session's terminal can be brought to the front.
    pub term: bool,
    pub state: State,
    pub why: Why,
    /// How long the session has been in this state, seconds.
    pub age_s: u32,
    /// How long the session has been running, seconds; 0 = unknown.
    pub up_s: u32,
    /// Tokens burned since this app started (its damage); < 0 = unknown.
    pub burn: f64,
    /// The session's whole spend; tokens < 0 = unknown.
    pub tokens: f64,
    pub usd: f64,
    /// The opening of its last message as plain ASCII, at most 400 characters; "" = none yet.
    pub said: String,
    pub said_kind: SaidKind,
    /// How long ago it was said, seconds; < 0 = unknown.
    pub said_age_s: i32,
}

/// This month's totals, Claude and Codex summed: GOLD (dollars) and EXP (tokens).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Usage {
    pub tokens: f64,
    pub usd: f64,
}

/// Quota left on the plans, percent; < 0 = unknown. The MP bars.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mana {
    pub claude_5h: i32,
    pub claude_week: i32,
    pub codex_week: i32,
}

impl Default for Mana {
    fn default() -> Self {
        Mana { claude_5h: -1, claude_week: -1, codex_week: -1 }
    }
}

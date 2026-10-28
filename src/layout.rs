//! Where everything sits on the 426x266 canvas (design/engine.js): the
//! battlefield on top, monsters on its left and the party in two columns of
//! four on its right; the monster, party and info windows along the bottom.

pub const FIELD_H: i32 = 176;

/// The party: slot s stands at (party_sx(s), party_sy(s)), feet on the ground.
pub const PARTY: usize = 8;
pub const fn party_sx(s: usize) -> f32 {
    (300 + if s < 4 { 0 } else { 54 } + (s % 4) as i32 * 6) as f32
}
pub const fn party_sy(s: usize) -> f32 {
    (80 + (s % 4) as i32 * 29) as f32
}

/* the windows along the bottom */
pub const WIN_Y: i32 = FIELD_H;
pub const WIN_H: i32 = 266 - FIELD_H;
pub const MON_X: i32 = 0;
pub const MON_W: i32 = 126;
pub const PARTY_X: i32 = 126;
pub const PARTY_W: i32 = 222;
pub const INFO_X: i32 = 348;
pub const INFO_W: i32 = 78;
pub const ROW_Y: i32 = WIN_Y + 6;
pub const ROW_H: i32 = 10;
/// The "+N WAIT" tab over the party window, for sessions past the eighth.
pub const BENCH_X: i32 = 226;

/* the status card a click on a player opens, over the field */
pub const CARD_X: i32 = 8;
pub const CARD_Y: i32 = 4;
pub const CARD_W: i32 = 410;
pub const CARD_H: i32 = 168;
pub const CARD_PORT_X: i32 = CARD_X + 10;
pub const CARD_PORT_Y: i32 = CARD_Y + 10;
pub const CARD_PORT_W: i32 = 84;
pub const CARD_PORT_H: i32 = 96;
pub const CARD_COL_X: i32 = CARD_X + 106;
pub const CARD_ROW_Y: i32 = CARD_Y + 46;
pub const CARD_ROW_H: i32 = 10;
/* the last message: 4 lines of 64 characters */
pub const SAID_Y: i32 = CARD_Y + 110;
pub const SAID_ROWS: usize = 4;
pub const SAID_COLS: usize = 64;
pub const SAID_PITCH: i32 = 10;

/* the config window a click on the info window opens */
pub const CFG_X: i32 = 108;
pub const CFG_Y: i32 = 34;
pub const CFG_W: i32 = 210;
pub const CFG_H: i32 = 108;
pub const CFG_BTN_X: i32 = CFG_X + CFG_W - 76;
pub const CFG_BTN_W: i32 = 64;
pub const CFG_BTN_H: i32 = 13;
pub const fn cfg_row_y(r: i32) -> i32 {
    CFG_Y + 24 + r * 18
}
pub const CFG_CLOSE_Y: i32 = CFG_Y + CFG_H - 22;

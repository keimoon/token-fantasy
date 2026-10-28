/*
 * The battle scene on the 426x266 canvas: the field with up to 4 monsters (projects) on the left and up to
 * 8 players (agent sessions) in a 2x4 formation on the right, the monster,
 * party and info windows along the bottom, and the effects.
 *
 * Everything that moves runs as a generator ("an action") that yields and
 * receives the frame's dt; the host feeds sessions in with join(), state(),
 * tool(), tokens() and leave().
 */
(function (root) {
'use strict';
const FFC = root.FFC, PL = root.FF_PLAYERS;
const W = 426, H = 266, FIELD_H = 176;
const hex = FFC.hexPx;

/* ------------------------------------------------------------ rules */

const RULES = {
    hpMax: 999,
    atbSeconds: 8,          // a working player attacks this often, for the tokens burned since the last one
    castGap: 2.5,           // at most one spell per player this often; extra tool calls fold into it
    monsterSeconds: 30,     // a monster hits one idle player of its project this often (real time)
    koSeconds: 600,         // idle this long and the player is knocked out
    critical: 0.25,         // kneels under this much HP
};
/* the page runs idle time faster, so a KO shows within a minute */
let idleScale = 20;

const TOOLS = {
    bash:  { spell: 'FIRE',     fx: 'fire',    label: 'Bash',      tools: 'Bash' },
    edit:  { spell: 'BLIZZARD', fx: 'ice',     label: 'Edit',      tools: 'Edit, Write, NotebookEdit' },
    read:  { spell: 'SCAN',     fx: 'scan',    label: 'Read',      tools: 'Read, Grep, Glob, LS' },
    web:   { spell: 'GALE',     fx: 'gale',    label: 'WebFetch',  tools: 'WebFetch, WebSearch' },
    agent: { spell: 'SUMMON',   fx: 'summon',  label: 'Agent',     tools: 'Agent, Task (a subagent)' },
    todo:  { spell: 'PROTECT',  fx: 'protect', label: 'TodoWrite', tools: 'TodoWrite' },
    mcp:   { spell: 'THUNDER',  fx: 'bolt',    label: 'MCP',       tools: 'mcp__* tools and anything else' },
};
const SUMMONS = ['DRAGON KING', 'SALAMANDER', 'FROST QUEEN', 'RAIJIN'];
const SLASH_OF = { sword: 'slash', dagger: 'slash2', rapier: 'thrust', lance: 'thrust', staffB: 'bonk', staffW: 'bonk', rod: 'bonk' };

/* ------------------------------------------------------------ colours */

const COL = {
    win0: hex('#2c58c8'), win1: hex('#0e1a5a'), border: hex('#eceaf4'), borderLo: hex('#8a90b8'),
    outer: hex('#06081c'), text: hex('#f4f4f8'), shadow: hex('#080a28'), dim: hex('#a8b0d8'),
    gold: hex('#f8d860'), red: hex('#ff5a50'), green: hex('#5ee08a'), black: hex('#000000'),
    white: hex('#ffffff'), ko: hex('#7a7c98'),
    atbOff: hex('#1a2250'), atbFill: hex('#f8d860'), atbFull: hex('#fff4c0'), atbIdle: hex('#5a6290'),
    mana5h: hex('#50c8ff'), manaWeek: hex('#b48cff'), manaCodex: hex('#5ee08a'),
    claude: hex('#d97757'), claudeHi: hex('#f4b090'), codexBg: hex('#3e4250'), codexFg: hex('#e8f0f4'), codexHi: hex('#6ae0d0'),
    lime: hex('#b4f03c'),
};

/* ------------------------------------------------------------ frame buffer */

let fb = null;
function put(x, y, c) { x |= 0; y |= 0; if (x >= 0 && y >= 0 && x < W && y < H) fb[y * W + x] = c; }
function rect(x, y, w, h, c) {
    x |= 0; y |= 0;
    const x0 = Math.max(0, x), y0 = Math.max(0, y), x1 = Math.min(W, x + w), y1 = Math.min(H, y + h);
    if (x1 <= x0) return;
    for (let j = y0; j < y1; j++) fb.fill(c, j * W + x0, j * W + x1);
}
function chan(c) { return [c & 255, (c >>> 8) & 255, (c >>> 16) & 255]; }
function mix(a, b, t) {
    const A = chan(a), B = chan(b);
    return FFC.pack([A[0] + (B[0] - A[0]) * t | 0, A[1] + (B[1] - A[1]) * t | 0, A[2] + (B[2] - A[2]) * t | 0]);
}
function blend(x, y, c, t) { x |= 0; y |= 0; if (x >= 0 && y >= 0 && x < W && y < H) fb[y * W + x] = mix(fb[y * W + x], c, t); }
function line(x0, y0, x1, y1, c, wdt = 1) {
    x0 |= 0; y0 |= 0; x1 |= 0; y1 |= 0;
    const dx = Math.abs(x1 - x0), dy = -Math.abs(y1 - y0), sx = x0 < x1 ? 1 : -1, sy = y0 < y1 ? 1 : -1;
    let e = dx + dy;
    for (;;) {
        if (wdt === 1) put(x0, y0, c); else rect(x0 - (wdt >> 1), y0 - (wdt >> 1), wdt, wdt, c);
        if (x0 === x1 && y0 === y1) break;
        const e2 = 2 * e;
        if (e2 >= dy) { e += dy; x0 += sx; }
        if (e2 <= dx) { e += dx; y0 += sy; }
    }
}
function disc(cx, cy, r, c) {
    for (let j = -r; j <= r; j++) for (let i = -r; i <= r; i++) if (i * i + j * j <= r * r + r * 0.6) put(cx + i, cy + j, c);
}
function ring(cx, cy, rx, ry, c) {
    const n = Math.max(12, (rx + ry) * 4 | 0);
    for (let k = 0; k < n; k++) { const a = k / n * Math.PI * 2; put(cx + Math.cos(a) * rx, cy + Math.sin(a) * ry, c); }
}
function ellipseShade(cx, cy, rx, ry, t) {
    for (let j = -ry; j <= ry; j++) for (let i = -rx; i <= rx; i++)
        if ((i * i) / (rx * rx) + (j * j) / (ry * ry) <= 1) blend(cx + i, cy + j, COL.black, t);
}

const BAYER = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5];

/* opts: flip, white (0..1 towards white), tint [colour, t], vis (0..1 dither), sweep (dissolve from the top) */
function blit(img, x, y, o = {}) {
    x |= 0; y |= 0;
    for (let j = 0; j < img.h; j++) {
        const yy = y + j;
        if (yy < 0 || yy >= H) continue;
        let vis = o.vis === undefined ? 1 : o.vis;
        if (o.sweep !== undefined) vis = Math.min(vis, Math.max(0, Math.min(1, (1 - o.sweep) * 2.2 - j / img.h * 1.2)));
        for (let i = 0; i < img.w; i++) {
            const v = img.px[j * img.w + (o.flip ? img.w - 1 - i : i)];
            if (!v) continue;
            const xx = x + i;
            if (xx < 0 || xx >= W) continue;
            if (vis < 1 && BAYER[(yy & 3) * 4 + (xx & 3)] >= vis * 16) continue;
            let c = v;
            if (o.tint) c = mix(c, o.tint[0], o.tint[1]);
            if (o.white) c = mix(c, COL.white, o.white);
            fb[yy * W + xx] = c;
        }
    }
}

/* ------------------------------------------------------------ text */

function glyph(ch) { return FFC.GLYPH[ch] || FFC.GLYPH['?']; }
function text(s, x, y, c, shadow = COL.shadow) {
    for (const ch of String(s)) {
        const g = glyph(ch);
        for (let j = 0; j < 7; j++) for (let i = 0; i < 5; i++) if (g[j][i] === '#') {
            if (shadow) put(x + i + 1, y + j + 1, shadow);
            put(x + i, y + j, c);
        }
        x += 6;
    }
}
function textR(s, xr, y, c, shadow) { text(s, xr - String(s).length * 6 + 1, y, c, shadow); }
/* damage numbers: outlined all round, like the classics' */
function numText(s, cx, y, c) {
    const w = String(s).length * 6 - 1;
    let x = (cx - w / 2) | 0;
    for (const ch of String(s)) {
        const g = glyph(ch);
        for (let j = 0; j < 7; j++) for (let i = 0; i < 5; i++) if (g[j][i] === '#')
            for (let dj = -1; dj <= 1; dj++) for (let di = -1; di <= 1; di++) put(x + i + di, y + j + dj, COL.outer);
        for (let j = 0; j < 7; j++) for (let i = 0; i < 5; i++) if (g[j][i] === '#') put(x + i, y + j, c);
        x += 6;
    }
}

function fmtTok(n) {
    n = Math.round(n);
    if (n < 10000) return String(n);
    if (n < 1e5) return (n / 1e3).toFixed(1) + 'K';
    if (n < 1e6) return Math.round(n / 1e3) + 'K';
    if (n < 1e7) return (n / 1e6).toFixed(2) + 'M';
    if (n < 1e8) return (n / 1e6).toFixed(1) + 'M';
    return Math.round(n / 1e6) + 'M';
}

/* the window: dark outer edge, white border, a lighter inner line, blue bands */
function window_(x, y, w, h) {
    rect(x + 2, y, w - 4, h, COL.outer);
    rect(x, y + 2, w, h - 4, COL.outer);
    rect(x + 1, y + 1, w - 2, h - 2, COL.outer);
    rect(x + 2, y + 1, w - 4, h - 2, COL.border);
    rect(x + 1, y + 2, w - 2, h - 4, COL.border);
    rect(x + 3, y + 3, w - 6, h - 6, COL.borderLo);
    const ih = h - 8, bands = 8;
    for (let j = 0; j < ih; j++) rect(x + 4, y + 4 + j, w - 8, 1, mix(COL.win0, COL.win1, Math.floor(j * bands / ih) / (bands - 1)));
}

/* ------------------------------------------------------------ small sprites */

function spr(rows, pal, outline = false) { return FFC.raster(FFC.sprite('s', rows, pal), outline); }
const ICON = {
    claude: spr(['A..A..A', '.A.A.A.', '..AAA..', 'AAAaAAA', '..AAA..', '.A.A.A.', 'A..A..A'], { A: '#d97757', a: '#f4b090' }),
    codex: spr(['LLLLLLL', 'LALLLLL', 'LLALLLL', 'LLLALLL', 'LLALLLL', 'LALLaaL', 'LLLLLLL'], { L: '#3e4250', A: '#e8f0f4', a: '#6ae0d0' }),
    cursor: spr([
        '..KKK.......',
        '.KWWWKKKKKK.',
        'KWWWWWWWWWWK',
        'KWWWWWKKKKK.',
        'KWWWWWWK....',
        'KwWWWWKK....',
        'KwwWWWWK....',
        '.KKKKKK.....',
    ], {}),
    feather: spr([
        '...Y.',
        '..YX.',
        '..XX.',
        '.XXx.',
        '.XXx.',
        'XXx..',
        'XXx..',
        'Xx...',
        'x....',
        'n....',
    ], { X: '#f86040', x: '#b02818' }, true),
    bubbleAsk: spr([
        '.KKKKKKK.',
        'KWWWWWWWK',
        'KWWKKKWWK',
        'KWKWWWKWK',
        'KWWWWKWWK',
        'KWWWKWWWK',
        'KWWWWWWWK',
        'KWWWKWWWK',
        '.KKKKKKK.',
        '...KWK...',
        '....K....',
    ], {}),
    bubblePerm: spr([
        '.KKKKKKK.',
        'KYYYYYYYK',
        'KYYYKYYYK',
        'KYYYKYYYK',
        'KYYYKYYYK',
        'KYYYKYYYK',
        'KYYYYYYYK',
        'KYYYKYYYK',
        '.KKKKKKK.',
        '...KYK...',
        '....K....',
    ], {}),
    rock: spr(['..OON..', '.OYONNn', 'OYONNnn', 'ONNNNnn', 'NNNnNnn', '.NnnNn.', '..nnn..'], { O: '#f8a040', N: '#9a6a48', n: '#5a3a28' }, true),
    skull: spr(['.WWW.', 'WWWWW', 'WKWKW', 'WWWWW', '.W.W.'], {}),
    /* the config gear in the info window's corner */
    gear: spr(['....W....', '.W.WWW.W.', '..WWWWW..', '.WWW.WWW.', 'WWW...WWW', '.WWW.WWW.', '..WWWWW..', '.W.WWW.W.', '....W....'], {}),
};
ICON.cursorL = FFC.flipX(ICON.cursor);

/* ------------------------------------------------------------ field */

const TOD = {
    day: { sky: ['#4a88e0', '#8ac0f0', '#d0e8f8'], far: '#7898c8', near: '#4a6a98', trees: '#2a5a3a', g0: '#3a7a3a', g1: '#5a9a48', tuft: '#7ab858', rock: '#8a8478', cloud: '#f4f8ff', stars: 0 },
    dusk: { sky: ['#2a2a6a', '#a04a6a', '#f0a060'], far: '#6a4a78', near: '#4a3060', trees: '#2a2a40', g0: '#3a4a3a', g1: '#5a6a40', tuft: '#7a8a48', rock: '#7a6060', cloud: '#f8c0a0', stars: 0.3 },
    night: { sky: ['#06081c', '#141a3a', '#2a3468'], far: '#20284a', near: '#161c36', trees: '#0e1a1c', g0: '#16261e', g1: '#22382a', tuft: '#30503a', rock: '#3a4050', cloud: '#2a3458', stars: 1 },
};
const HORIZON = 64;

function rng(seed) { let s = seed >>> 0; return () => { s = (s * 1664525 + 1013904223) >>> 0; return s / 4294967296; }; }

function makeField(tod) {
    const P = TOD[tod], save = fb;
    fb = new Uint32Array(W * H);
    const r = rng(7);
    const sky = P.sky.map(hex);
    for (let y = 0; y < HORIZON; y++) {
        const t = y / HORIZON, seg = t < 0.6 ? 0 : 1, tt = seg === 0 ? t / 0.6 : (t - 0.6) / 0.4;
        for (let x = 0; x < W; x++) {
            const d = BAYER[(y & 3) * 4 + (x & 3)] / 16;
            const band = Math.floor(tt * 6 + d) / 6;
            fb[y * W + x] = mix(sky[seg], sky[seg + 1], Math.min(1, band));
        }
    }
    if (P.stars) for (let k = 0; k < 70 * P.stars; k++) put(r() * W, r() * HORIZON * 0.75, r() < 0.3 ? hex('#f8f0c0') : hex('#8a88a8'));
    /* clouds */
    const cl = hex(P.cloud);
    for (let k = 0; k < 5; k++) {
        const cx = r() * W, cy = 8 + r() * 26, n = 3 + (r() * 3 | 0);
        for (let b = 0; b < n; b++) {
            const bx = cx + b * 7 - n * 3, by = cy + (b % 2) * 2, rr = 3 + (r() * 3 | 0);
            for (let j = -rr; j <= 0; j++) for (let i = -rr * 2; i <= rr * 2; i++)
                if (i * i / 4 + j * j <= rr * rr) blend(bx + i, by + j, cl, tod === 'night' ? 0.35 : 0.8);
        }
    }
    /* mountains: far, then near */
    const ridge = (base, amp, freq, c, seed) => {
        const rr = rng(seed);
        const ph = [rr() * 9, rr() * 9, rr() * 9];
        for (let x = 0; x < W; x++) {
            const h = Math.sin(x * freq + ph[0]) * amp + Math.sin(x * freq * 2.3 + ph[1]) * amp * 0.5 + Math.sin(x * freq * 5.1 + ph[2]) * amp * 0.2;
            for (let y = (base - h) | 0; y < HORIZON; y++) put(x, y, c);
        }
    };
    ridge(HORIZON - 14, 9, 0.021, hex(P.far), 3);
    ridge(HORIZON - 6, 6, 0.034, hex(P.near), 11);
    /* tree line */
    const tc = hex(P.trees);
    for (let x = 0; x < W; x++) {
        const h = 3 + ((Math.sin(x * 0.7) + Math.sin(x * 1.9)) * 1.5 | 0) + (x % 7 === 0 ? 2 : 0);
        for (let y = HORIZON - h; y < HORIZON + 2; y++) put(x, y, tc);
    }
    /* ground: darker far, lighter near, dithered bands, tufts and pebbles */
    const g0 = hex(P.g0), g1 = hex(P.g1), tuft = hex(P.tuft), rock = hex(P.rock);
    for (let y = HORIZON + 2; y < FIELD_H; y++) {
        const t = (y - HORIZON) / (FIELD_H - HORIZON);
        for (let x = 0; x < W; x++) {
            const d = BAYER[(y & 3) * 4 + (x & 3)] / 16;
            fb[y * W + x] = mix(g0, g1, Math.min(1, Math.floor(t * 5 + d * 0.9) / 5));
        }
    }
    for (let k = 0; k < 120; k++) {
        const y = HORIZON + 4 + Math.pow(r(), 0.7) * (FIELD_H - HORIZON - 6), x = r() * W;
        const s = (y - HORIZON) / (FIELD_H - HORIZON);
        if (r() < 0.85) { put(x, y, tuft); put(x - 1, y + 1, tuft); put(x + 1, y + 1, tuft); if (s > 0.5) put(x, y - 1, tuft); }
        else { rect(x, y, 2 + s * 3, 1 + s * 2, rock); put(x, y, mix(rock, COL.white, 0.3)); }
    }
    const out = fb;
    fb = save;
    return out;
}

/* ------------------------------------------------------------ art caches */

const CLS = PL.CLASSES.map(c => {
    const variants = [0, 1].map(v => {
        const sp = PL.build(c, v), imgs = {};
        for (const k of Object.keys(sp)) imgs[k] = PL.rasterPose(sp[k]);
        imgs.stepR = FFC.flipX(imgs.step);
        imgs.readyR = FFC.flipX(imgs.ready);
        return imgs;
    });
    return { c, variants, imgs: variants[0] };
});
/* upright sprites stand on (21, 44) of their 38x46 image; the KO sprite lies
 * with its lowest pixel on the ground, centred */
const UP_AX = 21, UP_AY = 44;
function koAnchor(img) {
    let minx = img.w, maxx = 0, maxy = 0;
    for (let j = 0; j < img.h; j++) for (let i = 0; i < img.w; i++) if (img.px[j * img.w + i]) { minx = Math.min(minx, i); maxx = Math.max(maxx, i); maxy = Math.max(maxy, j); }
    return [(minx + maxx) >> 1, maxy];
}
CLS.forEach(k => { k.koA = koAnchor(k.imgs.ko); });

let MONS = [];
function loadMonsters() {
    MONS = (root.FF_MONSTERS || []).map(m => {
        const f = {};
        for (const [k, rows] of Object.entries(m.frames)) f[k] = FFC.raster(FFC.sprite(`${m.id}.${k}`, rows, m.pal));
        return { m, f };
    });
}
loadMonsters();

/* ------------------------------------------------------------ layout */

const PARTY_SLOTS = [];
for (let s = 0; s < 8; s++) {
    const col = s < 4 ? 0 : 1, row = s % 4;
    PARTY_SLOTS.push([300 + col * 54 + row * 6, 80 + row * 29]);
}
const MON_SLOTS = {
    1: [[112, 144]],
    2: [[78, 122], [168, 160]],
    3: [[68, 106], [170, 124], [96, 168]],
    4: [[62, 106], [164, 106], [82, 168], [184, 168]],
};

const MON_X = 0, MON_W = 126, PARTY_X = 126, PARTY_W = 222, INFO_X = 348, INFO_W = 78, WIN_Y = FIELD_H, WIN_H = H - FIELD_H;
const ROW_Y = WIN_Y + 6, ROW_H = 10;

/* ------------------------------------------------------------ actors */

function* wait(s) { let t = 0; while (t < s) t += yield; }
function* tween(s, fn) { let t = 0; while (t < s) { fn(t / s); t += yield; } fn(1); }

class Actor {
    constructor() { this.queue = []; this.co = null; }
    push(genFn) { this.queue.push(genFn); }
    busy() { return !!this.co || this.queue.length > 0; }
    step(dt) {
        if (!this.co && this.queue.length) { this.co = this.queue.shift()(); this.co.next(); }
        if (this.co && this.co.next(dt).done) this.co = null;
    }
}

const S = {
    players: [], monsters: [], fx: [], nums: [], banner: null, log: [],
    tod: 'auto', field: null, fieldKey: '', atbMode: 'wait', selected: null,
    month: { usd: 214.37, tokens: 48.2e6 }, mana: { c5h: 72, cwk: 41, xwk: 89 },
    time: 0, flash: 0, dim: 0,
};
const projDamage = {};   // project -> damage dealt, for the life of the host

function logEvent(s) { S.log.unshift({ t: S.time, s }); if (S.log.length > 40) S.log.pop(); if (root.FF_ONLOG) root.FF_ONLOG(S.log); }
function banner(textS, sub, prio = 1) {
    if (S.banner && S.banner.prio > prio && S.banner.t < S.banner.dur) return;
    S.banner = { text: textS, sub: sub || '', t: 0, dur: prio > 1 ? 2.0 : 1.3, prio };
}
/* numbers on the same spot stack upwards instead of overprinting */
function popNum(x, y, s, c) {
    const near = S.nums.filter(n => Math.abs(n.x - x) < 24 && Math.abs(n.y0 - y) < 30 && n.t < 0.9).length;
    S.nums.push({ x, y: y - near * 9, y0: y, s, c, t: 0 });
}

function monsterOf(proj) { return S.monsters.find(m => m.proj === proj && !m.leaving); }
function kindForProject(proj) {
    let h = 2166136261;
    for (const ch of proj) h = Math.imul(h ^ ch.charCodeAt(0), 16777619) >>> 0;
    const used = new Set(S.monsters.filter(m => !m.leaving).map(m => m.kind));
    for (let k = 0; k < MONS.length; k++) { const kk = (h + k) % MONS.length; if (!used.has(kk)) return kk; }
    return h % Math.max(1, MONS.length);
}
function relayoutMonsters() {
    const live = S.monsters.filter(m => !m.leaving);
    const slots = MON_SLOTS[Math.max(1, live.length)] || MON_SLOTS[4];
    live.forEach((m, i) => { m.tx = slots[i][0]; m.ty = slots[i][1]; if (m.x === undefined) { m.x = m.tx; m.y = m.ty; } });
}

function addMonster(proj) {
    if (!MONS.length) return null;
    const m = new Actor();
    Object.assign(m, { proj, kind: kindForProject(proj), vis: 0, white: 0, shake: 0, frame: 'idle', leaving: false, timer: 2 + Math.random() * 2, phase: Math.random() * 6 });
    S.monsters.push(m);
    relayoutMonsters();
    const name = MONS[m.kind].m.name;
    m.push(function* () { banner(`${name} APPEARS`); logEvent(`${name} appears: project ${proj}`); yield* tween(0.7, t => { m.vis = t; }); });
    return m;
}
/* how: 'defeated' when the project's last session has ended, 'flees' when a
 * busier project takes its place */
function removeMonster(m, how = 'defeated') {
    m.leaving = true;
    const name = MONS[m.kind].m.name;
    m.queue.length = 0; m.co = null;
    m.push(function* () {
        if (how === 'defeated') {
            banner(`${name} DEFEATED`, `${fmtTok(projDamage[m.proj] || 0)} DMG`);
            logEvent(`${name} defeated (${m.proj}): ${fmtTok(projDamage[m.proj] || 0)} damage in all`);
        } else {
            banner(`${name} FLEES`);
            logEvent(`${name} flees: a busier project takes the place of ${m.proj}`);
        }
        yield* tween(1.1, t => { m.sweep = t; });
        S.monsters.splice(S.monsters.indexOf(m), 1);
        relayoutMonsters();
    });
}

/* The 4 projects with the most sessions on the field get a monster; on a
 * tie the one that has it keeps it, then the one seen first. */
const projSeen = {};
let seenOrder = 0;
function rebalanceMonsters() {
    const count = {};
    for (const p of S.players) if (!p.leaving && !p.bench) count[p.proj] = (count[p.proj] || 0) + 1;
    const has = proj => (monsterOf(proj) ? 1 : 0);
    const ranked = Object.keys(count).sort((a, b) => count[b] - count[a] || has(b) - has(a) || projSeen[a] - projSeen[b]);
    const want = new Set(ranked.slice(0, 4));
    for (const m of S.monsters) if (!m.leaving && !want.has(m.proj)) removeMonster(m, count[m.proj] ? 'flees' : 'defeated');
    for (const proj of ranked.slice(0, 4)) if (!monsterOf(proj)) addMonster(proj);
}

const MAX_PARTY = 8;
function freeSlot() {
    for (let s = 0; s < MAX_PARTY; s++) if (!S.players.some(p => p.slot === s && !p.leaving && !p.bench)) return s;
    return -1;
}

/* Claude Code sessions are mages, Codex sessions fighters. A new session
 * takes the class of its group with the fewest players, and the palette
 * swap when that class is already on the field. */
function pickClass(tool) {
    const kind = tool === 'codex' ? 'fighter' : 'mage';
    const live = S.players.filter(p => !p.leaving);
    const ids = CLS.map((k, i) => i).filter(i => CLS[i].c.kind === kind);
    const n = i => live.filter(p => p.cls === i).length;
    const least = Math.min(...ids.map(n));
    const pick = ids.filter(i => n(i) === least);
    const cls = pick[(Math.random() * pick.length) | 0];
    const used = new Set(live.filter(p => p.cls === cls).map(p => p.variant));
    const variant = !used.has(0) ? 0 : !used.has(1) ? 1 : least % 2;
    return { cls, variant };
}
/* a name of that class nobody on the field has */
function pickName(cls) {
    const pool = CLS[cls].c.names, used = new Set(S.players.filter(p => !p.leaving).map(p => p.name));
    const free = pool.filter(nm => !used.has(nm));
    if (free.length) return free[(Math.random() * free.length) | 0];
    return pool[0].slice(0, 5) + (S.players.length % 10);
}

function join({ id, proj, tool = 'claude', remote = false }) {
    const { cls, variant } = pickClass(tool);
    const p = new Actor();
    Object.assign(p, {
        id, proj, tool, remote, cls, variant, name: pickName(cls), slot: -1, bench: true,
        state: 'work', hp: RULES.hpMax, ko: false, atb: Math.random() * 0.5,
        dealt: 0, pending: 0, castWait: 0, nextTool: null, hurt: 0, white: 0, leaving: false, fwd: 0,
    });
    if (projSeen[proj] === undefined) projSeen[proj] = seenOrder++;
    S.players.push(p);
    logEvent(`${p.name} (${CLS[cls].c.job}) joins: a ${tool === 'claude' ? 'Claude Code' : 'Codex'}${remote ? ' remote' : ''} session in ${proj}`);
    if (!seat(p)) logEvent(`${p.name} waits off the field: the party is full`);
    return p;
}

/* a waiting session takes a free slot and runs in */
function seat(p) {
    const slot = freeSlot();
    if (slot < 0) return false;
    Object.assign(p, { slot, bench: false, hx: PARTY_SLOTS[slot][0], hy: PARTY_SLOTS[slot][1] });
    p.x = W + 30; p.y = p.hy; p.pose = null;
    p.push(() => walkTo(p, p.hx, p.hy, 0.9));
    rebalanceMonsters();
    return true;
}

function leave(p) {
    if (p.leaving) return;
    p.leaving = true;
    p.queue.length = 0;
    logEvent(`${p.name} leaves (session ended)`);
    const gone = () => {
        S.players.splice(S.players.indexOf(p), 1);
        if (S.selected === p) S.selected = null;
        const next = S.players.find(q => q.bench && !q.leaving);
        if (!next || !seat(next)) rebalanceMonsters();
    };
    if (p.bench) { gone(); return; }
    p.push(function* () {
        p.ko = false;
        yield* walkTo(p, W + 40, p.y, 0.8, true);
        gone();
    });
}

function* walkTo(p, x, y, s, away = false) {
    const x0 = p.x, y0 = p.y;
    p.walking = away ? 'R' : 'L';
    yield* tween(s, t => { p.x = x0 + (x - x0) * t; p.y = y0 + (y - y0) * t; });
    p.walking = null;
}

/* ------------------------------------------------------------ player actions */

/* a session whose project has no monster fights the nearest one */
function target(p) {
    const own = monsterOf(p.proj);
    if (own) return own;
    let best = null, bd = 1e9;
    for (const m of S.monsters) {
        if (m.leaving) continue;
        const d = (m.tx - p.hx) ** 2 + (m.ty - p.hy) ** 2;
        if (d < bd) { bd = d; best = m; }
    }
    return best;
}
function monCenter(m) { const img = MONS[m.kind].f.idle; return [m.x, m.y - img.h / 2]; }
function plCenter(p) { return [p.x - 2, p.y - 14]; }

function* actAttack(p, dmg) {
    const m = target(p);
    if (!m) return;
    yield* walkTo(p, p.hx - 16, p.hy, 0.18);
    p.pose = 'attack';
    yield* wait(0.06);
    const weapon = CLS[p.cls].c.weapon;
    fxStrike(m, SLASH_OF[weapon] || 'flurry');
    m.white = 0.25; m.shake = 0.25;
    p.dealt += dmg; projDamage[m.proj] = (projDamage[m.proj] || 0) + dmg;
    const [cx, cy] = monCenter(m);
    popNum(cx, cy - 6, fmtTok(dmg), COL.white);
    yield* wait(0.4);
    p.pose = null;
    yield* walkTo(p, p.hx, p.hy, 0.18);
}

/* Each class's special move. Mages (Claude Code) cast the tool's spell and
 * keep their special for a subagent; fighters (Codex) answer every tool call
 * with their special. Like the spells, they do no damage. */
const SPECIALS = {
    sorcerer: { name: 'FLARE', fx: 'flare' },
    cleric: { name: 'HOLY', fx: 'holy' },
    spellsword: { name: 'DOUBLECAST', fx: 'doublecast' },
    summoner: { name: 'SUMMON', fx: 'summon' },
    warrior: { name: 'CROSS CUT', fx: 'cross' },
    thief: { name: 'STEAL', fx: 'steal' },
    monk: { name: 'CHI BURST', fx: 'chiburst' },
    lancer: { name: 'JUMP', fx: 'jump' },
};
const SPELL_OF = { fire: 'FIRE', ice: 'BLIZZARD', bolt: 'THUNDER', gale: 'GALE' };
const LOOT = ['POTION', 'GREAT POTION', 'ETHER', 'ELIXIR', 'PHOENIX PLUME', 'TENT', 'MAX POTION', 'REMEDY', 'GRAND ELIXIR'];
function moveFor(p, kind) {
    const c = CLS[p.cls].c;
    return c.kind === 'fighter' || kind === 'agent' ? SPECIALS[c.id] : TOOLS[kind];
}

function* actMove(p, kind) {
    const T = TOOLS[kind], m = target(p);
    if (!T) return;
    const mv = moveFor(p, kind), special = mv !== T;
    if (mv.fx === 'cross' || mv.fx === 'steal') { yield* actDash(p, m, mv, T); return; }
    if (mv.fx === 'jump') { yield* actJump(p, m, T); return; }
    yield* walkTo(p, p.hx - 10, p.hy, 0.14);
    p.pose = mv.fx === 'chiburst' ? 'attack' : 'cast';
    let name = mv.name, sub = '', pair = null;
    if (mv.fx === 'summon') { name = SUMMONS[(Math.random() * SUMMONS.length) | 0]; sub = 'SUMMON'; }
    if (mv.fx === 'doublecast') {
        const pool = Object.keys(SPELL_OF);
        pair = [pool.splice((Math.random() * pool.length) | 0, 1)[0], pool[(Math.random() * pool.length) | 0]];
        sub = `${SPELL_OF[pair[0]]} + ${SPELL_OF[pair[1]]}`;
    }
    banner(name, sub, special ? 2 : 1);
    logEvent(`${p.name} ${mv.fx === 'summon' ? 'summons' : special ? 'uses' : 'casts'} ${name}${pair ? ` (${sub})` : ''} for ${T.label}`);
    if (mv.fx !== 'chiburst') S.fx.push(fxChant(p));
    yield* wait(mv.fx === 'chiburst' ? 0.15 : 0.45);
    if (mv.fx === 'protect') S.fx.push(fxProtect(p));
    else if (m) {
        if (mv.fx === 'summon') S.fx.push(fxSummon(m, name));
        else if (mv.fx === 'chiburst') S.fx.push(fxChiBurst(p, m));
        else if (pair) { S.fx.push(FX_ON_MONSTER[pair[0]](m)); yield* wait(0.55); S.fx.push(FX_ON_MONSTER[pair[1]](m)); }
        else S.fx.push(FX_ON_MONSTER[mv.fx](m));
    }
    yield* wait(mv.fx === 'summon' ? 1.2 : mv.fx === 'flare' || mv.fx === 'holy' ? 0.9 : 0.5);
    if (m && mv.fx !== 'protect') m.white = 0.2;
    yield* wait(0.25);
    p.pose = null;
    yield* walkTo(p, p.hx, p.hy, 0.14);
}

/* the Warrior's cross cut and the Thief's steal: a dash to the monster and back */
function* actDash(p, m, mv, T) {
    if (!m) return;
    const f = MONS[m.kind].f.idle;
    banner(mv.name, '', 2);
    yield* walkTo(p, m.x + f.w / 2 + 12, m.y, 0.22);
    p.pose = 'attack';
    if (mv.fx === 'cross') {
        logEvent(`${p.name} uses CROSS CUT for ${T.label}`);
        S.fx.push(fxCross(m));
        yield* wait(0.12);
        m.white = 0.5; m.shake = 0.35;
        yield* wait(0.4);
    } else {
        const loot = Math.random() < 0.15 ? null : LOOT[(Math.random() * LOOT.length) | 0];
        S.fx.push(fxSparkle({ x: m.x + 6, y: m.y - 4 }, COL.gold));
        yield* wait(0.15);
        banner('STEAL', loot ? `GOT ${loot}!` : 'MISSED!', 2);
        logEvent(`${p.name} uses STEAL for ${T.label}: ${loot ? `got ${loot.toLowerCase()}` : 'missed'}`);
        yield* wait(0.35);
    }
    p.pose = null;
    yield* walkTo(p, p.hx, p.hy, 0.3, true);
}

/* the Lancer's jump: up off the screen, a shadow on the monster, down on it, back */
function* actJump(p, m, T) {
    if (!m) return;
    banner('JUMP', '', 2);
    logEvent(`${p.name} uses JUMP for ${T.label}`);
    p.air = true;
    p.pose = 'kneel';
    yield* wait(0.15);
    p.pose = 'ready';
    const y0 = p.y;
    yield* tween(0.22, t => { p.y = y0 - t * t * 200; });
    p.hidden = true;
    S.fx.push(fxShadow(m));
    yield* wait(0.6);
    p.hidden = false; p.diving = true; p.pose = 'kneel'; p.x = m.x + 6;
    const top = m.y - 160;
    yield* tween(0.14, t => { p.y = top + (m.y + 2 - top) * t; });
    p.diving = false;
    fxStrike(m, 'bonk');
    m.white = 0.6; m.shake = 0.4; S.flash = Math.max(S.flash, 0.25);
    yield* wait(0.25);
    p.pose = 'ready';
    yield* tween(0.18, t => { p.y = m.y + 2 - t * 170; });
    p.x = p.hx;
    yield* tween(0.3, t => { p.y = p.hy - 170 + 170 * t; });
    p.pose = null; p.air = false;
}

function* actRevive(p) {
    banner('PHOENIX PLUME', '', 2);
    logEvent(`PHOENIX PLUME on ${p.name}: the session woke up`);
    S.fx.push(fxFeather(p));
    yield* wait(1.0);
    S.fx.push(fxSparkle(p, COL.gold));
    p.white = 0.6;
    yield* wait(0.3);
    p.ko = false; p.hp = RULES.hpMax;
    popNum(p.x, p.y - 34, String(RULES.hpMax), COL.green);
    yield* wait(0.4);
}

function* actCure(p) {
    const gain = RULES.hpMax - p.hp;
    banner('CURE');
    S.fx.push(fxSparkle(p, COL.green));
    yield* wait(0.5);
    p.hp = RULES.hpMax;
    popNum(p.x, p.y - 34, String(gain), COL.green);
    yield* wait(0.3);
}

/* ------------------------------------------------------------ monster attack */

function* monAttack(m, p) {
    const M = MONS[m.kind].m;
    banner(M.attack);
    logEvent(`${M.name} uses ${M.attack} on ${p.name} (idle)`);
    for (let k = 0; k < 2; k++) { m.white = 0.7; yield* wait(0.09); m.white = 0; yield* wait(0.07); }
    m.frame = 'attack';
    const fx = (FX_MON[M.fx] || FX_MON.punch)(m, p);
    S.fx.push(fx);
    yield* wait(fx.impact);
    const per = RULES.hpMax * RULES.monsterSeconds / RULES.koSeconds;
    const dmg = Math.max(1, Math.round(per * (0.75 + Math.random() * 0.5)));
    p.hp = Math.max(0, p.hp - dmg);
    p.hurt = 0.35;
    popNum(p.x, p.y - 34, String(dmg), COL.white);
    if (p.hp === 0) { p.ko = true; logEvent(`${p.name} is knocked out (idle too long)`); }
    yield* wait(0.35);
    m.frame = 'idle';
}

/* ------------------------------------------------------------ effects */

function fxOf(dur, draw, extra = {}) { return Object.assign({ t: 0, dur, draw }, extra); }

function flame(cx, by, h, seed) {
    const r = rng(seed);
    const cols = [hex('#a82018'), hex('#e8641c'), hex('#f8c840'), hex('#fff4c0')];
    for (let k = 0; k < h; k++) {
        const t = k / h;
        const hw = Math.max(0, Math.sin(Math.PI * Math.pow(1 - t, 0.8)) * h * 0.32 + (r() - 0.5) * 1.6);
        const sway = Math.sin(t * 4 + seed) * t * 2;
        for (let i = -Math.ceil(hw); i <= Math.ceil(hw); i++) {
            const e = Math.abs(i) / (hw + 0.01);
            const lvl = Math.min(3, Math.max(0, Math.floor((1 - e) * 3.2 - t * 1.5 + 0.6)));
            put(cx + i + sway, by - k, cols[lvl]);
        }
    }
}

const FX_ON_MONSTER = {
    fire: m => fxOf(1.0, (f, t) => {
        const [cx, cy] = monCenter(m), img = MONS[m.kind].f.idle;
        const offs = [[-img.w * 0.25, 4], [img.w * 0.2, 6], [0, -2]];
        offs.forEach(([ox, oy], i) => {
            const tt = Math.max(0, Math.min(1, t * 1.6 - i * 0.18));
            const h = Math.sin(tt * Math.PI) * 18;
            if (h > 1) flame(cx + ox, cy + img.h * 0.35 + oy, h | 0, (S.time * 30 | 0) + i * 7);
        });
    }),
    ice: m => {
        const r = rng(Math.random() * 1e9 | 0), shards = [];
        for (let k = 0; k < 6; k++) shards.push([(r() - 0.5) * 34, (r() - 0.5) * 30, 3 + r() * 4]);
        const parts = [];
        for (let k = 0; k < 18; k++) parts.push([r() * Math.PI * 2, 10 + r() * 30]);
        return fxOf(1.0, (f, t) => {
            const [cx, cy] = monCenter(m);
            const ice = hex('#9ae8ff'), deep = hex('#2a6ab8'), wht = COL.white;
            if (t < 0.6) {
                shards.forEach(([ox, oy, s], i) => {
                    const g = Math.min(1, t * 4 - i * 0.12);
                    if (g <= 0) return;
                    const sz = Math.round(s * g);
                    for (let j = -sz * 2; j <= sz * 2; j++) {
                        const w = sz - Math.abs(j) / 2;
                        for (let i2 = -w; i2 <= w; i2++) put(cx + ox + i2, cy + oy + j, Math.abs(i2) >= w - 0.5 ? deep : (i2 < 0 ? wht : ice));
                    }
                });
            } else {
                const k = (t - 0.6) / 0.4;
                parts.forEach(([a, d]) => { const x = cx + Math.cos(a) * d * k * 1.2, y = cy + Math.sin(a) * d * k + k * k * 10; rect(x, y, 2, 2, k < 0.5 ? wht : ice); });
            }
        });
    },
    bolt: m => {
        const r = rng(Math.random() * 1e9 | 0);
        return fxOf(0.7, (f, t) => {
            if ((t * 10 | 0) % 3 === 2) return;
            const [cx, cy] = monCenter(m);
            let x = cx + (r() - 0.5) * 4, y = 0;
            const pts = [[x, y]];
            const rr = rng((t * 12 | 0) + 99);
            for (let k = 1; k <= 7; k++) { x = cx + (rr() - 0.5) * 18 * (1 - k / 8); y = cy * k / 7; pts.push([x, y]); }
            for (let k = 1; k < pts.length; k++) {
                line(pts[k - 1][0], pts[k - 1][1], pts[k][0], pts[k][1], hex('#a080ff'), 4);
                line(pts[k - 1][0], pts[k - 1][1], pts[k][0], pts[k][1], hex('#f8f070'), 2);
                line(pts[k - 1][0], pts[k - 1][1], pts[k][0], pts[k][1], COL.white, 1);
            }
            if (t < 0.15) S.flash = Math.max(S.flash, 0.35);
        });
    },
    scan: m => fxOf(1.0, (f, t) => {
        const img = MONS[m.kind].f.idle, [cx, cy] = monCenter(m), g = hex('#7cf8a0');
        const k = Math.min(1, t / 0.35), hw = img.w / 2 + 4 + (1 - k) * 30, hh = img.h / 2 + 4 + (1 - k) * 24;
        const L = 6;
        for (const [sx, sy] of [[-1, -1], [1, -1], [-1, 1], [1, 1]]) {
            const x = cx + sx * hw, y = cy + sy * hh;
            line(x, y, x - sx * L, y, g); line(x, y, x, y - sy * L, g);
        }
        if (t > 0.35) {
            const yy = cy - hh + (hh * 2) * ((t - 0.35) / 0.65);
            line(cx - hw + 1, yy, cx + hw - 1, yy, g);
            for (let y = cy - hh; y < yy; y += 2) for (let x = cx - hw; x < cx + hw; x += 2) if (BAYER[((y | 0) & 3) * 4 + ((x | 0) & 3)] < 3) blend(x, y, g, 0.5);
        }
    }),
    gale: m => fxOf(1.0, (f, t) => {
        const [cx, cy] = monCenter(m);
        for (let k = 0; k < 16; k++) {
            const a = k / 16 * Math.PI * 2 + t * 12, rr = 26 * (1 - t) + 6 + (k % 3) * 3;
            const x = cx + Math.cos(a) * rr, y = cy + Math.sin(a) * rr * 0.6 - t * 6;
            rect(x, y, 2, 1, k % 2 ? hex('#c8f8d0') : hex('#58d080'));
            put(x - Math.cos(a + 1.6) * 2, y - Math.sin(a + 1.6), hex('#58d080'));
        }
    }),
};

function fxSummon(m, name) {
    const colors = { 'DRAGON KING': ['#fff4c0', '#f89040'], SALAMANDER: ['#fff0a0', '#e84020'], 'FROST QUEEN': ['#ffffff', '#58c8f8'], RAIJIN: ['#ffffa0', '#a080ff'] }[name] || ['#fff', '#f80'];
    const c0 = hex(colors[0]), c1 = hex(colors[1]);
    return fxOf(1.8, (f, t) => {
        const [cx, cy] = monCenter(m);
        S.dim = Math.max(S.dim, t < 0.2 ? t / 0.2 * 0.55 : t > 0.8 ? Math.max(0, (1 - t) / 0.2) * 0.55 : 0.55);
        if (t > 0.1 && t < 0.9) {
            const k = Math.min(1, (t - 0.1) / 0.25), rx = 34 * k, ry = 10 * k;
            ring(cx, m.y, rx, ry, c1); ring(cx, m.y, rx - 4, ry - 1.5, c0);
            for (let n = 0; n < 8; n++) { const a = n / 8 * Math.PI * 2 + t * 4; rect(cx + Math.cos(a) * (rx - 2), m.y + Math.sin(a) * (ry - 0.7), 2, 2, c0); }
        }
        if (t > 0.45 && t < 0.85) {
            const k = (t - 0.45) / 0.4, x0 = W - 30, y0 = -10;
            const x1 = x0 + (cx - x0) * Math.min(1, k * 1.5), y1 = y0 + (cy - y0) * Math.min(1, k * 1.5);
            line(x0, y0, x1, y1, c1, 9); line(x0, y0, x1, y1, c0, 5); line(x0, y0, x1, y1, COL.white, 2);
        }
        if (t > 0.7) {
            const k = (t - 0.7) / 0.3;
            ring(cx, cy, 10 + k * 60, 6 + k * 34, c0); ring(cx, cy, 6 + k * 46, 4 + k * 26, c1);
            if (k < 0.25) S.flash = Math.max(S.flash, 0.6);
        }
    });
}

function fxChant(p) {
    return fxOf(0.5, (f, t) => {
        const hx = p.x - 12, hy = p.y - 30;
        for (let k = 0; k < 8; k++) {
            const a = k / 8 * Math.PI * 2 + t * 6, rr = 16 * (1 - t);
            put(hx + Math.cos(a) * rr, hy + Math.sin(a) * rr, k % 2 ? COL.white : COL.gold);
        }
        p.white = 0.25 * Math.sin(t * Math.PI);
    });
}
function fxProtect(p) {
    return fxOf(1.0, (f, t) => {
        if ((t * 14 | 0) % 2) return;
        const c = hex('#f8f0a0');
        ring(p.x - 1, p.y - 16, 16, 20, c);
        ring(p.x - 1, p.y - 16, 14, 18, mix(c, COL.white, 0.6));
    });
}
function fxSparkle(p, c) {
    const r = rng(Math.random() * 1e9 | 0), pts = [];
    for (let k = 0; k < 12; k++) pts.push([(r() - 0.5) * 22, r() * 0.5, r()]);
    return fxOf(0.9, (f, t) => {
        pts.forEach(([ox, d, s]) => {
            const k = (t - d) / (1 - d);
            if (k < 0 || k > 1) return;
            const x = p.x - 2 + ox, y = p.y - 4 - k * 34;
            const cc = s > 0.5 ? COL.white : c;
            put(x, y, cc); put(x - 1, y, c); put(x + 1, y, c); put(x, y - 1, c); put(x, y + 1, c);
        });
    });
}
function fxFeather(p) {
    return fxOf(1.0, (f, t) => {
        const x = p.x - 4 + Math.sin(t * 9) * 8, y = -10 + (p.y - 16) * t;
        blit(ICON.feather, x, y);
    });
}

/* Sorcerer: a ring closes on the monster, then it blows up */
function ditherDisc(cx, cy, r, c, vis) {
    for (let j = -r; j <= r; j++) for (let i = -r; i <= r; i++) {
        if (i * i + j * j > r * r + r * 0.6) continue;
        const x = (cx + i) | 0, y = (cy + j) | 0;
        if (vis < 1 && BAYER[(y & 3) * 4 + (x & 3)] >= vis * 16) continue;
        put(x, y, c);
    }
}
FX_ON_MONSTER.flare = m => fxOf(1.2, (f, t) => {
    const [cx, cy] = monCenter(m);
    if (t < 0.3) {
        const k = t / 0.3, r = 34 * (1 - k) + 2;
        ring(cx, cy, r, r, COL.white); ring(cx, cy, r * 0.75, r * 0.75, hex('#f8a040'));
        disc(cx, cy, 1 + k * 2 | 0, COL.white);
        return;
    }
    const k = (t - 0.3) / 0.7, R = (6 + Math.sqrt(k) * 22) | 0, vis = k < 0.6 ? 1 : (1 - k) / 0.4;
    if (k < 0.12) S.flash = Math.max(S.flash, 0.5);
    ditherDisc(cx, cy, R, hex('#a82018'), vis);
    ditherDisc(cx, cy, R * 0.8 | 0, hex('#e8641c'), vis);
    ditherDisc(cx, cy, R * 0.6 | 0, hex('#f8c840'), vis);
    ditherDisc(cx, cy, R * 0.35 | 0, hex('#fff4c0'), vis);
    ring(cx, cy, R + 6 + k * 14, (R + 6 + k * 14) * 0.6, hex('#fff4c0'));
});
/* Cleric: orbs of light circle in, then a pillar of light comes down */
FX_ON_MONSTER.holy = m => fxOf(1.3, (f, t) => {
    const [cx, cy] = monCenter(m), gold = hex('#f8f0a0');
    if (t < 0.6) {
        const k = t / 0.6, r = 34 * (1 - k) + 4;
        for (let n = 0; n < 6; n++) {
            const a = n / 6 * Math.PI * 2 + k * 6, x = cx + Math.cos(a) * r, y = cy + Math.sin(a) * r * 0.7;
            disc(x, y, 3, gold); disc(x, y, 1, COL.white);
        }
        return;
    }
    const k = (t - 0.6) / 0.4, w = (18 * (1 - k) + 2) | 0;
    if (k < 0.15) S.flash = Math.max(S.flash, 0.45);
    for (let y = 0; y < cy + 14; y++) {
        rect(cx - w / 2, y, w, 1, (y + (S.time * 60 | 0)) % 6 < 3 ? gold : COL.white);
        rect(cx - w / 4, y, w / 2, 1, COL.white);
    }
    ring(cx, cy + 12, w + 8, 4, gold);
});
/* Warrior: two cuts that cross on the monster */
function fxCross(m) {
    return fxOf(0.55, (f, t) => {
        if (t > 0.6 && (t * 20 | 0) % 2) return;
        const [cx, cy] = monCenter(m), L = 40, glow = hex('#9ad8ff');
        const k1 = Math.min(1, t / 0.2), k2 = Math.max(0, Math.min(1, (t - 0.18) / 0.2));
        line(cx + L / 2, cy - L / 2, cx + L / 2 - L * k1, cy - L / 2 + L * k1, glow, 4);
        line(cx + L / 2, cy - L / 2, cx + L / 2 - L * k1, cy - L / 2 + L * k1, COL.white, 2);
        if (k2 > 0) {
            line(cx - L / 2, cy - L / 2, cx - L / 2 + L * k2, cy - L / 2 + L * k2, glow, 4);
            line(cx - L / 2, cy - L / 2, cx - L / 2 + L * k2, cy - L / 2 + L * k2, COL.white, 2);
        }
    });
}
/* Lancer: his shadow grows on the monster while he is up in the air */
function fxShadow(m) {
    return fxOf(0.6, (f, t) => {
        ellipseShade(Math.round(m.x + 6), Math.round(m.y), (4 + t * 10) | 0, (1 + t * 2) | 0, 0.55);
    });
}
/* Monk: a ball of aura from the fists */
function fxChiBurst(p, m) {
    return fxOf(0.75, (f, t) => {
        const [ax, ay] = [p.x - 16, p.y - 18], [bx, by] = monCenter(m);
        if (t < 0.45) {
            const k = t / 0.45;
            for (let n = 4; n >= 0; n--) {
                const kk = Math.max(0, k - n * 0.05), x = ax + (bx - ax) * kk, y = ay + (by - ay) * kk;
                disc(x, y, n ? 4 - n * 0.6 | 0 : 6, n ? hex('#3a7af8') : hex('#3a7af8'));
                if (!n) { disc(x, y, 4, hex('#a8d8ff')); disc(x, y, 2, COL.white); }
            }
            return;
        }
        const k = (t - 0.45) / 0.55;
        ring(bx, by, 4 + k * 22, 4 + k * 16, hex('#a8d8ff'));
        ring(bx, by, 2 + k * 14, 2 + k * 10, COL.white);
        if (k < 0.3) disc(bx, by, (6 * (1 - k)) | 0, COL.white);
    });
}

/* the player's physical hit on the monster */
function fxStrike(m, kind) {
    S.fx.push(fxOf(0.3, (f, t) => {
        const [cx, cy] = monCenter(m), wht = COL.white, gl = hex('#c8d8ff');
        const k = Math.min(1, t / 0.15);
        if (kind === 'slash' || kind === 'slash2') {
            const n = kind === 'slash' ? 1 : 2;
            for (let i = 0; i < n; i++) {
                const ox = i * 8 - (n - 1) * 4, L = 26;
                const x0 = cx + ox + L / 2, y0 = cy - L / 2, x1 = x0 - L * k, y1 = y0 + L * k;
                line(x0, y0, x1, y1, gl, 3); line(x0, y0, x1, y1, wht, 1);
            }
        } else if (kind === 'thrust') {
            const x0 = cx + 26, x1 = cx + 26 - 40 * k;
            line(x0, cy, x1, cy, gl, 3); line(x0, cy, x1, cy, wht, 1);
            if (k === 1) { for (let a = 0; a < 8; a++) { const an = a / 8 * Math.PI * 2; line(x1, cy, x1 + Math.cos(an) * 5, cy + Math.sin(an) * 5, wht); } }
        } else {
            const pts = kind === 'flurry' ? [[-6, -4], [5, 2], [-2, 7]] : [[0, 0]];
            pts.forEach(([ox, oy], i) => {
                const kk = t * 3.3 - i * 0.6;
                if (kk < 0 || kk > 1) return;
                const rr = 3 + kk * 6;
                for (let a = 0; a < 8; a++) { const an = a / 8 * Math.PI * 2; line(cx + ox + Math.cos(an) * rr * 0.4, cy + oy + Math.sin(an) * rr * 0.4, cx + ox + Math.cos(an) * rr, cy + oy + Math.sin(an) * rr, wht); }
            });
        }
    }));
}

/* the monsters' attacks: from the monster to an idle player */
function travel(m, p, k) { const [ax, ay] = monCenter(m), [bx, by] = plCenter(p); return [ax + (bx - ax) * k, ay + (by - ay) * k]; }
const FX_MON = {
    punch: (m, p) => fxOf(0.5, (f, t) => {
        if (t < 0.15) return;
        const [cx, cy] = plCenter(p), k = (t - 0.15) / 0.35, rr = 3 + k * 8;
        for (let a = 0; a < 8; a++) { const an = a / 8 * Math.PI * 2 + 0.3; line(cx + Math.cos(an) * rr * 0.4, cy + Math.sin(an) * rr * 0.4, cx + Math.cos(an) * rr, cy + Math.sin(an) * rr, COL.white); }
    }, { impact: 0.15 }),
    blaze: (m, p) => fxOf(1.0, (f, t) => {
        if (t < 0.4) {
            const [x, y] = travel(m, p, t / 0.4);
            disc(x, y - Math.sin(t / 0.4 * Math.PI) * 14, 5, hex('#e8641c')); disc(x, y - Math.sin(t / 0.4 * Math.PI) * 14, 3, hex('#f8c840')); disc(x, y - Math.sin(t / 0.4 * Math.PI) * 14, 1, hex('#fff4c0'));
        } else {
            const [cx, cy] = plCenter(p), k = (t - 0.4) / 0.6, h = Math.sin(k * Math.PI) * 16;
            flame(cx - 5, cy + 12, h | 0, S.time * 30 | 0); flame(cx + 4, cy + 14, (h * 0.8) | 0, (S.time * 30 | 0) + 5);
        }
    }, { impact: 0.45 }),
    needles: (m, p) => {
        const r = rng(Math.random() * 1e9 | 0), ns = [];
        for (let k = 0; k < 16; k++) ns.push([r() * 0.35, (r() - 0.5) * 16, (r() - 0.5) * 20]);
        return fxOf(0.8, (f, t) => {
            ns.forEach(([d, ox, oy]) => {
                const k = (t - d) / 0.4;
                if (k < 0 || k > 1) return;
                const [x, y] = travel(m, p, k);
                line(x + ox * k, y + oy * k, x + ox * k - 3, y + oy * k, hex('#e8f8c0'));
                put(x + ox * k, y + oy * k, hex('#58a040'));
            });
        });
    },
    knife: (m, p) => fxOf(0.8, (f, t) => {
        const [mx, my] = monCenter(m);
        if (t < 0.3) { const s = Math.sin(t / 0.3 * Math.PI) * 4; line(mx + 8 - s, my - 4, mx + 8 + s, my - 4, COL.white); line(mx + 8, my - 4 - s, mx + 8, my - 4 + s, COL.white); return; }
        const [cx, cy] = plCenter(p), k = Math.min(1, (t - 0.3) / 0.15);
        line(cx + 10, cy - 12, cx + 10 - 22 * k, cy - 12 + 24 * k, hex('#ff5a50'), 3);
        line(cx + 10, cy - 12, cx + 10 - 22 * k, cy - 12 + 24 * k, COL.white, 1);
    }, { impact: 0.4 }),
    doom: (m, p) => fxOf(1.0, (f, t) => {
        const [cx, cy] = plCenter(p);
        const k = Math.sin(Math.min(1, t / 0.8) * Math.PI), rr = (k * 12) | 0;
        if (rr > 0) { disc(cx, cy, rr, hex('#2a1040')); disc(cx, cy, Math.max(0, rr - 4), hex('#6a2a98')); ring(cx, cy, rr + 1, rr + 1, hex('#e080ff')); }
        if (t > 0.3 && t < 0.75 && (t * 16 | 0) % 2) blit(ICON.skull, cx - 3, cy - 3);
    }, { impact: 0.65 }),
    breath: (m, p) => {
        const r = rng(Math.random() * 1e9 | 0), puffs = [];
        for (let k = 0; k < 12; k++) puffs.push([r() * 0.4, (r() - 0.5) * 18, (r() - 0.5) * 14, r() < 0.5]);
        return fxOf(1.1, (f, t) => {
            puffs.forEach(([d, ox, oy, purple]) => {
                const k = Math.min(1, (t - d) / 0.5);
                if (k < 0) return;
                const [x, y] = travel(m, p, k);
                const rr = (2 + k * 4) | 0;
                disc(x + ox * k, y + oy * k, rr, purple ? hex('#a868c8') : hex('#78c048'));
                disc(x + ox * k - 1, y + oy * k - 1, Math.max(0, rr - 2), purple ? hex('#d8a8f0') : hex('#b8f088'));
            });
        });
    },
    blaster: (m, p) => fxOf(0.8, (f, t) => {
        for (let n = 0; n < 5; n++) {
            const k = (t - n * 0.07) / 0.5;
            if (k < 0 || k > 1) continue;
            const [x, y] = travel(m, p, k);
            ring(x, y, 2 + k * 5, 4 + k * 9, n % 2 ? hex('#f8f070') : COL.white);
        }
    }, { impact: 0.5 }),
    meteor: (m, p) => {
        const r = rng(Math.random() * 1e9 | 0), rocks = [];
        for (let k = 0; k < 4; k++) rocks.push([k * 0.12, (r() - 0.5) * 20, (r() - 0.5) * 10]);
        return fxOf(1.1, (f, t) => {
            const [cx, cy] = plCenter(p);
            rocks.forEach(([d, ox, oy]) => {
                const k = (t - d) / 0.5;
                if (k < 0) return;
                if (k > 1) { if (k < 1.4) { const rr = (k - 1) * 20; ring(cx + ox, cy + oy + 8, rr, rr * 0.5, hex('#f8a040')); } return; }
                const x = cx + ox - 70 * (1 - k), y = cy + oy + 8 - 110 * (1 - k);
                const trail = [hex('#fff4c0'), hex('#f8c840'), hex('#f8a040'), hex('#e8641c'), hex('#a82018')];
                for (let tr = 1; tr < 16; tr++) { const c = trail[Math.min(4, tr / 3 | 0)]; const w = Math.max(1, 4 - tr / 4 | 0); rect(x - tr * 0.64 - w / 2, y - tr - w / 2, w, w, c); }
                blit(ICON.rock, x - 4, y - 4);
            });
        });
    },
};
for (const k of Object.keys(FX_MON)) {
    const f = FX_MON[k], imp = { punch: 0.15, blaze: 0.45, needles: 0.5, knife: 0.4, doom: 0.65, breath: 0.65, blaster: 0.5, meteor: 0.65 }[k];
    FX_MON[k] = (m, p) => { const e = f(m, p); if (e.impact === undefined) e.impact = imp; return e; };
}

/* ------------------------------------------------------------ host events */

function state(p, st) {
    if (!p || p.leaving) return;
    const prev = p.state;
    p.state = st;
    if (st === 'work' || st === 'tool') {
        p.state = 'work';
        if (p.ko) p.push(() => actRevive(p));
        else if (prev === 'idle' && p.hp < RULES.hpMax) p.push(() => actCure(p));
        if (prev !== 'work') logEvent(`${p.name} gets to work`);
    } else if (st === 'ask' || st === 'perm') {
        logEvent(`${p.name} raises a hand: ${st === 'ask' ? 'has a question' : 'needs your OK'}`);
        if (p.ko) p.push(() => actRevive(p));
    } else if (st === 'idle') {
        logEvent(`${p.name} is idle: the monster will attack`);
    }
}
function tool(p, kind) {
    if (!p || p.leaving || p.ko || p.state !== 'work') return;
    p.nextTool = kind;
}
function tokens(p, n) { if (p && !p.leaving && !p.ko) p.pending += n; }

/* ------------------------------------------------------------ tick */

function update(dt) {
    S.time += dt;
    for (const p of S.players) {
        if (p.bench) continue;
        p.hurt = Math.max(0, p.hurt - dt);
        p.white = Math.max(0, p.white - dt * 2);
        p.castWait = Math.max(0, p.castWait - dt);
        if (!p.leaving && !p.ko && p.state === 'work') {
            p.atb = Math.min(1, p.atb + dt / RULES.atbSeconds);
            if (!p.busy()) {
                if (p.atb >= 1 && p.pending > 0) { const d = p.pending; p.pending = 0; p.atb = 0; p.push(() => actAttack(p, d)); }
                else if (p.nextTool && p.castWait === 0) { const k = p.nextTool; p.nextTool = null; p.castWait = RULES.castGap; p.push(() => actMove(p, k)); }
            }
        } else if (p.state !== 'ask' && p.state !== 'perm') {
            p.atb = Math.max(0, p.atb - dt / 2);
        }
        const wantFwd = (p.state === 'ask' || p.state === 'perm') && !p.ko && !p.leaving ? 1 : 0;
        p.fwd += (wantFwd - p.fwd) * Math.min(1, dt * 8);
        p.step(dt);
    }
    for (const m of S.monsters) {
        m.white = Math.max(0, m.white - dt * 2);
        m.shake = Math.max(0, m.shake - dt);
        if (m.tx !== undefined) { m.x += (m.tx - m.x) * Math.min(1, dt * 4); m.y += (m.ty - m.y) * Math.min(1, dt * 4); }
        if (!m.leaving && m.vis >= 1) {
            m.timer -= dt * idleScale;
            if (m.timer <= 0 && !m.busy()) {
                m.timer = RULES.monsterSeconds * (0.8 + Math.random() * 0.4);
                const prey = S.players.filter(p => !p.bench && target(p) === m && !p.leaving && !p.ko && !p.busy() &&
                    (p.state === 'idle' || (S.atbMode === 'active' && (p.state === 'ask' || p.state === 'perm'))));
                if (prey.length) { const p = prey[(Math.random() * prey.length) | 0]; m.push(() => monAttack(m, p)); }
            }
        }
        m.step(dt);
    }
    S.fx = S.fx.filter(e => (e.t += dt / e.dur) < 1);
    S.nums = S.nums.filter(n => (n.t += dt) < 1.2);
    if (S.banner && (S.banner.t += dt) > S.banner.dur) S.banner = null;
    S.flash = Math.max(0, S.flash - dt * 2.5);
}

/* ------------------------------------------------------------ render */

function todNow() {
    if (S.tod !== 'auto') return S.tod;
    const h = new Date().getHours();
    return h >= 7 && h < 17 ? 'day' : h >= 17 && h < 19 ? 'dusk' : 'night';
}

function poseOf(p) {
    if (p.walking) return p.walking === 'R' ? ((S.time * 8 | 0) % 2 ? 'stepR' : 'readyR') : ((S.time * 8 | 0) % 2 ? 'step' : 'ready');
    if (p.pose) return p.pose;
    if (p.ko) return 'ko';
    if (p.hurt > 0) return 'hurt';
    if (p.state === 'ask' || p.state === 'perm') return 'raise';
    if (p.hp <= RULES.hpMax * RULES.critical) return 'kneel';
    return 'ready';
}

function drawPlayer(p) {
    if (p.hidden) return;
    const k = CLS[p.cls], pose = poseOf(p), img = k.variants[p.variant][pose];
    const x = Math.round(p.x - p.fwd * 8), y = Math.round(p.y);
    if (!p.air) ellipseShade(x - 1, y, 9, 2, 0.35);
    if (p.diving) for (let i = -6; i <= 6; i += 3) line(x + i, y - 70, x + i, y - 40 - Math.abs(i) * 2, COL.white);
    if (pose === 'ko') blit(img, x - k.koA[0], y - k.koA[1], { white: p.white });
    else {
        const bob = pose === 'raise' && (S.time * 3 | 0) % 2 ? -1 : 0;
        blit(img, x - UP_AX, y - UP_AY + bob, { white: p.white });
    }
    if ((p.state === 'ask' || p.state === 'perm') && !p.ko && !p.leaving && !p.walking) {
        const b = p.state === 'ask' ? ICON.bubbleAsk : ICON.bubblePerm;
        blit(b, x + 6, y - 40 + ((S.time * 2 | 0) % 2));
    }
}

function drawMonster(m) {
    const K = MONS[m.kind], f = K.f[m.frame === 'attack' ? 'attack' : ((S.time * 1.6 + m.phase) | 0) % 2 ? 'idle2' : 'idle'] || K.f.idle;
    const float = K.m.float ? Math.round(Math.sin(S.time * 2.4 + m.phase) * 2) - 6 : 0;
    const sx = m.shake > 0 ? ((S.time * 40 | 0) % 2 ? 1 : -1) : 0;
    const x = Math.round(m.x - f.w / 2) + sx, y = Math.round(m.y - f.h + 1 + float);
    ellipseShade(Math.round(m.x), Math.round(m.y), Math.max(6, f.w * 0.38 | 0), 3, K.m.float ? 0.25 : 0.35);
    const o = { white: m.white, vis: m.vis };
    if (m.sweep !== undefined) { o.tint = [hex('#e060c0'), Math.min(0.8, m.sweep * 2)]; o.sweep = m.sweep; }
    blit(f, x, y, o);
}

function drawMonWindow() {
    window_(MON_X, WIN_Y, MON_W, WIN_H);
    const live = S.monsters.filter(m => !m.leaving);
    if (!live.length) { text('NO FOES', MON_X + 8, ROW_Y + 2, COL.dim); return; }
    live.forEach((m, i) => {
        const y = ROW_Y + i * 20, M = MONS[m.kind].m;
        text(M.name, MON_X + 7, y, m.white > 0.3 ? COL.gold : COL.text);
        textR(fmtTok(projDamage[m.proj] || 0), MON_X + MON_W - 7, y, COL.text);
        let pr = m.proj.replace(/\/+$/, '').split('/').pop();      /* the folder name */
        if (pr.length > 18) pr = pr.slice(0, 16) + '..';
        text(pr, MON_X + 7, y + 9, COL.dim);
    });
}

function drawPartyWindow() {
    window_(PARTY_X, WIN_Y, PARTY_W, WIN_H);
    const x0 = PARTY_X + 18;
    const rows = S.players.filter(p => !p.bench).sort((a, b) => a.slot - b.slot);
    if (!rows.length) text('NO ONE IS WORKING', x0 + 4, ROW_Y + 2, COL.dim);
    rows.forEach((p, i) => {
        const y = ROW_Y + i * ROW_H, k = CLS[p.cls].c;
        const waiting = (p.state === 'ask' || p.state === 'perm') && !p.ko;
        blit(p.tool === 'claude' ? ICON.claude : ICON.codex, x0, y);
        if (p.remote) { put(x0 + 6, y, COL.lime); put(x0 + 5, y, COL.lime); put(x0 + 6, y + 1, COL.lime); }
        const blink = (S.time * 3 | 0) % 2;
        const nameC = p.ko ? COL.ko : waiting ? (blink ? COL.gold : COL.text) : COL.text;
        text(p.name, x0 + 10, y, nameC);
        const hpC = p.hp === 0 ? COL.red : p.hp <= RULES.hpMax * RULES.critical ? COL.gold : COL.text;
        textR(String(Math.round(p.hp)), x0 + 71, y, hpC);
        textR(fmtTok(p.dealt), x0 + 111, y, COL.dim);
        /* the time gauges */
        const bx = x0 + 118, bw = 62;
        rect(bx - 1, y, bw + 2, 7, COL.outer);
        rect(bx, y + 1, bw, 5, COL.atbOff);
        if (p.ko) text('KO', bx + bw / 2 - 6, y, COL.red, COL.outer);
        else if (waiting) rect(bx, y + 1, bw, 5, blink ? COL.atbFull : COL.atbFill);
        else {
            const fill = Math.round(bw * p.atb);
            rect(bx, y + 1, fill, 5, p.state === 'work' ? COL.atbFill : COL.atbIdle);
            if (p.state === 'work') rect(bx, y + 1, fill, 1, COL.atbFull);
        }
        const st = p.ko ? '' : waiting ? (p.state === 'ask' ? '?' : '!') : p.state === 'idle' ? 'z' : '';
        if (st) text(st, bx + bw + 4, y, waiting ? COL.gold : COL.dim);
        if (waiting || S.selected === p) blit(ICON.cursor, PARTY_X + 3 + (S.time * 4 | 0) % 2, y - 1);
    });
    /* sessions past the eighth wait off the field: a tab on the window's edge */
    const waitingOff = S.players.filter(p => p.bench && !p.leaving).length;
    if (waitingOff) {
        const s = `+${waitingOff} WAIT`, w = s.length * 6 + 12, x = 226;
        window_(x, WIN_Y - 13, w, 16);
        text(s, x + 7, WIN_Y - 9, COL.gold);
    }
}

function drawInfoWindow() {
    window_(INFO_X, WIN_Y, INFO_W, WIN_H);
    const x0 = INFO_X + 7, xr = INFO_X + INFO_W - 7;
    let y = ROW_Y;
    text('GOLD', x0, y, COL.dim); textR('$' + Math.round(S.month.usd), xr, y, COL.text); y += 10;
    text('EXP', x0, y, COL.dim); textR(fmtTok(S.month.tokens), xr, y, COL.text); y += 10;
    const d = new Date(), hhmm = String(d.getHours()).padStart(2, '0') + ((S.time * 2 | 0) % 2 ? ':' : ' ') + String(d.getMinutes()).padStart(2, '0');
    text('TIME', x0, y, COL.dim); textR(hhmm, xr, y, COL.text); y += 12;
    rect(x0, y - 2, INFO_W - 14, 1, COL.borderLo);
    text('MP', x0, y, COL.gold); y += 9;
    const bars = [['claude', '5H', S.mana.c5h, COL.mana5h], ['claude', 'WK', S.mana.cwk, COL.manaWeek], ['codex', 'WK', S.mana.xwk, COL.manaCodex]];
    for (const [t, lbl, v, c] of bars) {
        blit(t === 'claude' ? ICON.claude : ICON.codex, x0, y);
        text(lbl, x0 + 9, y, COL.text);
        const bx = x0 + 22, bw = INFO_W - 14 - 22;
        rect(bx - 1, y + 1, bw + 2, 5, COL.outer);
        const fc = v < 10 ? COL.red : v < 25 ? COL.gold : c;
        rect(bx, y + 2, Math.round(bw * v / 100), 3, fc);
        y += 9;
    }
    blit(ICON.gear, INFO_X + INFO_W - 15, H - 15);
}

function drawBanner() {
    const b = S.banner;
    if (!b) return;
    const s = b.text, w = Math.max(Math.max(s.length, b.sub.length) * 6 + 18, 60), x = ((W - w) / 2) | 0;
    const h = b.sub ? 26 : 17;
    window_(x, 3, w, h);
    text(s, x + ((w - s.length * 6) / 2 | 0), 8, COL.text);
    if (b.sub) text(b.sub, x + ((w - b.sub.length * 6) / 2 | 0), 17, COL.gold);
}

function render() {
    const key = todNow();
    if (S.fieldKey !== key) { S.field = makeField(key); S.fieldKey = key; }
    fb.set(S.field);
    if (S.dim > 0) for (let i = 0; i < W * FIELD_H; i++) fb[i] = mix(fb[i], hex('#06061a'), S.dim);
    S.dim = 0;
    const ents = [
        ...S.monsters.map(m => ({ y: m.y, d: () => drawMonster(m) })),
        ...S.players.filter(p => !p.bench).map(p => ({ y: p.air ? p.hy : p.y, d: () => drawPlayer(p) })),
    ].sort((a, b) => a.y - b.y);
    for (const e of ents) e.d();
    for (const e of S.fx) e.draw(fb, e.t);
    for (const n of S.nums) {
        const bounce = n.t < 0.3 ? -Math.sin(n.t / 0.3 * Math.PI) * 7 : 0;
        numText(n.s, n.x, Math.round(n.y + bounce), n.c);
    }
    if (S.selected && !S.selected.leaving) {
        const p = S.selected, x = Math.round(p.x - p.fwd * 8) - 34 + ((S.time * 4 | 0) % 2), y = Math.round(p.y) - 20;
        blit(ICON.cursor, x, y);
    }
    if (S.flash > 0) for (let i = 0; i < W * FIELD_H; i++) fb[i] = mix(fb[i], COL.white, S.flash);
    rect(0, FIELD_H - 1, W, 1, COL.outer);
    drawMonWindow();
    drawPartyWindow();
    drawInfoWindow();
    drawBanner();
}

/* ------------------------------------------------------------ demo director */

const DEMO_PROJECTS = ['~/p/moon-garden', '~/work/star-charts', '~/p/lantern', '~/p/night-market', '~/work/paper-boats'];
let demoOn = true, demoT = 0, demoN = 0;
function demoTick(dt) {
    if (!demoOn) return;
    for (const p of S.players) {
        if (p.leaving || p.ko || p.state !== 'work') continue;
        p.pending += dt * (300 + Math.random() * 2500);
        if (Math.random() < dt / 3.2) {
            const ks = ['bash', 'bash', 'edit', 'edit', 'read', 'read', 'read', 'web', 'agent', 'todo', 'mcp'];
            tool(p, ks[(Math.random() * ks.length) | 0]);
        }
        S.month.tokens += dt * 900;
        S.month.usd += dt * 0.004;
        S.mana.c5h = Math.max(3, S.mana.c5h - dt * 0.02);
    }
    demoT -= dt;
    if (demoT > 0) return;
    demoT = 1.6 + Math.random() * 2.2;
    const live = S.players.filter(p => !p.leaving);
    const r = Math.random();
    if (live.length < 4 || (r < 0.1 && live.length < 10)) {
        const proj = DEMO_PROJECTS[(Math.random() * (live.length > 5 ? 5 : 3)) | 0];
        join({ id: 'demo' + (demoN++), proj, tool: Math.random() < 0.6 ? 'claude' : 'codex', remote: Math.random() < 0.25 });
        return;
    }
    if (r < 0.14 && live.length > 5) { leave(live[(Math.random() * live.length) | 0]); return; }
    const p = live[(Math.random() * live.length) | 0];
    if (p.ko) { if (Math.random() < 0.5) state(p, 'work'); return; }
    if (p.state === 'idle') { state(p, Math.random() < 0.35 ? 'work' : 'idle'); return; }
    if (p.state === 'ask' || p.state === 'perm') { if (Math.random() < 0.6) state(p, 'work'); return; }
    const q = Math.random();
    if (q < 0.4) state(p, 'idle');
    else if (q < 0.55) state(p, 'ask');
    else if (q < 0.65) state(p, 'perm');
}

/* ------------------------------------------------------------ boot */

let canvas, ctx, img, last = 0, running = false;
function frame(ts) {
    const dt = Math.min(0.1, last ? (ts - last) / 1000 : 0.033);
    last = ts;
    demoTick(dt);
    update(dt);
    render();
    ctx.putImageData(img, 0, 0);
    if (root.FF_ONFRAME) root.FF_ONFRAME();
    requestAnimationFrame(frame);
}

function start(cv) {
    canvas = cv;
    ctx = canvas.getContext('2d');
    img = ctx.createImageData(W, H);
    fb = new Uint32Array(img.data.buffer);
    loadMonsters();
    /* an opening party already mid-fight */
    const opening = [
        ['~/p/moon-garden', 'claude', false, 'work'], ['~/p/moon-garden', 'claude', true, 'work'],
        ['~/work/star-charts', 'codex', false, 'work'], ['~/work/star-charts', 'claude', false, 'idle'],
        ['~/p/lantern', 'claude', false, 'ask'], ['~/p/moon-garden', 'codex', false, 'work'],
    ];
    opening.forEach(([proj, t, rem, st], i) => {
        const p = join({ id: 'o' + i, proj, tool: t, remote: rem });
        p.x = p.hx; p.queue.length = 0; p.dealt = Math.round(2e4 + Math.random() * 4e5); p.atb = Math.random();
        if (st !== 'work') state(p, st);
        if (st === 'idle') p.hp = 420;
    });
    S.monsters.forEach(m => { m.vis = 1; m.queue.length = 0; });
    S.banner = null;
    projDamage['~/p/moon-garden'] = 1.24e6; projDamage['~/work/star-charts'] = 386e3; projDamage['~/p/lantern'] = 52e3;
    if (!running) { running = true; requestAnimationFrame(frame); }
}

function hit(x, y) {
    let best = null, bd = 1e9;
    for (const p of S.players) {
        if (p.leaving || p.bench) continue;
        const dx = x - (p.x - p.fwd * 8), dy = y - (p.y - 16);
        if (Math.abs(dx) < 14 && Math.abs(dy) < 20 && dx * dx + dy * dy < bd) { best = p; bd = dx * dx + dy * dy; }
    }
    if (!best && y >= ROW_Y && x >= PARTY_X && x < PARTY_X + PARTY_W) {
        const rows = S.players.filter(p => !p.bench).sort((a, b) => a.slot - b.slot), i = Math.floor((y - ROW_Y) / ROW_H);
        best = rows[i] || null;
    }
    return best;
}

root.FFB = {
    W, H, S, RULES, TOOLS, CLS, SPECIALS, moveFor, get MONS() { return MONS; }, start, join, leave, state, tool, tokens, hit, fmtTok,
    setDemo(v) { demoOn = v; }, get demo() { return demoOn; },
    _tick(dt) { demoTick(dt); update(dt); render(); },
    _fb() { return fb; },
    setIdleScale(v) { idleScale = v; }, get idleScale() { return idleScale; },
    addProject(proj) { return proj; },
    DEMO_PROJECTS,
    /* for ../tools/gen_art.mjs, which bakes the art into the app */
    ICON, TOD, COL, makeField, UP_AX, UP_AY,
};
})(typeof window !== 'undefined' ? window : globalThis);

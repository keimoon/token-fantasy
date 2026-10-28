(function (root) {
'use strict';
const FFC = root.FFC;

/* Player sprites: 36x44 canvases (38x46 once outlined), facing left, feet on
 * the bottom row, the body's centre line near x = 20. Each pose is composed
 * from a head, a torso, legs (or a robe) and two arms; the class adds a cape
 * behind, headgear over the head and a weapon under the near hand.
 *
 * Palette keys: S s skin, E eyes, H h J hair (base, shade, shine), A a outfit,
 * B b legs / under-layer, T t trim, F f boots, C c cape, plus the headgear's
 * own keys (P p O o L l V v G g) and BASE (K W w Y y X x I i N n). */
const CW = 36, CH = 44;

const HEAD = {
    open: [
        '....HHHHH...',
        '..HHHHHHHHh.',
        '.HHJJHHHHHhh',
        'HHJHHHHHHHhh',
        'HHHHHHHHHHhh',
        'HSHHSHHHHhhh',
        'SSSSSSSHHhhh',
        'SESSESSSShh.',
        'SESSESSSShh.',
        '.SSSSSSsSh..',
        '..ssSSss....',
    ],
    shut: [
        '....HHHHH...',
        '..HHHHHHHHh.',
        '.HHJJHHHHHhh',
        'HHJHHHHHHHhh',
        'HHHHHHHHHHhh',
        'HSHHSHHHHhhh',
        'SSSSSSSHHhhh',
        'SSSSSSSSShh.',
        'EESEESSSShh.',
        '.SSSSSSsSh..',
        '..ssSSss....',
    ],
    shout: [
        '....HHHHH...',
        '..HHHHHHHHh.',
        '.HHJJHHHHHhh',
        'HHJHHHHHHHhh',
        'HHHHHHHHHHhh',
        'HSHHSHHHHhhh',
        'SSSSSSSHHhhh',
        'SESSESSSShh.',
        'SESSESSSShh.',
        '.KKSSSSsSh..',
        '..ssSSss....',
    ],
};

const TORSO = {
    tunic: [
        '.aAAAAAAa.',
        'aAAAAAAAAa',
        'AAAATAAAaa',
        'AAAATAAAaa',
        'AAAAAAAAaa',
        'tTTTTTTTtt',
    ],
    robe: [
        '.aAAAAAAa.',
        'aAAAAAAAAa',
        'AAAAAAAAaa',
        'AAAAAAAAaa',
        'AAAAAAAAaa',
        'tTTTTTTTtt',
    ],
};

/* legs (tunic) or the robe's skirt, per stance; drawn under the torso */
const LOWER = {
    tunic: {
        stand: [
            '.BBBBBBbb.',
            '.BBBb.Bbb.',
            '.BBb...Bb.',
            '.BBb...Bb.',
            '.BBb...Bb.',
            'FFFf..FFf.',
            'FFFf.FFFf.',
        ],
        step: [
            '.BBBBBBbb..',
            'BBBb..Bbb..',
            'BBb....Bb..',
            'Bb.....bBb.',
            'Bb......Bb.',
            'FFf.....FFf',
            'FFf....FFFf',
        ],
        lunge: [
            '..BBBBBBbb..',
            '.BBBb..Bbbb.',
            'BBBb....Bbb.',
            'BBb......Bb.',
            'Bb.......Bb.',
            'FFf......Bbf',
            'FFFf.....FFf',
        ],
        kneel: [
            'BBBBBBBbb...',
            'BBBBb.Bbbb..',
            'FFbb...BBbb.',
            'FFf.....bBBf',
        ],
    },
    robe: {
        stand: [
            '.AAAAAAaa.',
            '.AAAAAAAa.',
            'AAAAAAAAaa',
            'AAAAAAAAaa',
            'AAAAAAAAAa',
            'TTTTTTTTtt',
            'FFf...FFf.',
        ],
        step: [
            '.AAAAAAaa..',
            'AAAAAAAAa..',
            'AAAAAAAAaa.',
            'AAAAAAAAAa.',
            'AAAAAAAAAaa',
            'TTTTTTTTTtt',
            'FFf.....FFf',
        ],
        lunge: [
            '..AAAAAAaa..',
            '.AAAAAAAAa..',
            'AAAAAAAAAaa.',
            'AAAAAAAAAaa.',
            'AAAAAAAAAAa.',
            'TTTTTTTTTTt.',
            'FFf......FFf',
        ],
        kneel: [
            'AAAAAAAAa...',
            'AAAAAAAAAa..',
            'AAAAAAAAAAa.',
            'TTTTTTTTTTtt',
        ],
    },
};

const ARM = {
    rest: ['aAK', 'AAK', 'AaK', 'AaK', 'SsK', 'Ss.'],
    fwd: ['SsAAAa', 'SsaAAa'],
    up: ['Ss..', 'SS..', 'AA..', 'AA..', 'Aa..', '.Aa.', '.AAa', '..Aa', '..AA'],
    cast: ['Ss...', 'SSa..', '.AAa.', '..AAa', '..aAA'],
    back: ['AAa..', '.AAa.', '..aSs', '...Ss'],
    kneel: ['aAK', 'AAK', 'AaK', 'Aa.', 'Ss.', 'Ss.'],
    farRest: ['aa', 'aa', 'aa', 'ss'],
    farUp: ['s.', 'aa', 'aa', 'aa'],
};

/* Poses. Coordinates are canvas pixels. hand: where the near hand grips the
 * weapon, with the direction the weapon points (null: no weapon). */
const POSES = {
    ready: { torso: [15, 31], lower: ['stand', 15, 37], head: ['open', 14, 20], far: ['farRest', 24, 32], near: ['rest', 14, 32], hand: [14, 36, 'rest'] },
    step: { torso: [15, 31], lower: ['step', 14, 37], head: ['open', 14, 20], far: ['farRest', 24, 32], near: ['rest', 14, 32], hand: [14, 36, 'rest'] },
    attack: { torso: [13, 31], lower: ['lunge', 12, 37], head: ['open', 11, 21], far: ['farRest', 23, 32], near: ['fwd', 8, 33], hand: [8, 33, 'fwd'] },
    cast: { torso: [15, 31], lower: ['stand', 15, 37], head: ['open', 14, 20], far: ['farUp', 23, 28], near: ['cast', 10, 28], hand: [10, 28, 'cast'] },
    raise: { torso: [15, 31], lower: ['stand', 15, 37], head: ['shout', 14, 20], far: ['farRest', 24, 32], near: ['up', 11, 23], hand: null },
    hurt: { torso: [17, 31], lower: ['step', 16, 37], head: ['shut', 18, 19], far: null, near: ['back', 24, 31], hand: null },
    kneel: { torso: [15, 34], lower: ['kneel', 14, 40], head: ['shut', 13, 24], far: ['farRest', 24, 35], near: ['kneel', 13, 35], hand: [13, 39, 'kneel'] },
    ko: { torso: [15, 31], lower: ['stand', 15, 37], head: ['shut', 14, 20], far: ['farRest', 24, 32], near: ['rest', 14, 32], hand: null, rot: true },
};

/* Weapons are drawn procedurally along a direction from the grip: `back`
 * runs behind the hand, `seq` ahead of it; each step is a colour key, or
 * { c, s (second pixel for thickness), g (cross-guard colour) }. */
const DIRS = { ul: [-1, -1], u: [0, -1], l: [-1, 0], dl: [-1, 1], ur: [1, -1] };
const SIDE = { ul: [1, 0], u: [1, 0], l: [0, 1], dl: [1, 0], ur: [-1, 0] };
const PERP = { ul: [[1, -1], [-1, 1]], u: [[-1, 0], [1, 0]], l: [[0, -1], [0, 1]], dl: [[1, 1], [-1, -1]], ur: [[1, 1], [-1, -1]] };
const rep = (n, v) => Array(n).fill(v);
const WEAPONS = {
    sword: { dirs: { rest: 'ul', fwd: 'l', cast: 'u', kneel: 'dl' },
        back: ['n'], seq: ['N', { c: 'y', g: 'Y' }, ...rep(8, { c: 'I', s: 'i' }), 'I'] },
    dagger: { dirs: { rest: 'ul', fwd: 'l', cast: 'u', kneel: 'dl' },
        back: ['n'], seq: ['N', { c: 'y', g: 'Y' }, ...rep(4, { c: 'I', s: 'i' }), 'I'] },
    rapier: { dirs: { rest: 'ul', fwd: 'l', cast: 'u', kneel: 'dl' },
        back: ['Y'], seq: ['N', { c: 'Y', g: 'Y', s: 'y' }, ...rep(11, 'I')] },
    staffB: { dirs: { rest: 'u', fwd: 'ul', cast: 'u', kneel: 'u' },
        back: rep(3, 'n'), seq: [...rep(10, { c: 'N', s: 'n' }), { c: 'G', s: 'g', g: 'G' }, { c: 'G', s: 'g' }] },
    staffW: { dirs: { rest: 'u', fwd: 'ul', cast: 'u', kneel: 'u' },
        back: rep(3, 'n'), seq: [...rep(10, { c: 'N', s: 'n' }), { c: 'Y', g: 'Y' }, { c: 'G', s: 'g', g: 'Y' }, { c: 'Y' }] },
    lance: { dirs: { rest: 'u', fwd: 'l', cast: 'u', kneel: 'u' },
        back: rep(4, 'n'), seq: [...rep(12, { c: 'N', s: 'n' }), { c: 'Y', g: 'Y' }, { c: 'I', s: 'i', g: 'i' }, { c: 'I', s: 'i' }, 'I'],
        l: { back: rep(10, 'n'), seq: [...rep(3, { c: 'N', s: 'n' }), { c: 'Y', g: 'Y' }, { c: 'I', s: 'i', g: 'i' }, { c: 'I', s: 'i' }, 'I'] } },
    rod: { dirs: { rest: 'ul', fwd: 'l', cast: 'u', kneel: 'u' },
        back: ['n'], seq: [...rep(6, { c: 'N', s: 'n' }), { c: 'Y', g: 'Y' }, { c: 'G', s: 'g' }, 'G'] },
};

function drawWeapon(g, w, kind, gx, gy) {
    const dir = w.dirs[kind];
    if (!dir) return;
    const spec = w[dir] || w;
    const [dx, dy] = DIRS[dir], [sx, sy] = SIDE[dir];
    const put = (x, y, ch) => { if (ch && x >= 0 && y >= 0 && x < CW && y < CH) g[y][x] = ch; };
    spec.back.forEach((st, k) => put(gx - dx * (k + 1), gy - dy * (k + 1), typeof st === 'string' ? st : st.c));
    spec.seq.forEach((st, k) => {
        const x = gx + dx * k, y = gy + dy * k;
        const o = typeof st === 'string' ? { c: st } : st;
        put(x, y, o.c);
        if (o.s) put(x + sx, y + sy, o.s);
        if (o.g) for (const [px, py] of PERP[dir]) put(x + px, y + py, o.g);
    });
}

/* Headgear and other class pieces, placed relative to the head (hx, hy). */
const GEAR = {
    helm: { at: [-2, -6], rows: [
        'I.............I.',
        'Ii...........iI.',
        '.Ii.........iI..',
        '..Ii..III..iI...',
        '...IIIIIIIIIi...',
        '..IIIIIIIIIIii..',
        '..IIIIIIIIIIIii.',
        '.IIIIIIIIIIIiii.',
        '.iIIIIIIIIIiiii.',
        '.YYYYYYYYYyyyyy.',
        '.YVYYYYYYYyiiii.',
        '..........iiii..',
        '..........iii...',
        '..........ii....',
    ] },
    wizardHat: { at: [-4, -10], rows: [
        '.............PP.....',
        '...........PPPp.....',
        '..........PPPp......',
        '.........PPPp.......',
        '........PPPPp.......',
        '.......PPPPPp.......',
        '......PPPPPPpp......',
        '......PPPPPPPp......',
        '.....PPPPPPPPpp.....',
        '.....PPPPPPPPPp.....',
        '....oOOOOOOOOOOo....',
        '..PPPPPPPPPPPPPPPp..',
        'PPPPPPPPPPPPPPPPPPpp',
        '.pppppppppppppppppp.',
    ] },
    hood: { at: [-1, -2], rows: [
        '.....WWWWW.....',
        '...WWWWWWWWw...',
        '..WWWWWWWWWWww.',
        '.WWWWWWWWWWWww.',
        '.WWWWWWWWWWWwww',
        'WWWWWWWWWWWWwww',
        'WXXWXXWWWWWWwww',
        'X.....XWWWWWwww',
        'X......XWWWWww.',
        'X.......XWWWww.',
        'X.......XWWWw..',
        '........XWWww..',
        '.......XWWww...',
        '.......wwww....',
    ] },
    plumedHat: { at: [-3, -5], rows: [
        '..............WW....',
        '.............WWwW...',
        '.....PPPPP..WWw.W...',
        '....PPPPPPPWWw..W...',
        '...PPPPPPPPPWw.W....',
        '...PPPPPPPPPpp......',
        '...OOOOOOOOOoo......',
        '.PPPPPPPPPPPPPpp....',
        'PPPPPPPPPPPPPPPppp..',
        '.ppppppppppppppp....',
    ] },
    bandana: { at: [0, 0], rows: [
        '....LLLLL.......',
        '..LLLLLLLLl.....',
        '.LLLlLLLLLll....',
        'LLLLLLLLLLll....',
        'LlLLLLLLLLlll...',
        '.........llLL...',
        '..........lLLl..',
        '...........lLl..',
        '............ll..',
    ] },
    headband: { at: [-1, -3], rows: [
        '.H..H..H.......',
        '.HH.HH.HH......',
        '..HHHHHHHH.....',
        '...............',
        '...............',
        '...............',
        '...............',
        'LLLLLLLLLLLll..',
        '...........lLL.',
        '............lLL',
        '.............lL',
    ] },
    lancerHelm: { at: [-2, -7], rows: [
        '...........V......',
        '..........VV....V.',
        '.........VVv...VV.',
        '........VVv...VVv.',
        '.......VVv...VVv..',
        '.....PPPPPPPPVv...',
        '...PPPPPPPPPPPp...',
        '..PPPPPOPPPPPPpp..',
        '.PPPPPOPPPPPPPpp..',
        'PPPPPOPPPPPPPPpp..',
        '.PPPOPPPPPPPPPppp.',
        'PPOOOOOOOPPPPPppp.',
        '..........PPPPpp..',
        '..........PPPPpp..',
        '..........PPPpp...',
        '...........PPp....',
    ] },
    circlet: { at: [-2, -5], rows: [
        '.....WW..........',
        '....WWw..........',
        '...WWw...........',
        '...Ww............',
        '..HHHHH..........',
        '.HJJHHHHHH.......',
        '.................',
        '.................',
        '.................',
        'YYYYYYYYYYYYy....',
        '...........HHh...',
        '...........HHhh..',
        '...........JHHhh.',
        '............HHhh.',
        '............HHHh.',
        '............HHhh.',
        '...........HHHh..',
        '...........HHhh..',
        '............HHh..',
        '............hhh..',
        '.............hh..',
    ] },
};

const CAPE = {
    rest: ['XX..', 'XXx.', 'XXxx', 'XXXx', 'XXXx', 'XXXx', 'XXXx', 'XXxx', 'Xxx.'],
    fly: ['XX....', 'XXXx..', 'XXXxx.', '.XXXxx', '.XXXXx', '..XXXx', '..XXxx', '...Xx.'],
};

function compose(name, c, pose, pal) {
    const g = Array.from({ length: CH }, () => Array(CW).fill('.'));
    const put = (rows, x, y) => rows.forEach((r, j) => {
        for (let i = 0; i < r.length; i++) {
            const ch = r[i];
            if (ch === '.') continue;
            const gx = x + i, gy = y + j;
            if (gx < 0 || gy < 0 || gx >= CW || gy >= CH) throw new Error(`${name}: part leaves the canvas at ${gx},${gy}`);
            g[gy][gx] = ch;
        }
    });
    const body = c.body;
    const [tx, ty] = pose.torso;
    if (c.cape) put(pose === POSES.attack ? CAPE.fly : CAPE.rest, tx + 7, ty + 1);
    if (pose.far) put(ARM[pose.far[0]], pose.far[1], pose.far[2]);
    put(LOWER[body][pose.lower[0]], pose.lower[1], pose.lower[2]);
    put(TORSO[body], tx, ty);
    const [hk, hx, hy] = pose.head;
    put(HEAD[hk], hx, hy);
    if (c.gear) for (const k of c.gear) put(GEAR[k].rows, hx + GEAR[k].at[0], hy + GEAR[k].at[1]);
    if (pose.hand && c.weapon) drawWeapon(g, WEAPONS[c.weapon], pose.hand[2], pose.hand[0], pose.hand[1]);
    put(ARM[pose.near[0]], pose.near[1], pose.near[2]);
    const sp = FFC.sprite(name, g.map(r => r.join('')), pal || c.pal);
    sp.rot = !!pose.rot;
    return sp;
}

const SKIN = { S: '#f8c898', s: '#c88a60' };
const CLASSES = [
    { id: 'warrior', kind: 'fighter', names: ['ALDRIC', 'BRANDT', 'CADOC', 'DARIEN', 'EDRIC', 'GARETH', 'HALDOR', 'TORVIN'],
      alt: { A: '#c84848', a: '#7a2028', X: '#4a6ad8', x: '#2a3a8a', V: '#50e090', H: '#f0e8d0', h: '#b0a890' }, job: 'WARRIOR', weapon: 'sword', body: 'tunic', gear: ['helm'], cape: true,
      pal: { ...SKIN, E: '#1a1222', H: '#d8b060', h: '#a07830', J: '#f0d890', A: '#5a7ad8', a: '#34489a', T: '#f0c850', t: '#a8801c',
             B: '#48506a', b: '#2c3248', F: '#9aa4c0', f: '#5a6280', V: '#4a8af8' } },
    { id: 'sorcerer', kind: 'mage', names: ['MORVEN', 'QUILL', 'SABLE', 'VESPER', 'ORRIN', 'ZANTH'],
      alt: { A: '#8a48c8', a: '#542a80', E: '#ff9a40', P: '#a8b0c8', p: '#6a7090', O: '#4a5070' }, job: 'SORCERER', weapon: 'staffB', body: 'robe', gear: ['wizardHat'],
      pal: { S: '#24203a', s: '#1a1628', E: '#f8e040', H: '#24203a', h: '#1a1628', J: '#24203a', A: '#3a5ad0', a: '#22348a',
             T: '#2a2a4a', t: '#1c1c34', B: '#2a2a4a', b: '#1c1c34', F: '#8a5a30', f: '#5a3a1c',
             P: '#d8b468', p: '#9a7a3a', O: '#7a5a2a', o: '#5a4020', G: '#e84040', g: '#8c1c28' } },
    { id: 'cleric', kind: 'mage', names: ['ELOWEN', 'LIORA', 'ALBA', 'MAELIS', 'SOLENE', 'IONA'],
      alt: { A: '#c8d8f8', a: '#8898c8', B: '#c8d8f8', b: '#8898c8', W: '#c8d8f8', w: '#8898c8', T: '#3a7ad8', t: '#24489a', X: '#3a7ad8', x: '#24489a', H: '#8a5a3a', h: '#5a3a22', J: '#c08a5a', G: '#f86ab0', g: '#a83070' }, job: 'CLERIC', weapon: 'staffW', body: 'robe', gear: ['hood'],
      pal: { ...SKIN, E: '#1a1222', H: '#e8d088', h: '#b89850', J: '#f8ecb8', A: '#f4f0f4', a: '#b8b0c8', T: '#d83040', t: '#8c1c28',
             B: '#f4f0f4', b: '#b8b0c8', F: '#a87a50', f: '#6a4a2a', G: '#58c0f8', g: '#2a70b8' } },
    { id: 'spellsword', kind: 'mage', names: ['CASSIA', 'MARLO', 'SCARLA', 'VIOLA', 'AUREN', 'DESMA'],
      alt: { A: '#3a50c8', a: '#22307a', P: '#3a50c8', p: '#22307a', H: '#e87830', h: '#a04818', J: '#f8b070' }, job: 'SPELLSWORD', weapon: 'rapier', body: 'robe', gear: ['plumedHat'],
      pal: { ...SKIN, E: '#1a1222', H: '#f0d070', h: '#b89030', J: '#f8e8a8', A: '#d03838', a: '#86202a', T: '#f0e8d8', t: '#b0a088',
             B: '#f0e8d8', b: '#b0a088', F: '#6a3a28', f: '#40221a', P: '#d03838', p: '#86202a', O: '#f0c850', o: '#a8801c' } },
    { id: 'thief', kind: 'fighter', names: ['WREN', 'FINCH', 'KESTREL', 'PIP', 'NICO', 'TAMSIN'],
      alt: { A: '#c84848', a: '#7a2028', L: '#4aa858', l: '#2a6a3a', H: '#f0d070', h: '#b89030', J: '#f8e8a8' }, job: 'THIEF', weapon: 'dagger', body: 'tunic', gear: ['bandana'],
      pal: { ...SKIN, E: '#1a1222', H: '#9a6a3a', h: '#5e3c22', J: '#c89058', A: '#4aa858', a: '#2a6a3a', T: '#f0c850', t: '#a8801c',
             B: '#5a4a6a', b: '#3a3048', F: '#7a4a28', f: '#4a2a18', L: '#3a7ad8', l: '#24489a' } },
    { id: 'monk', kind: 'fighter', names: ['TAIGA', 'OSKAR', 'BRUNO', 'KENJI', 'HALVAR', 'MAGNA'],
      alt: { A: '#3a7ad8', a: '#24489a', T: '#3a7ad8', B: '#3a7ad8', b: '#24489a', L: '#f8f8f0', l: '#aeb0c8', H: '#8a4a2a', h: '#5a2a18', J: '#b8704a' }, job: 'MONK', weapon: null, body: 'tunic', gear: ['headband'],
      pal: { S: '#e8b080', s: '#b07850', E: '#1a1222', H: '#2a2230', h: '#16121c', J: '#4a4258', A: '#f08a30', a: '#a85418', T: '#f08a30', t: '#2a2230',
             B: '#f08a30', b: '#a85418', F: '#e8b080', f: '#b07850', L: '#e83c3c', l: '#8c1c28' } },
    { id: 'lancer', kind: 'fighter', names: ['ARVID', 'SIGRUN', 'TORAN', 'VALKA', 'BJORN'],
      alt: { A: '#b83848', a: '#701c28', P: '#b83848', p: '#701c28', T: '#f0c850', t: '#a8801c', B: '#5a2030', b: '#3a1420', F: '#b83848', f: '#701c28', V: '#f0c850', v: '#a8801c', O: '#f0c850' }, job: 'LANCER', weapon: 'lance', body: 'tunic', gear: ['lancerHelm'],
      pal: { ...SKIN, E: '#1a1222', H: '#f0d070', h: '#b89030', J: '#f8e8a8', A: '#5a4ac0', a: '#34287a', T: '#9a8ae8', t: '#5a4ac0',
             B: '#34287a', b: '#22184e', F: '#5a4ac0', f: '#34287a', P: '#4a3ca8', p: '#2a206a', O: '#c8b8ff', V: '#b8a8f8', v: '#6a5ad0' } },
    { id: 'summoner', kind: 'mage', names: ['ELARA', 'NERIS', 'OPHIRA', 'SELKA', 'TALIA'],
      alt: { H: '#b878e8', h: '#7a48b0', J: '#e0c0f8', A: '#d86a9a', a: '#8a3a60', B: '#d86a9a', b: '#8a3a60', G: '#58c0f8', g: '#2a70b8' }, job: 'SUMMONER', weapon: 'rod', body: 'robe', gear: ['circlet'],
      pal: { ...SKIN, E: '#1a1222', H: '#78e088', h: '#34a050', J: '#c8f8c0', A: '#2c7a8a', a: '#1a4a58', T: '#f0c850', t: '#a8801c',
             B: '#2c7a8a', b: '#1a4a58', F: '#8a5a30', f: '#5a3a1c', G: '#f86ab0', g: '#a83070' } },
];

/* variant 0 is the class's own colours, 1 the palette swap a second
 * player of the same class wears */
function build(c, variant = 0) {
    const pal = variant ? { ...c.pal, ...c.alt } : c.pal;
    const out = {};
    for (const [k, pose] of Object.entries(POSES)) out[k] = compose(`${c.id}.${k}`, c, pose, pal);
    return out;
}

function rasterPose(sp) {
    const img = FFC.raster(sp);
    return sp.rot ? FFC.rotCW(img) : img;
}

root.FF_PLAYERS = {
    CW, CH, CLASSES, POSES, build, rasterPose,
    sheet() {
        return CLASSES.flatMap(c => [0, 1].map(v => {
            const p = build(c, v);
            return { label: `${c.names[0]} ${c.job}${v ? ' (swap)' : ''}`, items: Object.entries(p).map(([k, sp]) => ({ label: k, img: rasterPose(sp) })) };
        }));
    },
};
})(typeof window !== 'undefined' ? window : globalThis);

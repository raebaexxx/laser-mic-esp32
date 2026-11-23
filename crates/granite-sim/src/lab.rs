//! Interactive 3D laboratory view ("the rig") for the digital twin.
//!
//! The page is a single self-contained HTML file: a perspective canvas scene
//! with the granite plate on its supports, the laser vibrometer, the ADC and
//! ESP32 boards, the Wi-Fi link and the analysis laptop, plus a rack of slabs.
//! Every slab in the rack is one scenario of the twin; clicking it loads that
//! scenario's measured modes, damping, packet statistics and verdict.
//!
//! The plate deformation is a superposition of the detected mode shapes with
//! the measured frequencies and damping ratios.  It is deliberately slowed
//! down and exaggerated for the eye, but the modal ordering and the decay
//! differences between healthy and damaged slabs are physical.

use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::Path;

use granite_core::time::{format_unix_utc, now_unix_s};

use crate::scenario::ScenarioRun;

/// One tracked mode, copied out of the analysis for the page payload.
#[derive(Clone, Debug)]
pub struct LabMode {
    pub frequency_hz: f64,
    pub damping_ratio: f64,
    pub q_factor: f64,
    pub amplitude: f64,
    pub persistence: f64,
}

/// Everything the browser needs about one scenario/slab.
#[derive(Clone, Debug)]
pub struct LabScene {
    pub id: String,
    pub title: String,
    pub description: String,
    pub expected: String,
    pub predicted: String,
    pub condition_class: String,
    pub score: f64,
    pub confidence: f64,
    pub fundamental_hz: f64,
    pub damping: f64,
    pub t60_s: f64,
    pub mode_count: usize,
    pub event_count: usize,
    pub loss: f64,
    pub jitter_ms: f64,
    pub flags: Vec<String>,
    pub truth: String,
    pub material: String,
    pub length_mm: f64,
    pub width_mm: f64,
    pub thickness_mm: f64,
    pub modes: Vec<LabMode>,
}

/// Build the payload for one finished scenario run.
pub fn scene_from_run(run: &ScenarioRun) -> LabScene {
    let stats = run.capture.stats.as_ref();
    LabScene {
        id: run.scenario.id.clone(),
        title: run.scenario.title.clone(),
        description: run.scenario.description.clone(),
        expected: run.scenario.expected.label().to_string(),
        predicted: run.assessment.condition.label().to_string(),
        condition_class: run.assessment.condition.css_class().to_string(),
        score: run.assessment.anomaly_score,
        confidence: run.assessment.confidence,
        fundamental_hz: run.analysis.features.fundamental_hz,
        damping: run.analysis.features.mean_damping,
        t60_s: run.analysis.features.decay_t60_s,
        mode_count: run.analysis.features.mode_count,
        event_count: run.analysis.features.event_count,
        loss: stats
            .map(|stats| stats.packet_loss_fraction())
            .unwrap_or(0.0),
        jitter_ms: stats.map(|stats| stats.max_jitter_ms).unwrap_or(0.0),
        flags: run.assessment.flags.clone(),
        truth: run.truth.description.clone(),
        material: run.scenario.material.name.to_string(),
        length_mm: run.scenario.geometry.length_m * 1000.0,
        width_mm: run.scenario.geometry.width_m * 1000.0,
        thickness_mm: run.scenario.geometry.thickness_m * 1000.0,
        modes: run
            .analysis
            .modes
            .iter()
            .map(|mode| LabMode {
                frequency_hz: mode.frequency_hz,
                damping_ratio: mode.damping_ratio,
                q_factor: mode.q_factor,
                amplitude: mode.amplitude,
                persistence: mode.persistence,
            })
            .collect(),
    }
}

/// Write the interactive lab page for the supplied runs.
pub fn write_lab(runs: &[ScenarioRun], path: &Path) -> io::Result<()> {
    let scenes: Vec<LabScene> = runs.iter().map(scene_from_run).collect();
    let selected = if scenes.iter().any(|scene| scene.id == "crack") {
        "crack"
    } else {
        scenes.first().map(|scene| scene.id.as_str()).unwrap_or("")
    };
    let seed = runs.first().map(|run| run.seed).unwrap_or(0);
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    fs::write(path, render_lab_html(&scenes, selected, seed))
}

/// Render the complete self-contained page.
pub fn render_lab_html(scenes: &[LabScene], selected: &str, seed: u64) -> String {
    let mut html = String::with_capacity(180 * 1024);
    html.push_str(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>Granite Lab — impact-modal NDT</title><style>",
    );
    html.push_str(LAB_CSS);
    html.push_str("</style></head><body>");
    html.push_str(LAB_BODY);
    html.push_str("<script>\nconst LAB = ");
    html.push_str(&lab_json(scenes, selected, seed));
    html.push_str(";\n");
    html.push_str(LAB_JS);
    html.push_str("\n</script></body></html>");
    html
}

fn num(value: f64) -> String {
    if value.is_finite() {
        format!("{value:.6}")
    } else {
        "0".to_string()
    }
}

fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u003e"),
            ch if (ch as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", ch as u32);
            }
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

fn lab_json(scenes: &[LabScene], selected: &str, seed: u64) -> String {
    let mut out = String::with_capacity(scenes.len() * 1200 + 256);
    let _ = write!(
        out,
        "{{\"generated\":{},\"seed\":{},\"selected\":{},\"scenes\":[",
        json_string(&format_unix_utc(now_unix_s())),
        seed,
        json_string(selected)
    );
    for (index, scene) in scenes.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        let _ = write!(
            out,
            "{{\"id\":{},\"title\":{},\"description\":{},\"expected\":{},\"predicted\":{},\"condition_class\":{},\"score\":{},\"confidence\":{},\"f1\":{},\"damping\":{},\"t60\":{},\"mode_count\":{},\"event_count\":{},\"loss\":{},\"jitter_ms\":{},\"material\":{},\"length_mm\":{},\"width_mm\":{},\"thickness_mm\":{},\"truth\":{},\"flags\":[",
            json_string(&scene.id),
            json_string(&scene.title),
            json_string(&scene.description),
            json_string(&scene.expected),
            json_string(&scene.predicted),
            json_string(&scene.condition_class),
            num(scene.score),
            num(scene.confidence),
            num(scene.fundamental_hz),
            num(scene.damping),
            num(scene.t60_s),
            scene.mode_count,
            scene.event_count,
            num(scene.loss),
            num(scene.jitter_ms),
            json_string(&scene.material),
            num(scene.length_mm),
            num(scene.width_mm),
            num(scene.thickness_mm),
            json_string(&scene.truth),
        );
        for (flag_index, flag) in scene.flags.iter().enumerate() {
            if flag_index > 0 {
                out.push(',');
            }
            out.push_str(&json_string(flag));
        }
        out.push_str("],\"modes\":[");
        for (mode_index, mode) in scene.modes.iter().enumerate() {
            if mode_index > 0 {
                out.push(',');
            }
            let _ = write!(
                out,
                "{{\"frequency_hz\":{},\"damping_ratio\":{},\"q_factor\":{},\"amplitude\":{},\"persistence\":{}}}",
                num(mode.frequency_hz),
                num(mode.damping_ratio),
                num(mode.q_factor),
                num(mode.amplitude),
                num(mode.persistence)
            );
        }
        out.push_str("]}");
    }
    out.push_str("]}");
    out
}

const LAB_CSS: &str = r##"
:root {
  --bg: #060b14;
  --ink: #e9f2fb;
  --muted: #8fa6c4;
  --line: rgba(118, 160, 210, .18);
  --cyan: #4cc9f0;
  --green: #06d6a0;
  --orange: #ffb703;
  --red: #ef476f;
  --panel: rgba(9, 19, 33, .84);
}
* { box-sizing: border-box; }
html, body { height: 100%; }
body {
  margin: 0;
  overflow: hidden;
  background: var(--bg);
  color: var(--ink);
  font-family: Inter, system-ui, -apple-system, "Segoe UI", Roboto, sans-serif;
  -webkit-font-smoothing: antialiased;
}
#scene { position: fixed; inset: 0; width: 100%; height: 100%; display: block; touch-action: none; cursor: grab; }
#scene:active { cursor: grabbing; }
.scanlines {
  position: fixed; inset: 0; pointer-events: none;
  background: repeating-linear-gradient(0deg, rgba(255, 255, 255, .016) 0 1px, transparent 1px 3px);
  opacity: .45;
}
.topbar {
  position: fixed; top: 0; left: 0; right: 0;
  display: flex; justify-content: space-between; align-items: flex-start;
  gap: 12px; padding: 14px 18px; z-index: 10; pointer-events: none;
}
.brand { display: flex; gap: 10px; align-items: flex-start; }
.brand strong { display: block; font-size: 15px; letter-spacing: .18em; }
.brand-sub { display: block; margin-top: 3px; color: var(--muted); font-size: 9px; letter-spacing: .15em; text-transform: uppercase; }
.pulse { color: var(--red); font-size: 11px; margin-top: 2px; animation: pulse 1.6s ease-in-out infinite; }
@keyframes pulse { 0%, 100% { opacity: .3; } 50% { opacity: 1; text-shadow: 0 0 12px rgba(239, 71, 111, .95); } }
.viewbar { display: flex; gap: 6px; align-items: center; flex-wrap: wrap; justify-content: flex-end; pointer-events: auto; }
button, select {
  font: inherit; font-size: 11px; color: var(--ink);
  background: rgba(17, 31, 51, .9);
  border: 1px solid var(--line); border-radius: 9px;
  padding: 6px 10px; cursor: pointer;
  transition: border-color .12s, background .12s, color .12s;
}
button:hover, select:hover { border-color: rgba(76, 201, 240, .55); background: rgba(23, 41, 66, .96); }
button:focus-visible, select:focus-visible { outline: 2px solid var(--cyan); outline-offset: 2px; }
button.primary {
  background: linear-gradient(180deg, #ffc233, #f0a500);
  border-color: #ffb703; color: #1b1200; font-weight: 800;
}
button.primary:hover { filter: brightness(1.06); }
.chip.active, #labels.active { border-color: var(--cyan); color: #c9f5ff; background: rgba(76, 201, 240, .14); }
.panel {
  position: fixed; top: 72px; right: 18px; width: 336px;
  max-height: calc(100vh - 268px); overflow-y: auto;
  padding: 14px 16px; z-index: 9;
  background: var(--panel); border: 1px solid var(--line); border-radius: 16px;
  backdrop-filter: blur(12px);
  box-shadow: 0 24px 60px rgba(0, 0, 0, .45);
  scrollbar-width: thin;
}
.eyebrow { font-size: 8.5px; letter-spacing: .17em; color: var(--muted); text-transform: uppercase; }
.panel h1 { margin: 4px 0 3px; font-size: 17px; line-height: 1.25; letter-spacing: -.01em; }
.lede {
  margin: 0 0 10px; font-size: 11px; line-height: 1.45; color: #b9cbe0;
  display: -webkit-box; -webkit-line-clamp: 3; -webkit-box-orient: vertical; overflow: hidden;
}
.verdict-row { display: flex; align-items: baseline; gap: 8px; margin: 2px 0 8px; }
.badge {
  display: inline-flex; align-items: center; padding: 3px 8px; border-radius: 99px;
  font-size: 9px; font-weight: 800; letter-spacing: .12em;
  border: 1px solid transparent;
}
.badge.healthy { color: #06251c; background: var(--green); border-color: var(--green); }
.badge.suspect { color: #241a00; background: var(--orange); border-color: var(--orange); }
.badge.defect { color: #2a0710; background: var(--red); border-color: var(--red); }
.badge.unknown { color: var(--ink); background: #33445a; border-color: #33445a; }
.score { font-size: 24px; font-weight: 800; font-variant-numeric: tabular-nums; letter-spacing: -.03em; }
.score-cap { font-size: 8.5px; color: var(--muted); letter-spacing: .1em; text-transform: uppercase; }
.gauge { position: relative; height: 8px; border-radius: 99px; background: rgba(255, 255, 255, .07); overflow: hidden; }
.gauge > div { position: absolute; left: 0; top: 0; bottom: 0; width: 0; border-radius: 99px; transition: width .45s ease; }
.gauge i { position: absolute; top: -2px; bottom: -2px; width: 1px; background: rgba(255, 255, 255, .4); }
.gauge-cap { display: flex; justify-content: space-between; margin: 3px 0 10px; font-size: 8px; letter-spacing: .08em; text-transform: uppercase; color: var(--muted); }
.stats { display: grid; grid-template-columns: 1fr 1fr; gap: 7px 10px; margin-bottom: 10px; }
.stat b { display: block; font-size: 12.5px; font-weight: 700; font-variant-numeric: tabular-nums; }
.stat span { display: block; margin-top: 1px; font-size: 8px; letter-spacing: .09em; text-transform: uppercase; color: var(--muted); }
.modes { display: flex; flex-wrap: wrap; gap: 4px; margin: 0 0 10px; }
.mode { padding: 3px 7px; border-radius: 99px; font-size: 8.5px; color: #b9cbe0; }
.mode b { color: var(--ink); font-size: 9px; margin-right: 2px; }
.mode.active { border-color: var(--orange); background: rgba(255, 183, 3, .16); color: #ffe6a8; }
.flags ul { margin: 4px 0 0; padding-left: 14px; font-size: 10.5px; line-height: 1.45; color: #c6d6e8; }
.flags li { margin-bottom: 2px; }
.truth {
  margin-top: 9px; padding-top: 8px; border-top: 1px solid var(--line);
  font-size: 9.5px; line-height: 1.5; color: var(--muted);
}
.truth b.healthy { color: var(--green); }
.truth b.suspect { color: var(--orange); }
.truth b.defect { color: var(--red); }
.truth b.unknown { color: var(--muted); }
.fine { margin-top: 8px; font-size: 8.5px; line-height: 1.45; color: #6d829c; }
.dock {
  position: fixed; left: 0; right: 0; bottom: 0; z-index: 10;
  display: flex; flex-direction: column; gap: 7px;
  padding: 26px 18px 14px;
  background: linear-gradient(180deg, rgba(6, 11, 20, 0), rgba(6, 11, 20, .9) 38%, rgba(6, 11, 20, .99));
  pointer-events: none;
}
.dock-head { display: flex; justify-content: space-between; align-items: center; gap: 10px; }
.live-note { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 9.5px; color: var(--muted); white-space: nowrap; }
.slab-strip { display: flex; gap: 6px; overflow-x: auto; padding: 2px 0; pointer-events: auto; scrollbar-width: thin; }
.slab {
  display: inline-flex; align-items: center; gap: 6px; white-space: nowrap;
  padding: 6px 9px; border-radius: 10px; background: rgba(11, 22, 38, .82);
  font-size: 10px;
}
.slab .dot { width: 7px; height: 7px; border-radius: 50%; background: var(--green); box-shadow: 0 0 8px rgba(6, 214, 160, .55); flex: none; }
.slab.suspect .dot { background: var(--orange); box-shadow: 0 0 8px rgba(255, 183, 3, .55); }
.slab.defect .dot { background: var(--red); box-shadow: 0 0 8px rgba(239, 71, 111, .65); }
.slab.unknown .dot { background: var(--muted); box-shadow: none; }
.slab b { font-size: 10.5px; font-weight: 700; }
.slab em { font-style: normal; font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 9px; color: var(--muted); }
.slab.active { border-color: var(--cyan); background: rgba(76, 201, 240, .13); box-shadow: inset 0 0 0 1px rgba(76, 201, 240, .28); }
.slab.active em { color: #c9f5ff; }
.transport { display: flex; gap: 6px; align-items: center; flex-wrap: wrap; pointer-events: auto; }
.hint { margin-left: auto; font-size: 9.5px; color: #7d93ae; }
.tooltip {
  position: fixed; z-index: 20; max-width: 250px; padding: 8px 10px;
  border: 1px solid rgba(76, 201, 240, .4); border-radius: 9px;
  background: rgba(7, 16, 28, .96); font-size: 10.5px; line-height: 1.45;
  box-shadow: 0 12px 34px rgba(0, 0, 0, .55); pointer-events: none;
  white-space: pre-line;
}
.tooltip b { color: var(--cyan); }
@media (max-width: 900px) {
  .panel { top: 56px; left: 10px; right: 10px; width: auto; max-height: none; padding: 9px 12px; border-radius: 13px; }
  .panel .lede, .panel .flags, .panel .fine, .panel .modes, .panel .truth, .panel .gauge-cap, .panel .eyebrow { display: none; }
  .panel h1 { font-size: 15px; margin: 1px 0 3px; }
  .verdict-row { margin: 0 0 5px; }
  .score { font-size: 20px; }
  .gauge { height: 6px; }
  .stats { grid-template-columns: repeat(3, 1fr); gap: 5px 8px; margin: 6px 0 0; }
  .stats .stat:nth-child(n+4) { display: none; }
  .hint { display: none; }
  .brand-sub { display: none; }
  .topbar { padding: 10px 12px; align-items: center; }
  .brand strong { font-size: 12px; letter-spacing: .14em; }
  .viewbar .chip { padding: 5px 8px; font-size: 10px; }
  .dock { padding: 20px 10px 10px; }
  .slab { padding: 5px 8px; }
  .slab em { display: none; }
}
@media (max-width: 620px) {
  .viewbar .view { display: none; }
  #labels, #reset { display: none; }
  .panel { top: 52px; }
  .brand strong { font-size: 11px; }
  .transport { gap: 5px; }
  .transport button, .transport select { padding: 7px 9px; font-size: 10.5px; }
  .slab b { font-size: 9.5px; }
  .live-note { display: none; }
}
"##;

const LAB_BODY: &str = r##"
<canvas id="scene" aria-label="Interactive 3D granite NDT laboratory"></canvas>
<div class="scanlines" aria-hidden="true"></div>
<header class="topbar">
  <div class="brand">
    <span class="pulse">●</span>
    <div><strong>GRANITE LAB</strong><span class="brand-sub">impact-modal NDT · digital twin</span></div>
  </div>
  <div class="viewbar" role="group" aria-label="camera views">
    <button type="button" class="chip view active" data-view="overview">Overview</button>
    <button type="button" class="chip view" data-view="plate">Slab</button>
    <button type="button" class="chip view" data-view="sensor">Sensor</button>
    <span style="width:1px;height:18px;background:var(--line);margin:0 3px"></span>
    <button type="button" class="chip" id="zoom-in" title="zoom in">＋</button>
    <button type="button" class="chip" id="zoom-out" title="zoom out">－</button>
  </div>
</header>
<aside class="panel" id="panel">
  <div class="eyebrow">Loaded slab</div>
  <h1 id="p-title">—</h1>
  <p class="lede" id="p-lede"></p>
  <div class="verdict-row">
    <span class="badge healthy" id="p-badge">…</span>
    <span class="score" id="p-score">0</span>
    <span class="score-cap" id="p-conf">/100</span>
  </div>
  <div class="gauge"><div id="p-gauge"></div><i style="left:30%"></i><i style="left:55%"></i></div>
  <div class="gauge-cap"><span>healthy</span><span>suspect</span><span>defect</span></div>
  <div class="stats" id="p-stats"></div>
  <div class="modes" id="p-modes"></div>
  <div class="flags"><div class="eyebrow">Why</div><ul id="p-flags"></ul></div>
  <div class="truth" id="p-truth"></div>
  <div class="fine">Plate deflection is exaggerated ×~40 and the oscillation is slowed; modal frequencies, damping and the verdict come from the Rust analysis of this capture.</div>
</aside>
<footer class="dock">
  <div class="dock-head">
    <span class="eyebrow">Slab library · click a tile in the scene or here</span>
    <span class="live-note" id="live-note">tap 1 · 0.00 s</span>
  </div>
  <div class="slab-strip" id="slab-strip"></div>
  <div class="transport">
    <button type="button" id="tap" class="primary">⚡ Tap now</button>
    <button type="button" id="play">⏸ Pause</button>
    <select id="speed" aria-label="speed">
      <option value="0.5">0.5×</option>
      <option value="1" selected>1×</option>
      <option value="2">2×</option>
    </select>
    <button type="button" id="sound" title="enable sound">🔇</button>
    <button type="button" id="labels" aria-pressed="true">Labels</button>
    <button type="button" id="reset">↺ View</button>
    <span class="hint">drag to orbit · wheel zoom · click a slab to load · click the plate to tap · <kbd>space</kbd> pause</span>
  </div>
</footer>
<div class="tooltip" id="tooltip" hidden></div>
"##;

const LAB_JS: &str = r##"
'use strict';
const scenes = LAB.scenes;
const $ = (id) => document.getElementById(id);
const canvas = $('scene');
const ctx = canvas.getContext('2d', { alpha: false });
const ui = {
  title: $('p-title'), lede: $('p-lede'), badge: $('p-badge'), score: $('p-score'),
  conf: $('p-conf'), gauge: $('p-gauge'), stats: $('p-stats'), flags: $('p-flags'),
  truth: $('p-truth'), modes: $('p-modes'), strip: $('slab-strip'), live: $('live-note'),
  play: $('play'), speed: $('speed'), sound: $('sound'), labels: $('labels'),
  tooltip: $('tooltip')
};

let W = 1, H = 1, DPR = 1, viewX = 0, viewY = 0, focal = 900;
let eye = [0, 0, 0], vright = [1, 0, 0], vup = [0, 1, 0], vfwd = [0, 0, 1];
let queue = [], overlay = [], labels = [], rackHits = [], groupQueue = null;
let hovered = -1, dragging = null, plateHit = null;
let current = null, vib = [];
let last = 0, lastHud = 0;
const DEFLECT = 18;
const SHAPES = [[1, 1], [2, 1], [1, 2], [2, 2], [3, 1], [1, 3], [3, 2], [2, 3], [3, 3], [4, 1], [4, 2], [2, 4]];
const SHORT = { healthy: 'healthy', 'healthy-noisy': 'noisy', 'lossy-link': 'lossy', crack: 'crack', delamination: 'delam', void: 'void', 'thickness-plus': 'thick+', overloaded: 'overload' };

const cam = { yaw: -1.15, pitch: 0.36, dist: 268, tx: 2, ty: 12, tz: 60 };
const goal = { yaw: cam.yaw, pitch: cam.pitch, dist: cam.dist, tx: cam.tx, ty: cam.ty, tz: cam.tz };
const state = {
  clock: 0, playing: true, speed: 1, sound: false, solo: -1,
  tapAt: -99, lastTap: -99, taps: 0, labels: true
};

const VIEWS = {
  overview: { yaw: -1.15, pitch: 0.36, dist: 268, tx: 2, ty: 12, tz: 60 },
  plate: { yaw: -1.02, pitch: 0.62, dist: 152, tx: -2, ty: 2, tz: 59 },
  sensor: { yaw: -1.9, pitch: 0.26, dist: 146, tx: -32, ty: -10, tz: 72 }
};

const clamp = (value, lo, hi) => Math.max(lo, Math.min(hi, value));
const rad = (deg) => (deg * Math.PI) / 180;

function resize() {
  DPR = Math.min(2, window.devicePixelRatio || 1);
  W = window.innerWidth;
  H = window.innerHeight;
  canvas.width = Math.round(W * DPR);
  canvas.height = Math.round(H * DPR);
  canvas.style.width = W + 'px';
  canvas.style.height = H + 'px';
  ctx.setTransform(DPR, 0, 0, DPR, 0, 0);
}
window.addEventListener('resize', resize);

function updateCamera() {
  const k = 0.11;
  cam.yaw += (goal.yaw - cam.yaw) * k;
  cam.pitch += (goal.pitch - cam.pitch) * k;
  cam.dist += (goal.dist - cam.dist) * k;
  cam.tx += (goal.tx - cam.tx) * k;
  cam.ty += (goal.ty - cam.ty) * k;
  cam.tz += (goal.tz - cam.tz) * k;

  const cp = Math.cos(cam.pitch), sp = Math.sin(cam.pitch);
  eye = [
    cam.tx + cam.dist * cp * Math.cos(cam.yaw),
    cam.ty + cam.dist * cp * Math.sin(cam.yaw),
    cam.tz + cam.dist * sp
  ];
  vfwd = norm([cam.tx - eye[0], cam.ty - eye[1], cam.tz - eye[2]]);
  vright = norm(cross(vfwd, [0, 0, 1]));
  vup = cross(vright, vfwd);
  focal = Math.max(W * 0.72, H * 0.5);
  viewX = W / 2 - (W > 980 ? 140 : 0);
  viewY = H / 2 + (W > 900 ? -18 : 22);
}

function sub(a, b) { return [a[0] - b[0], a[1] - b[1], a[2] - b[2]]; }
function dot(a, b) { return a[0] * b[0] + a[1] * b[1] + a[2] * b[2]; }
function cross(a, b) {
  return [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
}
function norm(a) {
  const len = Math.hypot(a[0], a[1], a[2]) || 1;
  return [a[0] / len, a[1] / len, a[2] / len];
}
function project(p) {
  if (!isFinite(p[0]) || !isFinite(p[1]) || !isFinite(p[2])) return null;
  const d = sub(p, eye);
  const z = dot(d, vfwd);
  if (z < 8) return null;
  return [viewX + dot(d, vright) * focal / z, viewY - dot(d, vup) * focal / z, z];
}

function qface(points, fill, edge, lw, forceZ) {
  const pp = [];
  let z = 0;
  for (const p of points) {
    const s = project(p);
    if (!s) return null;
    pp.push(s);
    z += s[2];
  }
  z /= points.length;
  if (forceZ !== undefined) z = forceZ;
  queue.push({
    z,
    draw() {
      ctx.beginPath();
      ctx.moveTo(pp[0][0], pp[0][1]);
      for (let i = 1; i < pp.length; i++) ctx.lineTo(pp[i][0], pp[i][1]);
      ctx.closePath();
      ctx.fillStyle = fill;
      ctx.fill();
      if (edge) {
        ctx.strokeStyle = edge;
        ctx.lineWidth = lw || 1;
        ctx.stroke();
      }
    }
  });
  return pp;
}

function qline(a, b, color, width, forceZ) {
  const p = project(a), q = project(b);
  if (!p || !q) return;
  const z = forceZ !== undefined ? forceZ : (p[2] + q[2]) / 2;
  (groupQueue || queue).push({
    z,
    draw() {
      ctx.strokeStyle = color;
      ctx.lineWidth = width || 1.5;
      ctx.lineCap = 'round';
      ctx.beginPath();
      ctx.moveTo(p[0], p[1]);
      ctx.lineTo(q[0], q[1]);
      ctx.stroke();
    }
  });
}

function qpoly(points, color, width, forceZ) {
  const pp = [];
  let z = 0;
  for (const p of points) {
    const s = project(p);
    if (!s) return;
    pp.push(s);
    z += s[2];
  }
  z /= points.length;
  if (forceZ !== undefined) z += forceZ;
  (groupQueue || queue).push({
    z,
    draw() {
      ctx.strokeStyle = color;
      ctx.lineWidth = width || 1.4;
      ctx.beginPath();
      ctx.moveTo(pp[0][0], pp[0][1]);
      for (let i = 1; i < pp.length; i++) ctx.lineTo(pp[i][0], pp[i][1]);
      ctx.stroke();
    }
  });
}

function qdot(p, radius, color, glow) {
  const s = project(p);
  if (!s) return;
  const r = Math.max(1.1, radius * focal / s[2]);
  (groupQueue || queue).push({
    z: s[2],
    draw() {
      if (glow) {
        ctx.shadowColor = color;
        ctx.shadowBlur = glow;
      }
      ctx.fillStyle = color;
      ctx.beginPath();
      ctx.arc(s[0], s[1], r, 0, Math.PI * 2);
      ctx.fill();
      ctx.shadowBlur = 0;
    }
  });
}

function shade(color, factor) {
  return 'rgb(' + Math.min(255, Math.round(color[0] * factor)) + ',' +
    Math.min(255, Math.round(color[1] * factor)) + ',' +
    Math.min(255, Math.round(color[2] * factor)) + ')';
}
function mix(a, b, t) {
  const k = isFinite(t) ? clamp(t, 0, 1) : 0;
  return 'rgb(' + Math.round(a[0] + (b[0] - a[0]) * k) + ',' +
    Math.round(a[1] + (b[1] - a[1]) * k) + ',' +
    Math.round(a[2] + (b[2] - a[2]) * k) + ')';
}

function box(cx, cy, z0, w, d, h, color, edge, flat) {
  const x0 = cx - w / 2, x1 = cx + w / 2;
  const y0 = cy - d / 2, y1 = cy + d / 2;
  const z1 = z0 + h;
  const top = flat ? 'rgb(' + color.join(',') + ')' : shade(color, 1.1);
  const front = flat ? top : shade(color, 0.83);
  const back = flat ? top : shade(color, 0.55);
  const side = flat ? top : shade(color, 0.67);
  qface([[x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]], top, edge);
  qface([[x0, y0, z0], [x1, y0, z0], [x1, y0, z1], [x0, y0, z1]], front, edge);
  qface([[x1, y0, z0], [x1, y1, z0], [x1, y1, z1], [x1, y0, z1]], side, edge);
  qface([[x0, y1, z0], [x1, y1, z0], [x1, y1, z1], [x0, y1, z1]], back, edge);
  qface([[x0, y0, z0], [x0, y1, z0], [x0, y1, z1], [x0, y0, z1]], side, edge);
}

function glowDot(x, y, r, color, blur) {
  ctx.save();
  ctx.shadowColor = color;
  ctx.shadowBlur = blur || 8;
  ctx.fillStyle = color;
  ctx.beginPath();
  ctx.arc(x, y, r, 0, Math.PI * 2);
  ctx.fill();
  ctx.restore();
}

function rrect(x, y, w, h, r) {
  ctx.beginPath();
  ctx.moveTo(x + r, y);
  ctx.arcTo(x + w, y, x + w, y + h, r);
  ctx.arcTo(x + w, y + h, x, y + h, r);
  ctx.arcTo(x, y + h, x, y, r);
  ctx.arcTo(x, y, x + w, y, r);
  ctx.closePath();
}

function background() {
  const g = ctx.createLinearGradient(0, 0, 0, H);
  g.addColorStop(0, '#071120');
  g.addColorStop(0.55, '#0a1728');
  g.addColorStop(1, '#040a13');
  ctx.fillStyle = g;
  ctx.fillRect(0, 0, W, H);
  const pool = ctx.createRadialGradient(viewX, viewY + 60, 20, viewX, viewY + 60, Math.max(W, H) * 0.55);
  pool.addColorStop(0, 'rgba(76, 201, 240, .10)');
  pool.addColorStop(1, 'rgba(76, 201, 240, 0)');
  ctx.fillStyle = pool;
  ctx.fillRect(0, 0, W, H);
}

function floorGrid() {
  ctx.lineWidth = 1;
  for (let x = -220; x <= 220; x += 20) {
    const a = project([x, -140, 0]), b = project([x, 135, 0]);
    if (!a || !b) continue;
    ctx.strokeStyle = x % 100 === 0 ? 'rgba(76, 201, 240, .10)' : 'rgba(76, 201, 240, .04)';
    ctx.beginPath();
    ctx.moveTo(a[0], a[1]);
    ctx.lineTo(b[0], b[1]);
    ctx.stroke();
  }
  for (let y = -140; y <= 135; y += 20) {
    const a = project([-220, y, 0]), b = project([220, y, 0]);
    if (!a || !b) continue;
    ctx.strokeStyle = y % 100 === 0 ? 'rgba(76, 201, 240, .10)' : 'rgba(76, 201, 240, .04)';
    ctx.beginPath();
    ctx.moveTo(a[0], a[1]);
    ctx.lineTo(b[0], b[1]);
    ctx.stroke();
  }
}

function drawWall() {
  qface([[-260, 135, 0], [260, 135, 0], [260, 135, 175], [-260, 135, 175]], '#081422', null, 0, 99999);
  for (let x = -240; x <= 240; x += 40) {
    qline([x, 134.4, 0], [x, 134.4, 175], 'rgba(76, 201, 240, .045)', 1, 99998);
  }
  for (let z = 20; z <= 160; z += 20) {
    qline([-260, 134.4, z], [260, 134.4, z], 'rgba(76, 201, 240, .035)', 1, 99998);
  }
}

const benchSpeckles = [];
const plateSpeckles = [];
let rng = 0x51ab13;
function rnd() {
  rng = (rng * 1664525 + 1013904223) >>> 0;
  return rng / 4294967296;
}
for (let i = 0; i < 130; i++) benchSpeckles.push([-82 + rnd() * 164, -38 + rnd() * 104]);
for (let i = 0; i < 140; i++) plateSpeckles.push([rnd(), rnd()]);

function drawBench() {
  box(-76, -24, 0, 8, 8, 47, [20, 25, 34]);
  box(76, -24, 0, 8, 8, 47, [20, 25, 34]);
  box(-76, 54, 0, 8, 8, 47, [20, 25, 34]);
  box(76, 54, 0, 8, 8, 47, [20, 25, 34]);
  box(0, 15, 47, 170, 110, 5, [30, 37, 49], 'rgba(120, 160, 200, .10)');
  for (const [x, y] of benchSpeckles) {
    qdot([x, y, 52.03], 0.32, 'rgba(150, 178, 206, .24)');
  }
  // rubber supports
  box(0, -14, 52, 76, 6, 5, [30, 34, 42]);
  box(0, 14, 52, 76, 6, 5, [30, 34, 42]);
}

function drawRack() {
  const steel = [44, 50, 62];
  box(-62, 92, 0, 5, 7, 106, steel);
  box(62, 92, 0, 5, 7, 106, steel);
  box(0, 92, 68, 124, 8, 2, steel);
  box(0, 92, 86, 124, 8, 2, steel);
  box(0, 92, 104, 124, 8, 2, steel);
  labels.push({ p: [0, 92, 113], text: 'SLAB LIBRARY · click a tile', color: '#7d93ae', small: true });

  scenes.forEach((scene, index) => {
    const col = index % 4;
    const row = Math.floor(index / 4);
    const x = -45 + col * 30;
    const z = 70 + row * 16;
    const active = current && current.id === scene.id;
    const hover = hovered === index;
    const y = active ? 90 : 92;
    const base = active ? [62, 112, 142] :
      scene.condition_class === 'defect' ? [96, 66, 72] :
      scene.condition_class === 'suspect' ? [104, 88, 58] : [82, 94, 106];
    box(x, y, z, 24, 3, 13, base,
      active ? '#4cc9f0' : hover ? '#9fd4ee' : '#0d1724');
    const ledColor = scene.condition_class === 'defect' ? '#ef476f' :
      scene.condition_class === 'suspect' ? '#ffb703' : '#06d6a0';
    const led = project([x + 9, y - 2.2, z + 11]);
    if (led) overlay.push(() => glowDot(led[0], led[1], 2.2, ledColor, 9));
    const pp = [
      [x - 12, y - 1.6, z], [x + 12, y - 1.6, z],
      [x + 12, y - 1.6, z + 13], [x - 12, y - 1.6, z + 13]
    ].map(project);
    if (pp.every(Boolean)) rackHits.push({ index, poly: pp });
  });
}

function bezier(a, b, c, t) {
  const u = 1 - t;
  return [
    u * u * a[0] + 2 * u * t * b[0] + t * t * c[0],
    u * u * a[1] + 2 * u * t * b[1] + t * t * c[1],
    u * u * a[2] + 2 * u * t * b[2] + t * t * c[2]
  ];
}

function cable(a, b, c, color) {
  const pts = [];
  for (let i = 0; i <= 12; i++) pts.push(bezier(a, b, c, i / 12));
  qpoly(pts, color, 2.4);
  qpoly(pts, 'rgba(80, 120, 160, .35)', 1);
}

const SCREEN = {
  bl: [43.5, 46.3, 54.5],
  br: [76.5, 46.3, 54.5],
  tr: [76.5, 50.6, 73.5],
  tl: [43.5, 50.6, 73.5]
};

function drawRig() {
  // --- laser vibrometer on a tripod
  qline([-56, -30, 60], [-66, -40, 0], '#1b2635', 2.2);
  qline([-56, -30, 60], [-46, -40, 0], '#1b2635', 2.2);
  qline([-56, -30, 60], [-56, -20, 0], '#1b2635', 2.2);
  box(-56, -30, 58, 4, 4, 22, [42, 48, 60]);
  box(-56, -30, 78, 15, 11, 9, [38, 44, 56], 'rgba(140, 170, 200, .18)');
  box(-48, -26, 81, 4, 6, 4, [24, 28, 36]);
  const led = project([-48, -26, 84]);
  if (led) overlay.push(() => glowDot(led[0], led[1], 2.4, '#ef476f', 10));

  labels.push({ p: [-56, -30, 95], text: 'laser + photodiode', color: '#ef476f' });

  // --- transimpedance amplifier + anti-alias filter
  box(-44, 18, 52, 14, 10, 4, [38, 44, 56], 'rgba(140, 180, 210, .14)');
  box(-50, 18, 56, 3, 3, 2, [176, 182, 192]);
  labels.push({ p: [-44, 18, 61], text: 'TIA + filter', color: '#8fa6c4', small: true });

  // --- solenoid tapper gantry
  box(-46, 40, 52, 5, 5, 58, [46, 52, 64]);
  box(-28, 40, 106, 36, 5, 4, [46, 52, 64]);
  box(-10, 22, 106, 5, 36, 4, [46, 52, 64]);
  box(-10, 4, 98, 9, 9, 8, [40, 46, 58], 'rgba(140, 170, 200, .16)');

  // --- electronics on the bench
  box(-26, 40, 52, 20, 14, 2, [28, 78, 56], 'rgba(120, 220, 180, .18)');
  box(-26, 40, 54, 8, 8, 1.6, [18, 22, 28]);
  box(8, 40, 52, 26, 16, 2, [26, 70, 54], 'rgba(120, 220, 180, .18)');
  box(2, 38, 54, 10, 10, 2, [186, 192, 202], 'rgba(230, 240, 250, .2)');
  box(18, 40, 54, 3, 8, 1.2, [206, 176, 86]);
  box(-2, 31, 53, 6, 4, 3, [150, 156, 166]);
  const espLed = project([14, 36, 55]);
  if (espLed) {
    overlay.push(() => {
      const on = (Math.sin(state.clock * 5.5) + 1) / 2;
      glowDot(espLed[0], espLed[1], 1.6 + on, 'rgba(76, 201, 240, ' + (0.35 + on * 0.6) + ')', 8);
    });
  }
  labels.push({ p: [-26, 45, 58], text: 'ADC · PCM1808', color: '#06d6a0' });
  labels.push({ p: [10, 45, 58], text: 'ESP32 · UDP', color: '#4cc9f0' });

  // --- laptop
  box(60, 30, 52, 34, 24, 2, [30, 36, 48], 'rgba(120, 160, 200, .12)');
  box(60, 44, 52, 8, 6, 2, [40, 46, 58]);
  qface([SCREEN.bl, SCREEN.br, SCREEN.tr, SCREEN.tl], '#081521', '#2a4058', 1.2);
  qface([SCREEN.bl, SCREEN.br, [SCREEN.br[0], SCREEN.br[1] + 1.6, SCREEN.br[2]],
    [SCREEN.bl[0], SCREEN.bl[1] + 1.6, SCREEN.bl[2]]], '#0e1c2c', null);
  labels.push({ p: [60, 16, 57], text: 'analysis PC', color: '#4cc9f0' });

  // --- cables
  cable([-56, -28, 53], [-60, -2, 54], [-52, 18, 53], '#1b2635');
  cable([-37, 20, 53], [-34, 32, 55], [-30, 42, 53], '#1b2635');
  cable([-16, 42, 53], [-6, 40, 55], [4, 42, 53], '#1b2635');
  cable([20, 40, 53], [38, 44, 54], [48, 36, 53], '#1b2635');

  // --- Wi-Fi path to the laptop
  const linkA = [12, 46, 58], linkB = [34, 44, 74], linkC = [52, 34, 57];
  const link = [];
  for (let i = 0; i <= 20; i++) link.push(bezier(linkA, linkB, linkC, i / 20));
  qpoly(link, 'rgba(76, 201, 240, .10)', 1.2);
  const ant = project([20, 44, 61]);
  if (ant) {
    overlay.push(() => {
      for (let k = 0; k < 3; k++) {
        const phase = ((state.clock * 0.9) + k / 3) % 1;
        ctx.strokeStyle = 'rgba(76, 201, 240, ' + (0.55 * (1 - phase)) + ')';
        ctx.lineWidth = 1.6;
        ctx.beginPath();
        ctx.arc(ant[0], ant[1], 7 + phase * 25, -1.25, 0.35);
        ctx.stroke();
      }
    });
  }

  const lossy = current && current.loss > 0.004;
  for (let i = 0; i < 10; i++) {
    const u = ((state.clock * 0.55) + i / 10) % 1;
    if (u > 0.94) continue;
    const lost = lossy && (i === 3 || i === 8);
    if (lost && u > 0.62) continue;
    const p = bezier(linkA, linkB, linkC, u);
    const color = lost ? (u < 0.45 ? '#ffb703' : '#ef476f') : '#4cc9f0';
    qdot(p, lost ? 0.85 : 0.62, color, lost ? 14 : 9);
  }
}

const PLATE_LIGHT = norm([-0.35, -0.55, 0.76]);

function mixArray(a, b, t) {
  return [
    a[0] + (b[0] - a[0]) * t,
    a[1] + (b[1] - a[1]) * t,
    a[2] + (b[2] - a[2]) * t
  ];
}

function plateColor(lift, dzdx, dzdy, grain) {
  const normal = norm([-dzdx, -dzdy, 1]);
  const lambert = Math.max(0, normal[0] * PLATE_LIGHT[0] + normal[1] * PLATE_LIGHT[1] + normal[2] * PLATE_LIGHT[2]);
  const light = clamp(0.56 + 0.56 * lambert, 0.42, 1.22);
  const tint = clamp(Math.abs(lift) / 4.0, 0, 1);
  let base = lift >= 0 ? mixArray([118, 124, 134], [96, 190, 228], 0.34 * tint) : mixArray([118, 124, 134], [232, 152, 110], 0.26 * tint);
  const g = 0.965 + 0.06 * grain;
  base = [base[0] * g, base[1] * g, base[2] * g];
  return 'rgb(' + Math.min(255, Math.round(base[0] * light)) + ',' +
    Math.min(255, Math.round(base[1] * light)) + ',' +
    Math.min(255, Math.round(base[2] * light)) + ')';
}

function surfaceZ(u, v, time) {
  return 59 + Math.max(0, vibration(u, v, time));
}

function drawPlate() {
  const NX = 26, NY = 18;
  const top = [];
  for (let j = 0; j <= NY; j++) {
    const row = [];
    for (let i = 0; i <= NX; i++) {
      const u = i / NX, v = j / NY;
      const lift = Math.max(0, vibration(u, v, state.clock));
      row.push([-32 + 64 * u, -22 + 44 * v, 59 + lift, lift]);
    }
    top.push(row);
  }
  const dx = 64 / NX, dy = 44 / NY;
  for (let j = 0; j < NY; j++) {
    for (let i = 0; i < NX; i++) {
      const p00 = top[j][i], p10 = top[j][i + 1], p11 = top[j + 1][i + 1], p01 = top[j + 1][i];
      const lift = (p00[3] + p10[3] + p11[3] + p01[3]) / 4;
      const dzdx = ((p10[3] + p11[3]) - (p00[3] + p01[3])) / (2 * dx);
      const dzdy = ((p01[3] + p11[3]) - (p00[3] + p10[3])) / (2 * dy);
      const gu = (i + 0.5) / NX, gv = (j + 0.5) / NY;
      const grain = 0.5 + 0.5 * Math.sin(gu * 23.1 + gv * 11.7) * Math.sin(gu * 9.3 - gv * 31.1);
      const fill = plateColor(lift, dzdx, dzdy, grain);
      qface([[p00[0], p00[1], p00[2]], [p10[0], p10[1], p10[2]],
        [p11[0], p11[1], p11[2]], [p01[0], p01[1], p01[2]]],
        fill, fill, 1);
    }
  }
  plateHit = [top[0][0], top[0][NX], top[NY][NX], top[NY][0]].map(project);
  if (!plateHit.every(Boolean)) plateHit = null;
  const rim = [];
  for (let i = 0; i <= NX; i++) rim.push([top[0][i][0], top[0][i][1], top[0][i][2] + 0.06]);
  for (let j = 1; j <= NY; j++) rim.push([top[j][NX][0], top[j][NX][1], top[j][NX][2] + 0.06]);
  for (let i = NX - 1; i >= 0; i--) rim.push([top[NY][i][0], top[NY][i][1], top[NY][i][2] + 0.06]);
  for (let j = NY - 1; j >= 0; j--) rim.push([top[j][0][0], top[j][0][1], top[j][0][2] + 0.06]);
  qpoly(rim, 'rgba(196, 228, 250, .42)', 1.1, -6);

  const side = [64, 68, 76];
  const sideShade = 'rgb(' + side.join(',') + ')';
  for (let i = 0; i < NX; i++) {
    const a = top[0][i], b = top[0][i + 1];
    qface([[a[0], -22, a[2]], [b[0], -22, b[2]], [b[0], -22, 57], [a[0], -22, 57]], sideShade, 'rgba(8, 14, 22, .35)', 0.5);
    const c = top[NY][i], d = top[NY][i + 1];
    qface([[c[0], 22, c[2]], [d[0], 22, d[2]], [d[0], 22, 57], [c[0], 22, 57]], shade(side, 0.7), 'rgba(8, 14, 22, .35)', 0.5);
  }
  for (let j = 0; j < NY; j++) {
    const a = top[j][0], b = top[j + 1][0];
    qface([[-32, a[1], a[2]], [-32, b[1], b[2]], [-32, b[1], 57], [-32, a[1], 57]], shade(side, 0.8), 'rgba(8, 14, 22, .35)', 0.5);
    const c = top[j][NX], d = top[j + 1][NX];
    qface([[32, c[1], c[2]], [32, d[1], d[2]], [32, d[1], 57], [32, c[1], 57]], shade(side, 0.8), 'rgba(8, 14, 22, .35)', 0.5);
  }
  for (const [u, v] of plateSpeckles) {
    qdot([-32 + 64 * u, -22 + 44 * v, surfaceZ(u, v, state.clock) + 0.05],
      0.28, 'rgba(226, 238, 250, .30)');
  }
  labels.push({ p: [-39, -26, 61], text: 'granite slab', color: '#9fe8ff', small: true });

  const dt = state.clock - state.tapAt;
  if (dt >= 0 && dt < 0.55) {
    const s = project([-10, 4, surfaceZ(0.34375, 0.5909, state.clock)]);
    if (s) {
      overlay.push(() => {
        const t = dt / 0.55;
        ctx.strokeStyle = 'rgba(76, 201, 240, ' + (1 - t) * 0.55 + ')';
        ctx.lineWidth = 2;
        ctx.beginPath();
        ctx.arc(s[0], s[1], 8 + 74 * t, 0, Math.PI * 2);
        ctx.stroke();
      });
    }
  }
}


function drawForeground() {
  const uSpot = 0.4375, vSpot = 0.545;
  const spot = [-4, 2, surfaceZ(uSpot, vSpot, state.clock)];
  const head = [-49, -25, 82];
  qline(head, spot, 'rgba(239, 71, 111, .16)', 8);
  qline(head, spot, 'rgba(255, 105, 125, .72)', 2);
  qdot(spot, 1.5, 'rgba(255, 150, 165, .95)', 18);
  const sp = project(spot);
  if (sp && isFinite(sp[0]) && isFinite(sp[1])) {
    overlay.push(() => {
      const g = ctx.createRadialGradient(sp[0], sp[1], 1, sp[0], sp[1], 26);
      g.addColorStop(0, 'rgba(255, 120, 140, .55)');
      g.addColorStop(1, 'rgba(255, 120, 140, 0)');
      ctx.fillStyle = g;
      ctx.beginPath();
      ctx.arc(sp[0], sp[1], 26, 0, Math.PI * 2);
      ctx.fill();
    });
  }

  const down = clamp(1 - Math.abs(state.clock - state.tapAt) / 0.09, 0, 1);
  const tip = 70 - 10.5 * down;
  qline([-10, 4, 98], [-10, 4, tip], '#aab4c2', 3);
  box(-10, 4, tip - 2, 3.4, 3.4, 3, [160, 166, 176]);
}

function drawLabels() {
  if (!state.labels) return;
  ctx.textAlign = 'left';
  ctx.textBaseline = 'middle';
  for (const label of labels) {
    const s = project(label.p);
    if (!s) continue;
    if (s[0] < 8 || s[0] > W - 8 || s[1] < 8 || s[1] > H - 8) continue;
    const font = label.small ? 9 : 10;
    ctx.font = '600 ' + font + 'px ui-monospace, SFMono-Regular, Menlo, monospace';
    const width = ctx.measureText(label.text).width;
    const bx = s[0] - width / 2 - 5;
    const by = s[1] - 9;
    rrect(bx, by, width + 10, 17, 5);
    ctx.fillStyle = 'rgba(5, 12, 21, .74)';
    ctx.fill();
    ctx.strokeStyle = label.color === '#4cc9f0' ? 'rgba(76, 201, 240, .45)' : 'rgba(120, 160, 200, .16)';
    ctx.lineWidth = 1;
    ctx.stroke();
    ctx.fillStyle = label.color || '#c9daee';
    ctx.fillText(label.text, bx + 5, by + 8.5);
    ctx.fillStyle = 'rgba(76, 201, 240, .7)';
    ctx.beginPath();
    ctx.arc(s[0], s[1], 1.8, 0, Math.PI * 2);
    ctx.fill();
  }
}

function drawScreen() {
  const p = [SCREEN.bl, SCREEN.br, SCREEN.tr, SCREEN.tl].map(project);
  if (p.some((v) => !v)) return;
  const screenWidth = Math.hypot(p[1][0] - p[0][0], p[1][1] - p[0][1]);
  ctx.save();
  ctx.beginPath();
  ctx.moveTo(p[0][0], p[0][1]);
  ctx.lineTo(p[1][0], p[1][1]);
  ctx.lineTo(p[2][0], p[2][1]);
  ctx.lineTo(p[3][0], p[3][1]);
  ctx.closePath();
  ctx.clip();
  const bil = (u, v) => {
    const bx = p[0][0] + (p[1][0] - p[0][0]) * u;
    const by = p[0][1] + (p[1][1] - p[0][1]) * u;
    const tx = p[3][0] + (p[2][0] - p[3][0]) * u;
    const ty = p[3][1] + (p[2][1] - p[3][1]) * u;
    return [bx + (tx - bx) * v, by + (ty - by) * v];
  };
  for (let k = 1; k < 4; k++) {
    const a = bil(0.06, k / 4), b = bil(0.94, k / 4);
    ctx.strokeStyle = 'rgba(76, 201, 240, .08)';
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(a[0], a[1]);
    ctx.lineTo(b[0], b[1]);
    ctx.stroke();
  }
  const now = state.clock;
  ctx.beginPath();
  for (let i = 0; i <= 96; i++) {
    const t = now - 0.34 + 0.34 * (i / 96);
    const value = vibration(0.32, 0.42, t);
    const u = 0.06 + 0.88 * (i / 96);
    const v = 0.5 + clamp(value / 3.4, -0.32, 0.32);
    const q = bil(u, v);
    if (i === 0) ctx.moveTo(q[0], q[1]);
    else ctx.lineTo(q[0], q[1]);
  }
  ctx.strokeStyle = '#4cc9f0';
  ctx.lineWidth = 1.4;
  ctx.shadowColor = '#4cc9f0';
  ctx.shadowBlur = 6;
  ctx.stroke();
  ctx.shadowBlur = 0;
  const small = screenWidth < 115;
  const title = bil(0.08, 0.82);
  ctx.font = '700 ' + clamp(screenWidth * 0.075, 6, 9).toFixed(1) + 'px ui-monospace, SFMono-Regular, Menlo, monospace';
  ctx.fillStyle = '#9fe8ff';
  ctx.textAlign = 'left';
  ctx.textBaseline = 'middle';
  ctx.fillText(small ? 'LIVE' : 'GRANITE NDT · LIVE', title[0], title[1]);
  const foot = bil(0.08, 0.14);
  ctx.font = '600 ' + clamp(screenWidth * 0.068, 6, 8).toFixed(1) + 'px ui-monospace, SFMono-Regular, Menlo, monospace';
  ctx.fillStyle = current ? (current.condition_class === 'defect' ? '#ff8fa5' :
    current.condition_class === 'suspect' ? '#ffd166' : '#7ef0cb') : '#7d93ae';
  ctx.fillText(current ? (small ? current.predicted : current.predicted + ' · ' + current.score.toFixed(1)) : '', foot[0], foot[1]);
  ctx.restore();
}

function drawOverlay() {
  for (const fn of overlay) fn();
  drawScreen();
  drawLabels();
}

function vibration(u, v, time) {
  const dt = time - state.tapAt;
  if (dt < 0 || dt > 4.5) return 0;
  let sum = 0;
  for (let i = 0; i < vib.length; i++) {
    if (state.solo >= 0 && state.solo !== i) continue;
    const mode = vib[i];
    sum += mode.a * Math.exp(-dt / mode.tau) * Math.sin(mode.w * dt) *
      Math.sin(mode.shape[0] * Math.PI * u) * Math.sin(mode.shape[1] * Math.PI * v);
  }
  return sum * DEFLECT;
}

function buildVib(scene) {
  const maxAmp = Math.max(1e-9, ...scene.modes.map((mode) => Number.isFinite(mode.amplitude) ? mode.amplitude : 0));
  const list = scene.modes.map((mode, index) => {
    const freq = Number.isFinite(mode.frequency_hz) ? mode.frequency_hz : 300;
    const zeta = Number.isFinite(mode.damping_ratio) ? mode.damping_ratio : 0.003;
    const amp = Number.isFinite(mode.amplitude) ? mode.amplitude : 0;
    return {
      w: 2 * Math.PI * clamp(freq / 90, 1.6, 5.2),
      tau: clamp(8.0 / (Math.max(0.0004, zeta) * 2 * Math.PI * Math.max(40, freq)), 0.32, 1.6),
      a: (0.28 + 0.72 * (amp / maxAmp)) / (1 + 0.2 * index),
      shape: SHAPES[index % SHAPES.length]
    };
  });
  const total = list.reduce((sum, mode) => sum + mode.a, 0) || 1;
  for (const mode of list) mode.a /= total;
  return list;
}

function triggerTap(delay) {
  const wait = delay === undefined ? 0.04 : delay;
  state.tapAt = state.clock + wait;
  state.lastTap = state.clock;
  state.taps += 1;
  if (state.sound) playTap(wait / state.speed);
}

function render() {
  ctx.setTransform(DPR, 0, 0, DPR, 0, 0);
  background();
  floorGrid();
  queue = [];
  overlay = [];
  labels = [];
  rackHits = [];
  plateHit = null;
  drawWall();
  drawBench();
  drawRack();
  drawRig();
  const back = queue;
  queue = [];
  groupQueue = queue;
  drawPlate();
  const plate = queue;
  queue = [];
  groupQueue = queue;
  drawForeground();
  const front = queue;
  groupQueue = null;
  for (const list of [back, plate, front]) {
    list.sort((a, b) => b.z - a.z);
    for (const item of list) item.draw();
  }
  drawOverlay();
}

function frame(now) {
  const dt = Math.min(0.06, Math.max(0, (now - last) / 1000));
  last = now;
  if (state.playing) state.clock += dt * state.speed;
  if (state.playing && state.clock - state.lastTap > 1.25) triggerTap(0.05);
  updateCamera();
  render();
  if (now - lastHud > 120) {
    lastHud = now;
    const dtTap = Math.max(0, state.clock - state.tapAt);
    ui.live.textContent = 'tap ' + state.taps + ' · ' + (state.clock - state.tapAt >= 0 ? dtTap.toFixed(2) + ' s after impact' : 'tapping…');
  }
  requestAnimationFrame(frame);
}

function fmt(value, digits) {
  if (!isFinite(value)) return '—';
  return value.toFixed(digits);
}

function renderPanel() {
  const s = current;
  if (!s) return;
  ui.title.textContent = s.title;
  ui.lede.textContent = s.description;
  ui.badge.textContent = s.predicted;
  ui.badge.className = 'badge ' + s.condition_class;
  ui.score.textContent = fmt(s.score, 1);
  ui.conf.textContent = '/100 · conf ' + Math.round(s.confidence * 100) + '%';
  ui.gauge.style.width = clamp(s.score, 0, 100) + '%';
  ui.gauge.style.background = s.condition_class === 'defect' ? 'linear-gradient(90deg,#ff8fa5,#ef476f)' :
    s.condition_class === 'suspect' ? 'linear-gradient(90deg,#ffd166,#ffb703)' :
    'linear-gradient(90deg,#06d6a0,#4cc9f0)';
  const rows = [
    ['f₁', fmt(s.f1, 1) + ' Hz'],
    ['ζ mean', fmt(s.damping, 5)],
    ['T60', fmt(s.t60, 2) + ' s'],
    ['modes · taps', s.mode_count + ' · ' + s.event_count],
    ['link loss', fmt(s.loss * 100, 2) + '%'],
    ['slab, mm', Math.round(s.length_mm) + '×' + Math.round(s.width_mm) + '×' + Math.round(s.thickness_mm)]
  ];
  ui.stats.innerHTML = rows.map(([key, value]) =>
    '<div class="stat"><b>' + value + '</b><span>' + key + '</span></div>').join('');
  const flags = s.flags.slice(0, 3);
  ui.flags.innerHTML = '';
  if (!flags.length) {
    const li = document.createElement('li');
    li.textContent = 'all modal features within the reference envelope';
    ui.flags.appendChild(li);
  } else {
    for (const flag of flags) {
      const li = document.createElement('li');
      li.textContent = flag;
      ui.flags.appendChild(li);
    }
  }
  const expectedClass = s.expected.toLowerCase();
  ui.truth.innerHTML = 'Expected <b class="' + expectedClass + '">' + s.expected +
    '</b> · predicted <b class="' + s.condition_class + '">' + s.predicted +
    '</b><br>' + escapeHtml(s.truth);
  renderModes();
  updateStrip();
}

function escapeHtml(text) {
  return String(text).replace(/[&<>"']/g, (ch) => ({
    '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;'
  }[ch]));
}

function renderModes() {
  ui.modes.innerHTML = '';
  if (!current) return;
  current.modes.slice(0, 6).forEach((mode, index) => {
    const button = document.createElement('button');
    button.type = 'button';
    button.className = 'mode';
    button.innerHTML = '<b>M' + (index + 1) + '</b>' + fmt(mode.frequency_hz, 0) + ' Hz';
    button.title = 'solo this mode shape';
    button.addEventListener('click', () => {
      state.solo = state.solo === index ? -1 : index;
      updateModeChips();
    });
    ui.modes.appendChild(button);
  });
  updateModeChips();
}

function updateModeChips() {
  Array.from(ui.modes.children).forEach((button, index) => {
    button.classList.toggle('active', state.solo === index);
  });
}

function updateStrip() {
  Array.from(ui.strip.children).forEach((button) => {
    button.classList.toggle('active', !!current && button.dataset.id === current.id);
  });
}

function selectScene(id, tap) {
  const scene = scenes.find((candidate) => candidate.id === id);
  if (!scene) return;
  current = scene;
  vib = buildVib(scene);
  state.solo = -1;
  renderPanel();
  if (tap === false) {
    state.tapAt = state.clock - 5;
    state.lastTap = state.clock;
  } else {
    state.lastTap = state.clock - 1.0;
    triggerTap(0.18);
  }
}

function buildChrome() {
  ui.strip.innerHTML = '';
  scenes.forEach((scene) => {
    const button = document.createElement('button');
    button.type = 'button';
    button.className = 'slab ' + scene.condition_class;
    button.dataset.id = scene.id;
    button.title = scene.title + ' — expected ' + scene.expected;
    button.innerHTML = '<span class="dot"></span><b>' + escapeHtml(SHORT[scene.id] || scene.id) + '</b><em>' +
      fmt(scene.score, 0) + '</em>';
    button.addEventListener('click', () => selectScene(scene.id));
    ui.strip.appendChild(button);
  });
}

function setPlaying(playing) {
  state.playing = playing;
  ui.play.textContent = playing ? '⏸ Pause' : '▶ Play';
}

function applyView(name) {
  const view = VIEWS[name] || VIEWS.overview;
  Object.assign(goal, view);
  Array.from(document.querySelectorAll('.view')).forEach((button) => {
    button.classList.toggle('active', button.dataset.view === name);
  });
}

// ---------------------------------------------------------------- audio
let audio = null, master = null;
function ensureAudio() {
  if (!audio) {
    const AC = window.AudioContext || window.webkitAudioContext;
    if (!AC) return false;
    audio = new AC();
    master = audio.createGain();
    master.gain.value = 0.5;
    const comp = audio.createDynamicsCompressor();
    master.connect(comp);
    comp.connect(audio.destination);
  }
  if (audio.state === 'suspended') audio.resume();
  return true;
}

function playTap(wait) {
  if (!state.sound || !current || !ensureAudio()) return;
  const start = audio.currentTime + Math.max(0, wait || 0);
  const length = Math.floor(audio.sampleRate * 0.03);
  const buffer = audio.createBuffer(1, length, audio.sampleRate);
  const channel = buffer.getChannelData(0);
  for (let i = 0; i < length; i++) {
    channel[i] = (Math.random() * 2 - 1) * Math.exp(-i / (length * 0.2)) * 0.45;
  }
  const noise = audio.createBufferSource();
  noise.buffer = buffer;
  const noiseGain = audio.createGain();
  noiseGain.gain.value = 0.35;
  noise.connect(noiseGain);
  noiseGain.connect(master);
  noise.start(start);

  const maxAmp = Math.max(1e-9, ...current.modes.map((mode) => Number.isFinite(mode.amplitude) ? mode.amplitude : 0));
  current.modes.forEach((mode, index) => {
    if (mode.frequency_hz > 9000) return;
    const amp = 0.13 * (0.3 + 0.7 * mode.amplitude / maxAmp) / (1 + 0.15 * index);
    const tau = clamp(2.0 / (Math.max(0.0005, mode.damping_ratio) * 2 * Math.PI *
      Math.max(40, mode.frequency_hz)), 0.08, 1.2);
    const osc = audio.createOscillator();
    osc.type = 'sine';
    osc.frequency.value = mode.frequency_hz;
    const gain = audio.createGain();
    gain.gain.setValueAtTime(0.0001, start);
    gain.gain.exponentialRampToValueAtTime(Math.max(0.001, amp), start + 0.006);
    gain.gain.exponentialRampToValueAtTime(0.0001, start + tau * 3.2);
    osc.connect(gain);
    gain.connect(master);
    osc.start(start);
    osc.stop(start + tau * 3.2 + 0.05);
  });
}

// ---------------------------------------------------------------- input
function pointInPoly(x, y, poly) {
  let inside = false;
  for (let i = 0, j = poly.length - 1; i < poly.length; j = i++) {
    const xi = poly[i][0], yi = poly[i][1];
    const xj = poly[j][0], yj = poly[j][1];
    if (((yi > y) !== (yj > y)) && (x < (xj - xi) * (y - yi) / (yj - yi) + xi)) inside = !inside;
  }
  return inside;
}

function pickRack(x, y) {
  for (let i = rackHits.length - 1; i >= 0; i--) {
    if (pointInPoly(x, y, rackHits[i].poly)) return rackHits[i].index;
  }
  return -1;
}

function showTooltip(scene, x, y) {
  ui.tooltip.hidden = false;
  ui.tooltip.innerHTML = '<b>' + escapeHtml(scene.id) + '</b> · ' + escapeHtml(scene.title) +
    '\n' + scene.predicted + ' · anomaly ' + fmt(scene.score, 1) + '/100 · f₁ ' + fmt(scene.f1, 1) + ' Hz';
  const width = 250;
  ui.tooltip.style.left = Math.min(W - width, x + 16) + 'px';
  ui.tooltip.style.top = Math.min(H - 70, y + 18) + 'px';
}
function hideTooltip() { ui.tooltip.hidden = true; }

canvas.addEventListener('pointerdown', (event) => {
  try { canvas.setPointerCapture(event.pointerId); } catch (error) {}
  dragging = { x: event.clientX, y: event.clientY, moved: 0 };
  hideTooltip();
});

canvas.addEventListener('pointermove', (event) => {
  if (dragging) {
    const dx = event.clientX - dragging.x;
    const dy = event.clientY - dragging.y;
    dragging.moved += Math.abs(dx) + Math.abs(dy);
    if (dragging.moved > 3) {
      cam.yaw -= dx * 0.005;
      cam.pitch = clamp(cam.pitch + dy * 0.0035, 0.06, 1.1);
      goal.yaw = cam.yaw;
      goal.pitch = cam.pitch;
      canvas.style.cursor = 'grabbing';
    }
    dragging.x = event.clientX;
    dragging.y = event.clientY;
    return;
  }
  const index = pickRack(event.clientX, event.clientY);
  hovered = index;
  const overPlate = plateHit ? pointInPoly(event.clientX, event.clientY, plateHit) : false;
  canvas.style.cursor = index >= 0 || overPlate ? 'pointer' : 'grab';
  if (index >= 0) showTooltip(scenes[index], event.clientX, event.clientY);
  else hideTooltip();
});

canvas.addEventListener('pointerup', (event) => {
  const moved = dragging ? dragging.moved : 0;
  dragging = null;
  canvas.style.cursor = 'grab';
  if (moved < 5) {
    const index = pickRack(event.clientX, event.clientY);
    if (index >= 0) {
      selectScene(scenes[index].id);
    } else if (plateHit && pointInPoly(event.clientX, event.clientY, plateHit)) {
      if (!state.playing) setPlaying(true);
      triggerTap(0.04);
    }
  }
});

canvas.addEventListener('pointerleave', () => {
  dragging = null;
  hovered = -1;
  hideTooltip();
});

canvas.addEventListener('wheel', (event) => {
  event.preventDefault();
  goal.dist = clamp(goal.dist * Math.exp(event.deltaY * 0.0012), 95, 560);
}, { passive: false });

ui.play.addEventListener('click', () => setPlaying(!state.playing));
$('tap').addEventListener('click', () => {
  if (!state.playing) setPlaying(true);
  triggerTap(0.05);
});
ui.speed.addEventListener('change', () => { state.speed = parseFloat(ui.speed.value) || 1; });
ui.sound.addEventListener('click', () => {
  state.sound = !state.sound;
  ui.sound.textContent = state.sound ? '🔊' : '🔇';
  ui.sound.title = state.sound ? 'mute audio' : 'enable sound';
  if (state.sound) {
    ensureAudio();
    if (!state.playing) setPlaying(true);
    triggerTap(0.05);
  }
});
ui.labels.addEventListener('click', () => {
  state.labels = !state.labels;
  ui.labels.setAttribute('aria-pressed', String(state.labels));
  ui.labels.classList.toggle('active', state.labels);
});
$('reset').addEventListener('click', () => applyView('overview'));
$('zoom-in').addEventListener('click', () => { goal.dist = clamp(goal.dist * 0.85, 95, 560); });
$('zoom-out').addEventListener('click', () => { goal.dist = clamp(goal.dist * 1.18, 95, 560); });
Array.from(document.querySelectorAll('.view')).forEach((button) => {
  button.addEventListener('click', () => applyView(button.dataset.view));
});

window.addEventListener('keydown', (event) => {
  const tag = event.target && event.target.tagName;
  if (tag === 'INPUT' || tag === 'SELECT' || tag === 'TEXTAREA') return;
  if (event.code === 'Space') {
    event.preventDefault();
    setPlaying(!state.playing);
  } else if (event.key >= '1' && event.key <= '8') {
    const scene = scenes[Number(event.key) - 1];
    if (scene) selectScene(scene.id);
  } else if (event.key === 'l' || event.key === 'L') {
    ui.labels.click();
  }
});

window.__graniteLab = {
  scenes, state, camera: cam, goal, frame, render,
  select: selectScene, tap: triggerTap, view: applyView
};

resize();
state.labels = W > 760;
ui.labels.classList.toggle('active', state.labels);
ui.labels.setAttribute('aria-pressed', String(state.labels));
if (W < 760) {
  cam.dist = 220;
  goal.dist = 220;
}
buildChrome();
applyView('overview');
selectScene(LAB.selected || (scenes[0] && scenes[0].id), false);
setPlaying(true);
last = performance.now();
triggerTap(0.4);
requestAnimationFrame(frame);
"##;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lab_page_has_every_scene_and_a_balanced_payload() {
        let run = crate::run("healthy").expect("healthy run");
        let crack = crate::run("crack").expect("crack run");
        let scenes = vec![scene_from_run(&run), scene_from_run(&crack)];
        let html = render_lab_html(&scenes, "crack", 1234);
        assert!(html.contains("const LAB = {"));
        assert!(html.contains("\"id\":\"healthy\""));
        assert!(html.contains("\"id\":\"crack\""));
        assert!(html.contains("\"selected\":\"crack\""));
        assert!(html.contains("<script>"));
        assert!(html.contains("id=\"scene\""));
        assert!(html.contains("requestAnimationFrame(frame)"));
        assert!(!html.contains("NaN"));
        let payload = html
            .split("const LAB = ")
            .nth(1)
            .expect("payload")
            .split(";\n")
            .next()
            .expect("payload end");
        assert!(payload.starts_with('{') && payload.ends_with('}'));
        assert_eq!(
            payload.chars().filter(|ch| *ch == '{').count(),
            payload.chars().filter(|ch| *ch == '}').count()
        );
    }
}

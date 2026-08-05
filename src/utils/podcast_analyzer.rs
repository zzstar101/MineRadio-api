use std::{
    cmp::Ordering,
    io::Cursor,
    str::FromStr,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::api::{ApiError, ApiErrorCode, ApiResult};
use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use symphonia::core::{
    audio::GenericAudioBufferRef,
    codecs::audio::{AudioDecoderOptions, CODEC_ID_NULL_AUDIO},
    errors::Error as SymphoniaError,
    formats::{FormatOptions, probe::Hint},
    io::{MediaSourceStream, MediaSourceStreamOptions},
    meta::MetadataOptions,
};
use symphonia::default::{get_codecs, get_probe};

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PodcastAudioFormat {
    Mp3,
    Ogg,
    M4a,
    Flac,
    Wav,
}

impl PodcastAudioFormat {
    fn extension(self) -> &'static str {
        match self {
            Self::Mp3 => "mp3",
            Self::Ogg => "ogg",
            Self::M4a => "m4a",
            Self::Flac => "flac",
            Self::Wav => "wav",
        }
    }
}

impl FromStr for PodcastAudioFormat {
    type Err = ApiError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "mp3" => Ok(Self::Mp3),
            "ogg" => Ok(Self::Ogg),
            "m4a" | "mp4" => Ok(Self::M4a),
            "flac" => Ok(Self::Flac),
            "wav" | "wave" => Ok(Self::Wav),
            _ => Err(ApiError::new(
                ApiErrorCode::BadRequest,
                format!("unsupported audio format: {value}"),
            )),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PodcastDjAnalyzerParams {
    pub format: PodcastAudioFormat,
    pub intro_sec: Option<u32>,
}

pub fn analyze_podcast_dj_beatmap(
    audio: &[u8],
    params: &PodcastDjAnalyzerParams,
) -> ApiResult<PodcastDjBeatMap> {
    if audio.is_empty() {
        return Err(ApiError::new(
            ApiErrorCode::BadRequest,
            "audio data required",
        ));
    }
    let decoded = decode_podcast_dj_bytes(audio, params.format, None).map_err(map_decode_error)?;
    let mut map = if let Some(intro_sec) = params.intro_sec.filter(|value| *value > 0) {
        let frame_limit = clamp_usize(
            ((intro_sec as f64 + 2.0) / decoded.hop_sec.max(0.010)).ceil() as usize,
            1,
            decoded.low_energy.len(),
        );
        let duration = (intro_sec as f64).min(frame_limit as f64 * decoded.hop_sec);
        let mut map = build_beat_map_from_low_energy(
            &decoded.low_energy[..frame_limit],
            &decoded.hit_energy[..frame_limit],
            decoded.hop_sec,
            duration,
        );
        map.partial = Some(true);
        map.partial_until_sec = Some(duration);
        map.full_duration = Some(decoded.duration);
        map.tempo_source = "podcast-dj-intro".to_owned();
        map
    } else {
        build_beat_map_from_low_energy(
            &decoded.low_energy,
            &decoded.hit_energy,
            decoded.hop_sec,
            decoded.duration,
        )
    };
    map.decode = Some(decoded.decode);
    Ok(map)
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PodcastDjBeatMap {
    pub kicks: Vec<f64>,
    pub beats: Vec<PodcastDjBeat>,
    pub pulse_beats: Vec<PodcastDjPulseBeat>,
    pub camera_beats: Vec<PodcastDjBeat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grid_step: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub section_steps: Vec<f64>,
    pub tempo_source: String,
    pub duration: f64,
    pub visual_beat_count: usize,
    pub analyzed_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub partial: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub partial_until_sec: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_duration: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decode: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub debug: Option<Value>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PodcastDjBeat {
    pub time: f64,
    pub strength: f64,
    pub confidence: f64,
    pub impact: f64,
    pub primary: bool,
    pub camera: bool,
    pub pulse: bool,
    pub tone: String,
    pub low: f64,
    pub body: f64,
    pub snap: f64,
    pub mass: f64,
    pub sharpness: f64,
    pub combo: String,
    pub step: f64,
    pub index: usize,
    pub dj: bool,
    pub grid: bool,
    pub kick_only: bool,
    pub server: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sampled: Option<bool>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PodcastDjPulseBeat {
    pub time: f64,
    pub strength: f64,
    pub impact: f64,
    pub combo: String,
    pub low: f64,
    pub body: f64,
    pub snap: f64,
    pub dj: bool,
}

type BeatMap = PodcastDjBeatMap;
type Beat = PodcastDjBeat;
type PulseBeat = PodcastDjPulseBeat;

#[derive(Clone, Debug)]
struct Candidate {
    frame: usize,
    time: f64,
    score: f64,
    low_tone: f64,
    hit_tone: f64,
    low_rel: f64,
    power: f64,
}

#[derive(Clone, Debug)]
struct EnergyDecode {
    low_energy: Vec<f64>,
    hit_energy: Vec<f64>,
    hop_sec: f64,
    duration: f64,
    decode: Value,
}

#[derive(Clone, Debug)]
struct Profile {
    time: f64,
    avg: f64,
    hi: f64,
    activity: f64,
    step: f64,
    anchor: f64,
}

#[derive(Clone, Debug)]
struct PhaseInfo {
    phase: f64,
}

#[derive(Clone, Debug)]
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

fn build_range_beat(
    time: f64,
    step_override: f64,
    grid_index: usize,
    profiles: &[Profile],
) -> Beat {
    let profile = profile_at(profiles, time);
    let slot = grid_index % 4;
    let mut combo = match slot {
        0 => "downbeat",
        1 => "push",
        2 => "drop",
        _ => "rebound",
    }
    .to_owned();
    let section_energy =
        clamp01((profile.avg - 0.055) / 0.54) * clamp_range(profile.activity, 0.30, 1.10);
    let motion = ((grid_index as f64 * 1.618 + profile.avg * 9.7).sin() * 0.5
        + (grid_index as f64 * 0.317).sin() * 0.28)
        * (0.08 + section_energy * 0.17);
    let rel = clamp01(
        0.12 + section_energy * 0.70 + motion + if combo == "downbeat" { 0.060 } else { 0.0 },
    );
    if rel > 0.82 && combo != "downbeat" {
        combo = "accent".to_owned();
    }
    let visual_rel = if rel > 0.78 {
        0.78 + (rel - 0.78) * 0.50
    } else {
        rel
    };
    let combo_lift = if combo == "downbeat" {
        0.10 * section_energy
    } else if combo == "drop" {
        0.050 * section_energy
    } else if combo == "accent" {
        0.075 * section_energy
    } else {
        0.0
    };
    let impact = clamp_range(
        0.026 + visual_rel.powf(1.48) * (0.42 + profile.hi * 0.34) + combo_lift,
        0.020,
        0.90,
    );
    let strength = clamp_range(
        0.15 + visual_rel.powf(1.02) * 0.66 + combo_lift * 0.68,
        0.12,
        0.93,
    );
    let camera_active = impact >= 0.105 || (combo == "downbeat" && section_energy >= 0.16);
    let low = clamp_range(
        0.50 + visual_rel * 0.32
            + if combo == "downbeat" {
                0.050 * section_energy
            } else {
                0.0
            }
            - if combo == "accent" { 0.12 } else { 0.0 },
        0.42,
        0.90,
    );
    let body = clamp_range(
        0.06 + visual_rel * 0.15
            + if combo == "push" {
                0.22 * section_energy
            } else {
                0.0
            }
            + if combo == "drop" {
                0.30 * section_energy
            } else {
                0.0
            },
        0.045,
        0.56,
    );
    let snap = clamp_range(
        0.025
            + visual_rel * 0.035
            + if combo == "accent" {
                0.40 * section_energy
            } else {
                0.0
            }
            + if combo == "rebound" {
                0.12 * section_energy
            } else {
                0.0
            },
        0.02,
        0.62,
    );
    Beat {
        time,
        strength,
        confidence: 0.68 + visual_rel * 0.22,
        impact,
        primary: camera_active,
        camera: camera_active,
        pulse: impact > 0.16 || (combo == "downbeat" && section_energy >= 0.24),
        tone: "podcast-dj-server-range-grid".to_owned(),
        low,
        body,
        snap,
        mass: clamp_range(low * 0.72 + visual_rel.powf(1.22) * 0.24, 0.36, 0.94),
        sharpness: if combo == "accent" { 0.20 } else { 0.08 },
        combo,
        step: step_override.max(0.01),
        index: grid_index,
        dj: true,
        grid: true,
        kick_only: true,
        server: true,
        sampled: Some(true),
    }
}

fn profile_at(profiles: &[Profile], time: f64) -> Profile {
    if profiles.len() <= 1 {
        return profiles.first().cloned().unwrap_or(Profile {
            time,
            avg: 0.16,
            hi: 0.55,
            activity: 0.5,
            step: 0.50,
            anchor: 0.0,
        });
    }
    let mut prev = profiles[0].clone();
    let mut next = profiles[profiles.len() - 1].clone();
    for profile in profiles {
        if profile.time <= time {
            prev = profile.clone();
        }
        if profile.time >= time {
            next = profile.clone();
            break;
        }
    }
    if (prev.time - next.time).abs() < f64::EPSILON {
        return prev;
    }
    let mix = clamp01((time - prev.time) / (next.time - prev.time).max(1.0));
    Profile {
        time,
        avg: prev.avg + (next.avg - prev.avg) * mix,
        hi: prev.hi + (next.hi - prev.hi) * mix,
        activity: prev.activity + (next.activity - prev.activity) * mix,
        step: prev.step + (next.step - prev.step) * mix,
        anchor: prev.anchor + (next.anchor - prev.anchor) * mix,
    }
}

fn phase_from_map(map: &BeatMap, base_step: f64) -> PhaseInfo {
    let step = clamp_range(map.grid_step.unwrap_or(base_step), 0.32, 0.86);
    let beats = if !map.camera_beats.is_empty() {
        map.camera_beats.clone()
    } else {
        map.beats.clone()
    }
    .into_iter()
    .filter(|beat| beat.time.is_finite() && beat.time > 0.35)
    .collect::<Vec<_>>();
    if beats.is_empty() {
        return PhaseInfo { phase: 0.0 };
    }
    let mut sx = 0.0;
    let mut sy = 0.0;
    let mut total = 0.0;
    for beat in beats {
        let impact = if beat.impact.is_finite() {
            beat.impact
        } else {
            0.3
        };
        let weight = 0.20 + impact.max(0.0).powf(1.45);
        let phase = ((beat.time % step) + step) % step;
        let angle = phase / step * std::f64::consts::TAU;
        sx += angle.cos() * weight;
        sy += angle.sin() * weight;
        total += weight;
    }
    if total <= 0.0 {
        return PhaseInfo {
            phase: beats_first_phase(map, step),
        };
    }
    let mut angle = (sy / total).atan2(sx / total);
    if angle < 0.0 {
        angle += std::f64::consts::TAU;
    }
    PhaseInfo {
        phase: angle / std::f64::consts::TAU * step,
    }
}

fn beats_first_phase(map: &BeatMap, step: f64) -> f64 {
    map.camera_beats
        .first()
        .or_else(|| map.beats.first())
        .map(|beat| ((beat.time % step) + step) % step)
        .unwrap_or(0.0)
}

fn build_beat_map_from_low_energy(
    low_energy: &[f64],
    hit_energy: &[f64],
    hop_sec: f64,
    duration_sec: f64,
) -> BeatMap {
    let n_frames = low_energy.len().min(hit_energy.len());
    if n_frames < 20 {
        return empty_map(duration_sec, "podcast-dj-server-empty", None, None);
    }

    let low_floor = percentile(low_energy, 0.22, 16000)
        .unwrap_or(0.001)
        .max(0.0004);
    let low_mid =
        (percentile(low_energy, 0.58, 16000).unwrap_or(low_floor + 0.0002)).max(low_floor + 0.0002);
    let low_ref =
        (percentile(low_energy, 0.86, 16000).unwrap_or(low_mid + 0.0002)).max(low_mid + 0.0002);
    let low_ceil =
        (percentile(low_energy, 0.96, 16000).unwrap_or(low_ref + 0.0004)).max(low_ref + 0.0004);
    let hit_ref = percentile(hit_energy, 0.86, 16000)
        .unwrap_or(0.001)
        .max(0.0004);

    let mut onset = vec![0.0; n_frames];
    for index in 4..n_frames {
        let prev = low_energy[index - 1] * 0.62
            + low_energy[index - 2] * 0.28
            + low_energy[index - 3] * 0.10;
        let low_rise = (low_energy[index] - prev).max(0.0);
        let wide_rise = (((low_energy[index] + low_energy[index - 1]) * 0.5)
            - ((low_energy[index - 3] + low_energy[index - 4]) * 0.5))
            .max(0.0);
        let peak_rise = (hit_energy[index] - hit_energy[index - 2] * 0.84).max(0.0);
        onset[index] = low_rise * 1.72 + wide_rise * 0.86 + peak_rise * 0.10;
    }

    let win_n = (0.82 / hop_sec).round().max(52.0) as usize;
    let min_frame_gap = (0.215 / hop_sec).round().max(18.0) as usize;
    let mut candidates: Vec<Candidate> = Vec::new();
    let mut sum_o = 0.0;
    let mut sq_o = 0.0;
    for value in onset.iter().take(win_n) {
        sum_o += *value;
        sq_o += value * value;
    }
    for frame in (win_n + 4)..(n_frames.saturating_sub(4)) {
        let mean = sum_o / win_n as f64;
        let std = (sq_o / win_n as f64 - mean * mean).max(0.0).sqrt();
        let threshold = mean + std * 1.66 + low_ref * 0.0038;
        let onset_value = onset[frame];
        if onset_value > threshold
            && onset_value >= onset[frame - 1]
            && onset_value > onset[frame + 1]
        {
            let mut peak_frame = frame;
            let mut peak_score = onset_value + low_energy[frame] * 0.10;
            for probe in frame.saturating_sub(2)..=(frame + 3).min(n_frames - 1) {
                let probe_score = onset[probe] + low_energy[probe] * 0.10;
                if probe_score > peak_score {
                    peak_score = probe_score;
                    peak_frame = probe;
                }
            }
            let low_tone = (band_at(low_energy, peak_frame) / low_ref).min(2.6);
            let hit_tone = (band_at(hit_energy, peak_frame) / hit_ref).min(2.6);
            let low_rel = clamp01(
                (band_at(low_energy, peak_frame) - low_floor) / (low_ceil - low_floor).max(0.0001),
            );
            let score =
                (onset_value - threshold) / (std + mean * 0.38 + low_ref * 0.012).max(0.0006);
            if score > 0.16 && (low_tone > 0.32 || low_rel > 0.22 || hit_tone > 0.52) {
                let mut candidate = Candidate {
                    frame: peak_frame,
                    time: peak_frame as f64 * hop_sec,
                    score,
                    low_tone,
                    hit_tone,
                    low_rel,
                    power: 0.0,
                };
                candidate.power = candidate.score * 0.56
                    + clamp01((candidate.low_tone - 0.22) / 1.42).powf(0.82) * 0.34
                    + candidate.hit_tone.min(1.5) * 0.08
                    + candidate.low_rel * 0.10;
                if let Some(last) = candidates.last_mut() {
                    if candidate.frame.saturating_sub(last.frame) < min_frame_gap {
                        if candidate.power > last.power {
                            *last = candidate;
                        }
                    } else {
                        candidates.push(candidate);
                    }
                } else {
                    candidates.push(candidate);
                }
            }
        }
        let old = onset[frame - win_n];
        let next = onset[frame];
        sum_o += next - old;
        sq_o += next * next - old * old;
    }

    if candidates.is_empty() {
        return empty_map(
            duration_sec.max(n_frames as f64 * hop_sec),
            "podcast-dj-server-empty",
            None,
            None,
        );
    }

    let powers = candidates
        .iter()
        .map(|candidate| candidate.power)
        .collect::<Vec<_>>();
    let p30 = percentile(&powers, 0.30, 16000).unwrap_or(0.001);
    let p50 = percentile(&powers, 0.50, 16000).unwrap_or(p30);
    let p90 = percentile(&powers, 0.90, 16000)
        .unwrap_or(p50 + 0.001)
        .max(p50 + 0.001);
    let p96 = percentile(&powers, 0.965, 16000)
        .unwrap_or(p90 + 0.001)
        .max(p90 + 0.001);
    let mut strong = candidates
        .iter()
        .filter(|candidate| candidate.power >= p50 && candidate.low_tone > 0.34)
        .cloned()
        .collect::<Vec<_>>();
    if strong.len() < 16 {
        strong = candidates.clone();
    }

    let mut global_step = estimate_step(&strong)
        .or_else(|| estimate_step(&candidates))
        .unwrap_or(0.50);
    global_step = clamp_range(global_step, 0.32, 0.86);

    let mut phase_source = strong
        .iter()
        .filter(|candidate| candidate.time < duration_sec.min(180.0))
        .take(72)
        .cloned()
        .collect::<Vec<_>>();
    if phase_source.is_empty() {
        if let Some(first) = strong.first().cloned() {
            phase_source.push(first);
        }
    }
    let mut best_anchor = phase_source
        .first()
        .map(|candidate| candidate.time)
        .unwrap_or(0.0);
    let mut best_anchor_score = f64::NEG_INFINITY;
    for candidate in &phase_source {
        let score = score_phase(candidate.time, global_step, &candidates, p30, duration_sec);
        if score > best_anchor_score {
            best_anchor_score = score;
            best_anchor = candidate.time;
        }
    }
    let half_step = global_step * 0.5;
    if half_step >= 0.31 {
        let half_score = score_phase(best_anchor, half_step, &candidates, p30, duration_sec);
        if half_score > best_anchor_score * 1.04 {
            global_step = half_step;
        }
    }
    let mut anchor = best_anchor;
    while anchor - global_step > 0.05 {
        anchor -= global_step;
    }

    let duration = if duration_sec > 0.0 {
        duration_sec
    } else {
        n_frames as f64 * hop_sec
    };
    let section_len = if duration > 3600.0 { 96.0 } else { 72.0 };
    let section_count = (duration / section_len).ceil().max(1.0) as usize;
    let mut section_steps = Vec::with_capacity(section_count);
    for section_index in 0..section_count {
        let start = section_index as f64 * section_len;
        let end = duration.min(start + section_len);
        let segment = strong
            .iter()
            .filter(|candidate| candidate.time >= start && candidate.time < end)
            .cloned()
            .collect::<Vec<_>>();
        let previous = section_steps.last().copied().unwrap_or(global_step);
        let mut local = estimate_step(&segment).unwrap_or(previous);
        local = clamp_range(local, previous * 0.94, previous * 1.06);
        local = clamp_range(local, global_step * 0.86, global_step * 1.14);
        section_steps.push(local * 0.30 + previous * 0.70);
    }

    let mut beats = Vec::new();
    let mut grid_index = 0_usize;
    let mut cursor_index = 0_usize;
    let mut grid_time = anchor;
    while grid_time < duration - 0.04 {
        let local_step = step_at(&section_steps, section_len, grid_time, global_step);
        let win_sec = clamp_range(local_step * 0.20, 0.060, 0.135);
        while cursor_index < candidates.len() && candidates[cursor_index].time < grid_time - win_sec
        {
            cursor_index += 1;
        }
        let best_candidate = nearest_candidate(&candidates, grid_time, win_sec, cursor_index);
        let grid_frame = clamp_usize((grid_time / hop_sec).round() as usize, 0, n_frames - 1);
        let grid_low = band_at(low_energy, grid_frame);
        let grid_hit = band_at(hit_energy, grid_frame);
        let grid_low_tone = (grid_low / low_ref).min(2.6);
        let grid_hit_tone = (grid_hit / hit_ref).min(2.6);
        let low_tone = best_candidate
            .map(|candidate| (grid_low_tone * 0.62).max(candidate.low_tone))
            .unwrap_or(grid_low_tone);
        let hit_tone = best_candidate
            .map(|candidate| (grid_hit_tone * 0.62).max(candidate.hit_tone))
            .unwrap_or(grid_hit_tone);
        let dist_penalty = best_candidate
            .map(|candidate| 1.0 - ((candidate.time - grid_time).abs() / win_sec).min(1.0) * 0.26)
            .unwrap_or(0.54);
        let base_power = best_candidate
            .map(|candidate| candidate.power * dist_penalty)
            .unwrap_or(grid_low_tone * 0.25 + grid_hit_tone * 0.06);
        let power_rel = clamp01((base_power - p30 * 0.78) / (p96 - p30 * 0.78).max(0.001));
        let low_rel = clamp01((grid_low - low_floor) / (low_ceil - low_floor).max(0.0001));
        let kick_rel =
            clamp01(power_rel * 0.74 + low_rel * 0.22 + clamp01((hit_tone - 0.26) / 1.70) * 0.04);
        let soft_grid = (best_candidate.is_none() && low_rel < 0.20) || kick_rel < 0.16;
        let slot = grid_index % 4;
        let mut combo = match slot {
            0 => "downbeat",
            1 => "push",
            2 => "drop",
            _ => "rebound",
        }
        .to_owned();
        if kick_rel > 0.84 && combo != "downbeat" {
            combo = "accent".to_owned();
        }
        let visual_rel = if kick_rel > 0.76 {
            0.76 + (kick_rel - 0.76) * 0.52
        } else {
            kick_rel
        };
        let down_lift = if combo == "downbeat" {
            if visual_rel > 0.18 {
                0.016 + visual_rel * 0.036
            } else {
                visual_rel * 0.028
            }
        } else {
            0.0
        };
        let section_gate = clamp01((kick_rel - 0.10) / 0.58);
        let mut impact = clamp_range(
            0.022 + visual_rel.powf(1.62) * 0.86 + down_lift,
            0.020,
            0.88,
        );
        let mut strength = clamp_range(
            0.13 + visual_rel.powf(1.12) * 0.68 + down_lift * 0.70,
            0.12,
            0.93,
        );
        if soft_grid {
            let soft_mul = if combo == "downbeat" { 0.48 } else { 0.30 };
            impact *= soft_mul;
            strength *= 0.58 + section_gate * 0.22;
        }
        let timing_pull = best_candidate
            .map(|_| 0.24 + clamp01((kick_rel - 0.25) / 0.65) * 0.46)
            .unwrap_or(0.0);
        let source_time = best_candidate
            .map(|candidate| grid_time * (1.0 - timing_pull) + candidate.time * timing_pull)
            .unwrap_or(grid_time);
        let camera_active = impact >= 0.13
            || (combo == "downbeat" && kick_rel >= 0.14)
            || (best_candidate.is_some() && kick_rel >= 0.18);
        let low_mix = clamp_range(
            0.52 + visual_rel * 0.32 + low_tone * 0.035
                - if combo == "accent" { 0.10 } else { 0.0 },
            0.42,
            0.90,
        );
        let body_mix = clamp_range(
            0.060
                + visual_rel * 0.12
                + if combo == "push" { 0.18 } else { 0.0 }
                + if combo == "drop" { 0.24 } else { 0.0 },
            0.035,
            0.54,
        );
        let snap_mix = clamp_range(
            0.026
                + if combo == "accent" { 0.40 } else { 0.0 }
                + if combo == "rebound" { 0.08 } else { 0.0 }
                + visual_rel * 0.038,
            0.015,
            0.62,
        );
        beats.push(Beat {
            time: source_time,
            strength,
            confidence: clamp_range(
                0.46 + kick_rel * 0.43
                    + if best_candidate.is_some() {
                        0.08
                    } else {
                        -0.03
                    },
                0.44,
                0.99,
            ),
            impact,
            primary: camera_active,
            camera: camera_active,
            pulse: impact > 0.16 || (combo == "downbeat" && kick_rel >= 0.18),
            tone: "podcast-dj-server-low-grid".to_owned(),
            low: low_mix,
            body: body_mix,
            snap: snap_mix,
            mass: clamp_range(low_mix * 0.72 + visual_rel.powf(1.22) * 0.24, 0.36, 0.94),
            sharpness: clamp_range(snap_mix * 1.18, 0.03, 0.28),
            combo,
            step: local_step,
            index: beats.len(),
            dj: true,
            grid: true,
            kick_only: true,
            server: true,
            sampled: None,
        });
        grid_index += 1;
        grid_time += local_step;
    }

    let camera_beats = beats
        .iter()
        .filter(|beat| beat.camera)
        .cloned()
        .collect::<Vec<_>>();
    let pulse_beats = beats
        .iter()
        .filter(|beat| beat.pulse && (beat.impact >= 0.16 || beat.combo == "downbeat"))
        .map(|beat| PulseBeat {
            time: beat.time,
            strength: beat.strength,
            impact: beat.impact,
            combo: beat.combo.clone(),
            low: beat.low,
            body: beat.body,
            snap: beat.snap,
            dj: true,
        })
        .collect::<Vec<_>>();

    BeatMap {
        kicks: beats.iter().map(|beat| beat.time).collect(),
        beats,
        pulse_beats,
        camera_beats: camera_beats.clone(),
        grid_step: Some(global_step),
        section_steps,
        tempo_source: "podcast-dj-server-low-offline".to_owned(),
        duration,
        visual_beat_count: camera_beats.len(),
        analyzed_at: now_millis(),
        partial: None,
        partial_until_sec: None,
        full_duration: None,
        decode: None,
        debug: Some(json!({
            "candidates": candidates.len(),
            "hopSec": hop_sec,
            "lowRef": low_ref,
            "step": global_step,
        })),
    }
}

fn step_at(section_steps: &[f64], section_len: f64, time: f64, global_step: f64) -> f64 {
    if section_steps.is_empty() {
        return global_step;
    }
    let index = clamp_usize(
        (time / section_len).floor() as usize,
        0,
        section_steps.len() - 1,
    );
    section_steps[index]
}

fn nearest_candidate<'a>(
    candidates: &'a [Candidate],
    center: f64,
    window_sec: f64,
    start_index: usize,
) -> Option<&'a Candidate> {
    let mut best = None;
    let mut best_score = f64::NEG_INFINITY;
    let mut index = start_index;
    while index < candidates.len() && candidates[index].time < center - window_sec {
        index += 1;
    }
    while index < candidates.len() && candidates[index].time <= center + window_sec {
        let distance = (candidates[index].time - center).abs();
        let score = candidates[index].power * (1.0 - distance / window_sec.max(0.001) * 0.42);
        if score > best_score {
            best = Some(&candidates[index]);
            best_score = score;
        }
        index += 1;
    }
    best
}

fn score_phase(
    anchor_time: f64,
    step: f64,
    candidates: &[Candidate],
    p30: f64,
    duration_sec: f64,
) -> f64 {
    let mut start = anchor_time;
    while start - step > 0.05 {
        start -= step;
    }
    let end = duration_sec.min(180.0);
    let win = clamp_range(step * 0.18, 0.055, 0.125);
    let mut score = 0.0;
    let mut count = 0_u64;
    let mut cursor = 0_usize;
    let mut grid_time = start;
    while grid_time < end {
        while cursor < candidates.len() && candidates[cursor].time < grid_time - win {
            cursor += 1;
        }
        let mut best_score = 0.0;
        let mut probe = cursor;
        while probe < candidates.len() && candidates[probe].time <= grid_time + win {
            let distance = (candidates[probe].time - grid_time).abs();
            let value = candidates[probe].power * (1.0 - distance / win * 0.44);
            if value > best_score {
                best_score = value;
            }
            probe += 1;
        }
        score += if best_score > 0.0 {
            best_score
        } else {
            -p30 * 0.08
        };
        count += 1;
        grid_time += step;
    }
    if count == 0 {
        f64::NEG_INFINITY
    } else {
        score / count as f64
    }
}

fn estimate_step(candidates: &[Candidate]) -> Option<f64> {
    if candidates.len() < 3 {
        return None;
    }
    let bin = 0.006;
    let mut histogram = std::collections::HashMap::<i64, f64>::new();
    let mut median_gaps = Vec::new();
    for anchor_index in 0..candidates.len() {
        for other_index in (anchor_index + 1)..candidates.len().min(anchor_index + 10) {
            let raw_gap = candidates[other_index].time - candidates[anchor_index].time;
            if raw_gap < 0.24 {
                continue;
            }
            if raw_gap > 2.55 {
                break;
            }
            for div in 1..=6 {
                let gap = raw_gap / div as f64;
                if gap < 0.31 {
                    break;
                }
                if gap > 0.86 {
                    continue;
                }
                let weight = (candidates[anchor_index].power * candidates[other_index].power)
                    .max(0.001)
                    .sqrt()
                    / ((other_index - anchor_index) as f64 * div as f64).sqrt();
                let key = (gap / bin).round() as i64;
                *histogram.entry(key).or_insert(0.0) += weight;
                median_gaps.push(gap);
            }
        }
    }
    let mut best_key = None;
    let mut best_score = 0.0;
    for (key, _) in &histogram {
        let score = histogram.get(key).copied().unwrap_or(0.0)
            + histogram.get(&(key - 1)).copied().unwrap_or(0.0) * 0.72
            + histogram.get(&(key + 1)).copied().unwrap_or(0.0) * 0.72;
        if score > best_score {
            best_score = score;
            best_key = Some(*key);
        }
    }
    if let Some(key) = best_key {
        return Some(key as f64 * bin);
    }
    median(&median_gaps)
}

fn empty_map(
    duration: f64,
    tempo_source: &str,
    decode: Option<Value>,
    debug: Option<Value>,
) -> BeatMap {
    BeatMap {
        kicks: Vec::new(),
        beats: Vec::new(),
        pulse_beats: Vec::new(),
        camera_beats: Vec::new(),
        grid_step: None,
        section_steps: Vec::new(),
        tempo_source: tempo_source.to_owned(),
        duration: duration.max(0.0),
        visual_beat_count: 0,
        analyzed_at: now_millis(),
        partial: None,
        partial_until_sec: None,
        full_duration: None,
        decode,
        debug,
    }
}

fn band_at(values: &[f64], index: usize) -> f64 {
    let clamped = clamp_usize(index, 0, values.len().saturating_sub(1));
    let left = values[clamp_usize(clamped.saturating_sub(1), 0, values.len().saturating_sub(1))];
    let center = values[clamped];
    let right = values[clamp_usize(
        (clamped + 1).min(values.len().saturating_sub(1)),
        0,
        values.len().saturating_sub(1),
    )];
    (left + center * 2.0 + right) * 0.25
}

fn clamp01(value: f64) -> f64 {
    clamp_range(value, 0.0, 1.0)
}

fn clamp_range(value: f64, min: f64, max: f64) -> f64 {
    value.max(min).min(max)
}

fn clamp_usize(value: usize, min: usize, max: usize) -> usize {
    value.max(min).min(max)
}

fn percentile(values: &[f64], percentile: f64, max_samples: usize) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let sample = if values.len() <= max_samples {
        values.to_vec()
    } else {
        let mut reduced = Vec::with_capacity(max_samples);
        let step = (values.len() - 1) as f64 / (max_samples - 1) as f64;
        for index in 0..max_samples {
            reduced.push(values[((index as f64 * step).floor() as usize).min(values.len() - 1)]);
        }
        reduced
    };
    let mut sample = sample;
    sample.sort_by(|left, right| left.partial_cmp(right).unwrap_or(Ordering::Equal));
    let index = ((sample.len() as f64 * percentile).floor() as usize).min(sample.len() - 1);
    Some(sample[index].max(0.001))
}

fn median(values: &[f64]) -> Option<f64> {
    let mut values = values
        .iter()
        .copied()
        .filter(|value| value.is_finite())
        .collect::<Vec<_>>();
    if values.is_empty() {
        return None;
    }
    values.sort_by(|left, right| left.partial_cmp(right).unwrap_or(Ordering::Equal));
    Some(values[(values.len() as f64 * 0.5).floor() as usize])
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn make_biquad(kind: &str, freq: f64, q: f64, sample_rate: f64) -> Biquad {
    let freq = clamp_range(freq, 8.0, sample_rate * 0.45);
    let w0 = std::f64::consts::TAU * freq / sample_rate;
    let cos = w0.cos();
    let sin = w0.sin();
    let alpha = sin / (2.0 * if q == 0.0 { 0.707 } else { q });
    let (b0, b1, b2) = if kind == "highpass" {
        ((1.0 + cos) * 0.5, -(1.0 + cos), (1.0 + cos) * 0.5)
    } else {
        ((1.0 - cos) * 0.5, 1.0 - cos, (1.0 - cos) * 0.5)
    };
    let a0 = 1.0 + alpha;
    let a1 = -2.0 * cos;
    let a2 = 1.0 - alpha;
    let inv = 1.0 / a0;
    Biquad {
        b0: b0 * inv,
        b1: b1 * inv,
        b2: b2 * inv,
        a1: a1 * inv,
        a2: a2 * inv,
        x1: 0.0,
        x2: 0.0,
        y1: 0.0,
        y2: 0.0,
    }
}

fn run_biquad(state: &mut Biquad, sample: f64) -> f64 {
    let value = state.b0 * sample + state.b1 * state.x1 + state.b2 * state.x2
        - state.a1 * state.y1
        - state.a2 * state.y2;
    state.x2 = state.x1;
    state.x1 = sample;
    state.y2 = state.y1;
    state.y1 = value;
    value
}

fn decode_podcast_dj_bytes(
    audio: &[u8],
    format: PodcastAudioFormat,
    limit_sec: Option<f64>,
) -> anyhow::Result<EnergyDecode> {
    let mut hint = Hint::new();
    hint.with_extension(format.extension());

    let source = Cursor::new(audio.to_vec());
    let stream = MediaSourceStream::new(Box::new(source), MediaSourceStreamOptions::default());
    let mut format = get_probe().probe(
        &hint,
        stream,
        FormatOptions::default(),
        MetadataOptions::default(),
    )?;
    let track = format
        .tracks()
        .iter()
        .find(|track| {
            track
                .codec_params
                .as_ref()
                .and_then(|params| params.audio())
                .is_some_and(|params| params.codec != CODEC_ID_NULL_AUDIO)
        })
        .cloned()
        .context("audio track unavailable")?;
    let track_id = track.id;
    let audio_codec_params = track
        .codec_params
        .as_ref()
        .and_then(|params| params.audio())
        .context("audio codec parameters unavailable")?;
    let mut decoder =
        get_codecs().make_audio_decoder(audio_codec_params, &AudioDecoderOptions::default())?;

    let hop_sec = 0.010;
    let mut state = DecodeState::new(hop_sec);
    let mut chunks = 0_u64;
    let mut decoded_samples = 0_u64;

    loop {
        let packet = match format.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(SymphoniaError::IoError(_)) => break,
            Err(error) => return Err(error.into()),
        };
        if packet.track_id != track_id {
            continue;
        }
        let decoded: GenericAudioBufferRef<'_> = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(SymphoniaError::ResetRequired) => anyhow::bail!("decoder reset required"),
            Err(error) => return Err(error.into()),
        };
        let sample_rate = decoded.spec().rate() as f64;
        let channels = decoded.spec().channels().count();
        let mut samples = vec![0f32; decoded.samples_interleaved()];
        decoded.copy_to_slice_interleaved(samples.as_mut_slice());
        decoded_samples += (samples.len() / channels.max(1)) as u64;
        chunks += 1;
        state.process_interleaved(samples.as_slice(), channels.max(1), sample_rate, limit_sec);
        if state.limit_reached(limit_sec) {
            break;
        }
    }
    state.finish_frame();
    let duration = state.duration();
    let frames = state.low_energy.len();
    let decode = json!({
        "chunks": chunks,
        "decodedSamples": decoded_samples,
        "sampleRate": state.source_sample_rate.unwrap_or_default(),
        "effectiveSampleRate": state.effective_sample_rate.unwrap_or_default(),
        "frames": frames
    });

    Ok(EnergyDecode {
        low_energy: state.low_energy,
        hit_energy: state.hit_energy,
        hop_sec,
        duration,
        decode,
    })
}

fn map_decode_error(error: anyhow::Error) -> ApiError {
    let message = error.to_string();
    let lower = message.to_ascii_lowercase();
    let code = if lower.contains("unsupported") {
        ApiErrorCode::Unavailable
    } else if lower.contains("decoder reset") || lower.contains("conversion") {
        ApiErrorCode::Internal
    } else {
        ApiErrorCode::BadRequest
    };
    ApiError::new(code, format!("audio analysis failed: {message}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_format_parses_known_values_and_rejects_unknown_values() {
        assert!(matches!("MP3".parse(), Ok(PodcastAudioFormat::Mp3)));
        assert!(matches!("wave".parse(), Ok(PodcastAudioFormat::Wav)));
        let error = "aac".parse::<PodcastAudioFormat>().unwrap_err();
        assert_eq!(error.code, ApiErrorCode::BadRequest);
    }
}

struct DecodeState {
    hop_sec: f64,
    low_energy: Vec<f64>,
    hit_energy: Vec<f64>,
    highpass: Option<Biquad>,
    lowpass: Option<Biquad>,
    source_sample_rate: Option<f64>,
    effective_sample_rate: Option<f64>,
    sample_step: usize,
    hop_size: usize,
    frame_sum: f64,
    frame_peak: f64,
    frame_count: usize,
    effective_samples: usize,
}

impl DecodeState {
    fn new(hop_sec: f64) -> Self {
        Self {
            hop_sec,
            low_energy: Vec::new(),
            hit_energy: Vec::new(),
            highpass: None,
            lowpass: None,
            source_sample_rate: None,
            effective_sample_rate: None,
            sample_step: 1,
            hop_size: 0,
            frame_sum: 0.0,
            frame_peak: 0.0,
            frame_count: 0,
            effective_samples: 0,
        }
    }

    fn init_filters(&mut self, sample_rate: f64) {
        if self.effective_sample_rate.is_some() {
            return;
        }
        self.sample_step = if sample_rate >= 44100.0 {
            4
        } else if sample_rate >= 32000.0 {
            3
        } else {
            2
        };
        let effective = sample_rate / self.sample_step as f64;
        self.source_sample_rate = Some(sample_rate);
        self.effective_sample_rate = Some(effective);
        self.hop_size = (effective * self.hop_sec).floor().max(80.0) as usize;
        self.highpass = Some(make_biquad("highpass", 32.0, 0.72, effective));
        self.lowpass = Some(make_biquad("lowpass", 178.0, 0.82, effective));
    }

    fn process_interleaved(
        &mut self,
        samples: &[f32],
        channels: usize,
        sample_rate: f64,
        limit_sec: Option<f64>,
    ) {
        self.init_filters(sample_rate);
        let effective = self.effective_sample_rate.unwrap_or(sample_rate);
        for frame in samples.chunks(channels) {
            let index = self.effective_samples % self.sample_step;
            let mono = if channels >= 2 {
                (frame[0] as f64 + frame[1] as f64) * 0.5
            } else {
                frame[0] as f64
            };
            if index == 0 {
                let high = run_biquad(self.highpass.as_mut().unwrap(), mono);
                let filtered = run_biquad(self.lowpass.as_mut().unwrap(), high);
                let absolute = filtered.abs();
                self.frame_sum += filtered * filtered;
                if absolute > self.frame_peak {
                    self.frame_peak = absolute;
                }
                self.frame_count += 1;
                if self.frame_count >= self.hop_size {
                    self.push_frame();
                }
            }
            self.effective_samples += 1;
            if let Some(limit_sec) = limit_sec {
                if self.duration() >= limit_sec {
                    break;
                }
            }
        }
        let _ = effective;
    }

    fn push_frame(&mut self) {
        let count = self.frame_count.max(1) as f64;
        self.low_energy.push((self.frame_sum / count).sqrt());
        self.hit_energy.push(self.frame_peak);
        self.frame_sum = 0.0;
        self.frame_peak = 0.0;
        self.frame_count = 0;
    }

    fn finish_frame(&mut self) {
        if self.frame_count > 0 {
            self.push_frame();
        }
    }

    fn duration(&self) -> f64 {
        let effective_sr = self.effective_sample_rate.unwrap_or_default();
        if effective_sr > 0.0 {
            (self.effective_samples / self.sample_step.max(1)) as f64 / effective_sr
        } else {
            0.0
        }
    }

    fn limit_reached(&self, limit_sec: Option<f64>) -> bool {
        limit_sec.is_some_and(|limit| self.duration() >= limit)
    }
}

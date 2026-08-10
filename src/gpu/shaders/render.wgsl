// lumino-gpu-synth render kernel (pass 1).
//
// One invocation renders one voice for the whole block (BLOCK frames),
// producing `BLOCK * 2` interleaved stereo samples into `voice_out`.
//
// The voice state (playback position, envelope stage, filter state) is
// read from_val `states` at the start of the block and written back at the end,
// so voices can span arbitrary many blocks.
//
// Signal chain (mirrors XSynth's stereo voice):
//   sample(L/R) * amp(velocity volume) * pan_gain(L/R) * envelope
//   -> per-channel biquad low-pass (if enabled)
// The channel volume/expression/pan controllers are applied in the mix pass.

struct VoiceParams {
    is_active: u32,          // 0 = slot unused
    sample_offset: u32,   // offset of the sample data inside `samples`
    sample_offset_r: u32,  // offset of the right-channel sample data
    sample_len: u32,      // length of the (first channel) sample data
    offset: u32,          // playback start offset (converted domain)
    sample_end: u32,      // voice ends when time >= sample_end (conv(sample_end) - conv(offset))
    loop_mode: u32,       // 0 = no loop, 1 = continuous, 2 = sustain
    loop_start: u32,      // data-relative loop start (converted domain)
    loop_end: u32,        // data-relative loop end (converted domain)
    speed: f32,           // samples advanced per output frame
    amp: f32,             // static amplitude (volume * velocity curve)
    pan_l: f32,           // left gain from_val the zone pan
    pan_r: f32,           // right gain from_val the zone pan
    filter_on: u32,
    b0: f32, b1: f32, b2: f32, a1: f32, a2: f32,
    env_base: u32,        // index of this voice's first stage in `env_stages`
    env_count: u32,       // number of stages
    release_idx: u32,     // stage index (relative to env_base) to jump to on release
    finished_idx: u32,    // index of the terminal stage
    release_at: u32,      // absolute global frame at which release starts (0xFFFFFFFF = none)
    base_frame: u32,      // absolute global frame of this block's first sample
    interp: u32,          // 0 = linear, 1 = 64-point sinc
    channels: u32,        // 1 = mono, 2 = stereo pair
    start_at: u32,        // absolute global frame at which the voice starts (gated before)
    channel: u32,         // MIDI channel (0-15), used by the mix pass
}

struct VoiceState {
    int_time: u32,        // integer part of playback position (f64-equivalent)
    frac: f32,            // fractional part of playback position
    env_stage: u32,       // current stage index (relative to env_base)
    env_t: u32,           // samples elapsed in the current stage
    env_from: f32,        // value at the start of the current stage
    lx1: f32, lx2: f32, ly1: f32, ly2: f32,  // biquad state (left channel)
    rx1: f32, rx2: f32, ry1: f32, ry2: f32,  // biquad state (right channel)
    last_loop_pos: u32,   // loop position at release (loop sustain mode)
    is_released: u32,     // sampler-side release flag
    ended: u32,           // 1 when the voice has finished (sample end or env done)
}

struct EnvStageGpu {
    kind: u32,            // 0 lerp, 1 concave, 2 convex, 3 hold
    target_val: f32,
    duration: u32,
}

@group(0) @binding(0) var<storage, read> params: array<VoiceParams>;
@group(0) @binding(1) var<storage, read> samples: array<f32>;
@group(0) @binding(2) var<storage, read> sinc_table: array<f32>;
@group(0) @binding(3) var<storage, read> env_stages: array<EnvStageGpu>;
@group(0) @binding(4) var<storage, read_write> states: array<VoiceState>;
@group(0) @binding(5) var<storage, read_write> voice_out: array<f32>;

const VOICES_PER_GROUP: u32 = 128u;
const SINC_PHASES: u32 = 4096u;
const SINC_TAPS: u32 = 64u;
const BLOCK: u32 = 512u;

// ---------- helpers ----------

fn env_eval(kind: u32, from_val: f32, target_val: f32, f: f32) -> f32 {
    if kind == 0u {
        return from_val + (target_val - from_val) * f;
    }
    if kind == 1u {
        let m = (1.0 - f) * (1.0 - f);
        let m2 = m * m;
        let mult = m2 * m2;
        return (from_val - target_val) * mult + target_val;
    }
    if kind == 2u {
        let m = f * f;
        let m2 = m * m;
        let mult = m2 * m2;
        return from_val + (target_val - from_val) * mult;
    }
    return target_val;
}

// Reads one f32 from_val the samples buffer; out-of-bounds reads yield 0.0.
fn raw_sample(offset: u32, idx: u32) -> f32 {
    if offset + idx >= arrayLength(&samples) {
        return 0.0;
    }
    return samples[offset + idx];
}

// Computes the looped position for a data-relative absolute index.
fn loop_pos(p: VoiceParams, pos_abs: u32, released: u32, last_loop: u32) -> u32 {
    if p.loop_mode == 1u {
        var pos = pos_abs;
        if pos > p.loop_end {
            pos = (pos - p.loop_end - 1u) % (p.loop_end - p.loop_start) + p.loop_start;
        }
        return pos;
    }
    if p.loop_mode == 2u {
        if released == 0u {
            var pos = pos_abs;
            if pos > p.loop_end {
                pos = (pos - p.loop_end - 1u) % (p.loop_end - p.loop_start) + p.loop_start;
            }
            return pos;
        }
        // Released: continue from_val loop_end with the same elapsed time.
        let elapsed = pos_abs - last_loop;
        return p.loop_end + elapsed;
    }
    return pos_abs;
}

fn linear_interp(p: VoiceParams, pos_abs: u32, frac: f32, released: u32, last_loop: u32) -> f32 {
    let a = loop_pos(p, pos_abs, released, last_loop);
    let v0 = raw_sample(p.sample_offset, a);
    let v1 = raw_sample(p.sample_offset, a + 1u);
    return v0 * (1.0 - frac) + v1 * frac;
}

fn sinc_interp(p: VoiceParams, pos_abs: u32, frac: f32, released: u32, last_loop: u32) -> f32 {
    let phase = u32(frac * f32(SINC_PHASES)) & (SINC_PHASES - 1u);
    var acc = 0.0;
    for (var k = 0u; k < SINC_TAPS; k = k + 1u) {
        let c = sinc_table[phase * SINC_TAPS + k];
        let base = i32(pos_abs) + i32(k) - 31i;
        if base >= 0 {
            let a = loop_pos(p, u32(base), released, last_loop);
            acc += c * raw_sample(p.sample_offset, a);
        }
    }
    return acc;
}

// ---------- main ----------

@compute
@workgroup_size(VOICES_PER_GROUP)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let voice = gid.x;
    if (voice >= arrayLength(&params)) {
        return;
    }
    let p = params[voice];
    if (p.is_active == 0u) {
        return;
    }

    var st = states[voice];
    let out_base = voice * BLOCK * 2u;

    // Track the last loop position for loop-sustain release handling.
    if (p.loop_mode == 2u && st.is_released == 0u) {
        st.last_loop_pos = st.int_time;
    }

    var past_end = st.ended == 1u;
    var env_value = st.env_from;

    for (var f = 0u; f < BLOCK; f = f + 1u) {
        let frame = p.base_frame + f;

        // --- start gating: the voice is silent before its note-on frame ---
        if (frame < p.start_at) {
            let idx = out_base + f * 2u;
            voice_out[idx] = 0.0;
            voice_out[idx + 1u] = 0.0;
            continue;
        }

        // --- release scheduling (sample-accurate) ---
        if (p.release_at != 0xFFFFFFFFu && frame >= p.release_at && st.is_released == 0u) {
            st.is_released = 1u;
            st.env_stage = p.release_idx;
            st.env_t = 0u;
            st.env_from = env_value;
        }

        // --- envelope ---
        let stage_idx = st.env_stage;
        if (stage_idx >= p.finished_idx) {
            // Terminal stage reached (or passed): the envelope is done and
            // the voice ends, mirroring XSynth's `envelope.ended()`.
            st.ended = 1u;
            past_end = true;
        } else if (stage_idx < p.env_count) {
            let es = env_stages[p.env_base + stage_idx];
            if (es.kind == 3u) {
                env_value = es.target_val; // hold
            } else {
                let prog = f32(st.env_t) / f32(es.duration);
                env_value = env_eval(es.kind, st.env_from, es.target_val, prog);
                st.env_t = st.env_t + 1u;
                if (st.env_t >= es.duration) {
                    st.env_from = env_value;
                    st.env_stage = stage_idx + 1u;
                    st.env_t = 0u;
                }
            }
        }

        // --- sample position & interpolation ---
        let pos_abs = st.int_time + p.offset;
        var sample_l = 0.0;
        var sample_r = 0.0;

        if (!past_end) {
            if (p.interp == 1u) {
                sample_l = sinc_interp(p, pos_abs, st.frac, st.is_released, st.last_loop_pos);
            } else {
                sample_l = linear_interp(p, pos_abs, st.frac, st.is_released, st.last_loop_pos);
            }
            if (p.channels == 2u) {
                var p_r = p;
                p_r.sample_offset = p.sample_offset_r;
                if (p.interp == 1u) {
                    sample_r = sinc_interp(p_r, pos_abs, st.frac, st.is_released, st.last_loop_pos);
                } else {
                    sample_r = linear_interp(p_r, pos_abs, st.frac, st.is_released, st.last_loop_pos);
                }
            } else {
                // Mono sample duplicated to both channels (XSynth behaviour).
                sample_r = sample_l;
            }
        }

        // Sample ended check (no-loop only): time >= sample_end (already
        // reduced by offset on the CPU side).
        if (p.loop_mode == 0u && st.int_time >= p.sample_end) {
            past_end = true;
            st.ended = 1u;
        }

        // --- gain chain: sample * amp * pan * env (mirrors XSynth) ---
        var value_l = sample_l * p.amp * p.pan_l * env_value;
        var value_r = sample_r * p.amp * p.pan_r * env_value;

        // --- per-channel biquad low-pass (XSynth stereo voice structure) ---
        if (p.filter_on != 0u) {
            let yl = p.b0 * value_l + p.b1 * st.lx1 + p.b2 * st.lx2
                   - p.a1 * st.ly1 - p.a2 * st.ly2;
            st.lx2 = st.lx1; st.lx1 = value_l;
            st.ly2 = st.ly1; st.ly1 = yl;
            value_l = yl;

            let yr = p.b0 * value_r + p.b1 * st.rx1 + p.b2 * st.rx2
                   - p.a1 * st.ry1 - p.a2 * st.ry2;
            st.rx2 = st.rx1; st.rx1 = value_r;
            st.ry2 = st.ry1; st.ry1 = yr;
            value_r = yr;
        }

        let out_idx = out_base + f * 2u;
        voice_out[out_idx] = value_l;
        voice_out[out_idx + 1u] = value_r;

        // --- advance position ---
        var carry: u32 = 0u;
        var frac = st.frac + p.speed;
        if (frac >= 1.0) {
            let n = u32(frac);
            frac = frac - f32(n);
            carry = n;
        }
        st.int_time = st.int_time + carry;
        st.frac = frac;
    }

    states[voice] = st;
}

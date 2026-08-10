// lumino-gpu-synth mix kernel (pass 2).
//
// Sums all voices produced by the render pass into the stereo output,
// grouped per MIDI channel, and applies each channel's volume/expression/pan
// controllers with the same 10 ms linear smoothing (`ValueLerp`) semantics
// as XSynth:
//
//   vol  = min(vol_start + vol_delta * f, vol_end)   (f = frame in block)
//   expr = min(expr_start + expr_delta * f, expr_end)
//   amp  = (vol * expr)^2
//   pan  = min(pan_start + pan_delta * f, pan_end)
//   outL = sum * amp * cos(pan * PI/2)
//   outR = sum * amp * sin(pan * PI/2)

struct MixParams {
    voice_count: u32,
    block_size: u32,
    channel_count: u32,
}

struct ChannelMix {
    vol_start: f32,
    vol_delta: f32,
    vol_end: f32,
    expr_start: f32,
    expr_delta: f32,
    expr_end: f32,
    pan_start: f32,
    pan_delta: f32,
    pan_end: f32,
}

@group(0) @binding(0) var<storage, read> voice_out: array<f32>;
@group(0) @binding(1) var<storage, read_write> output: array<f32>;
@group(0) @binding(2) var<storage, read> voice_chans: array<u32>;
@group(0) @binding(3) var<storage, read> channel_mix: array<ChannelMix>;
@group(0) @binding(4) var<uniform> mix_params: MixParams;

const MAX_CHANNELS: u32 = 16u;

@compute
@workgroup_size(128)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let f = gid.x;
    if (f >= mix_params.block_size) {
        return;
    }

    // Accumulate voices grouped by channel.
    var acc_l: array<f32, MAX_CHANNELS>;
    var acc_r: array<f32, MAX_CHANNELS>;
    for (var c = 0u; c < MAX_CHANNELS; c = c + 1u) {
        acc_l[c] = 0.0;
        acc_r[c] = 0.0;
    }

    for (var v = 0u; v < mix_params.voice_count; v = v + 1u) {
        let base = (v * mix_params.block_size + f) * 2u;
        let ch = voice_chans[v] & (MAX_CHANNELS - 1u);
        acc_l[ch] = acc_l[ch] + voice_out[base];
        acc_r[ch] = acc_r[ch] + voice_out[base + 1u];
    }

    let ff = f32(f);
    let half_pi = 1.5707963267948966;

    var out_l = 0.0;
    var out_r = 0.0;

    for (var ch = 0u; ch < mix_params.channel_count; ch = ch + 1u) {
        if (ch >= arrayLength(&channel_mix)) {
            break;
        }
        let cm = channel_mix[ch];

        let vol = min(cm.vol_start + cm.vol_delta * ff, cm.vol_end);
        let expr = min(cm.expr_start + cm.expr_delta * ff, cm.expr_end);
        let amp = (vol * expr) * (vol * expr);

        let pan = min(cm.pan_start + cm.pan_delta * ff, cm.pan_end);
        let pan_angle = pan * half_pi;
        let pan_l = cos(pan_angle);
        let pan_r = sin(pan_angle);

        out_l = out_l + acc_l[ch] * amp * pan_l;
        out_r = out_r + acc_r[ch] * amp * pan_r;
    }

    output[f * 2u] = out_l;
    output[f * 2u + 1u] = out_r;
}

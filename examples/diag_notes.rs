//! Diagnostic: note-length and concurrency statistics of the MIDI file.
//! If note lengths are pathological (e.g. ~0 ticks) or concurrency explodes,
//! the renderer's voice limit will truncate notes -> "muddy, wrong length".

use std::collections::HashMap;

use lumino_gpu_synth::MidiFile;
use lumino_gpu_synth::midi::MidiEvent::*;

fn main() -> Result<(), lumino_gpu_synth::SynthError> {
    let midi = MidiFile::load("assets/right-example.mid", 64_000)?;

    // Pair note-ons/offs per (channel,key) with a FIFO queue so overlapping
    // notes of the same key do not corrupt the length histogram.
    let sr = 64_000u64;
    let mut vel_buckets = [0u64; 2];
    let mut pending: HashMap<(u8, u8), Vec<u64>> = HashMap::new();
    let mut on_samples: Vec<u64> = Vec::new();
    let mut concurrency = 0u64;
    let mut max_concurrency = 0u64;
    let mut lengths: Vec<u64> = Vec::new();
    let mut misses = 0u64;
    let mut on_secs: HashMap<u64, u64> = HashMap::new();

    for ev in &midi.sequence.events {
        match ev.event {
            NoteOn { key, vel } => {
                let b = if vel <= 1 { 0 } else { 1 };
                vel_buckets[b] += 1;
                pending
                    .entry((ev.channel, key))
                    .or_default()
                    .push(ev.sample);
                on_samples.push(ev.sample);
                *on_secs.entry(ev.sample / sr).or_insert(0) += 1;
                concurrency += 1;
                if concurrency > max_concurrency {
                    max_concurrency = concurrency;
                }
            }
            NoteOff { key } => {
                let q = pending.get_mut(&(ev.channel, key));
                match q.and_then(|q| {
                    if q.is_empty() {
                        None
                    } else {
                        Some(q.remove(0))
                    }
                }) {
                    Some(start) => lengths.push(ev.sample - start),
                    None => misses += 1,
                }
                concurrency = concurrency.saturating_sub(1);
            }
            _ => {}
        }
    }
    println!(
        "note_ons: vel<=1: {} (dropped), vel>1: {}",
        vel_buckets[0], vel_buckets[1]
    );
    println!(
        "notes: {}, unmatched note_offs: {misses}, leftover pending: {}",
        lengths.len(),
        pending.values().map(|v| v.len()).sum::<usize>()
    );
    println!("max concurrent voices: {max_concurrency}");

    lengths.sort_unstable();
    let n = lengths.len();
    if n > 0 {
        let q = |p: usize| lengths[(p * n / 100).min(n - 1)];
        println!(
            "note length (samples @64k): min={} p10={} p50={} p90={} max={}",
            lengths[0],
            q(10),
            q(50),
            q(90),
            lengths[n - 1]
        );
        let mut ms = [0u64; 8];
        for &l in &lengths {
            let m = l * 1000 / sr;
            let b = match m {
                0..=4 => 0,
                5..=19 => 1,
                20..=49 => 2,
                50..=99 => 3,
                100..=199 => 4,
                200..=499 => 5,
                500..=999 => 6,
                _ => 7,
            };
            ms[b] += 1;
        }
        println!(
            "length buckets (<=4ms, 5-19, 20-49, 50-99, 100-199, 200-499, 500-999, >=1s): {ms:?}"
        );
    }

    // Concurrency and note density per 10 s window for the whole file
    // (only vel>1 notes, i.e. the ones the renderer will actually play).
    let mut wstart = 0u64;
    let mut win_on = 0u64;
    let mut win_peak = 0u64;
    let mut cur = 0u64;
    let mut peak_at = 0u64;
    let mut density: Vec<(u64, u64, u64, u64)> = Vec::new(); // (sec, ons, peak, peak_at)
    for ev in &midi.sequence.events {
        match ev.event {
            NoteOn { vel, .. } if vel > 1 => {
                cur += 1;
                win_on += 1;
            }
            NoteOff { .. } => {
                cur = cur.saturating_sub(1);
            }
            _ => {}
        }
        if cur > win_peak {
            win_peak = cur;
            peak_at = ev.sample;
        }
        if ev.sample - wstart >= 10 * sr {
            density.push((wstart / sr, win_on, win_peak, peak_at / sr));
            wstart = ev.sample;
            win_on = 0;
            win_peak = 0;
        }
    }
    if win_on > 0 {
        density.push((wstart / sr, win_on, win_peak, peak_at / sr));
    }
    for &(sec, ons, peak, pa) in density.iter().take(25) {
        println!("{sec:>5}s: note_ons={ons:>7} peak_concurrent={peak:>6} (at {pa}s)");
    }
    Ok(())
}

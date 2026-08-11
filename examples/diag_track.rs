//! Extracts track N of a MIDI into a single-track file.
//! Usage: diag_track <in.mid> <track_idx> <out.mid>

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let raw = std::fs::read(&args[1])?;
    let smf = lumino_midly::Smf::parse(&raw)?;
    let idx: usize = args[2].parse()?;
    let track = smf.tracks.get(idx).cloned().unwrap_or_default();
    let out = lumino_midly::Smf {
        header: smf.header.clone(),
        tracks: vec![track],
    };
    out.save(&args[3])?;
    println!("wrote {} ({} events)", args[3], out.tracks[0].len());
    Ok(())
}

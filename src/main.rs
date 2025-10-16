use chrono::{DateTime, Utc};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rand::Rng;
use rusqlite::{Connection, OpenFlags, Result as SqlResult};
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use symphonia::core::audio::{SampleBuffer, SignalSpec};
use symphonia::core::codecs::{CODEC_TYPE_NULL, DecoderOptions};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

struct VoiceMemo {
    title: String,
    date: f64,
    duration: f64,
    path: String,
}

fn get_voice_memos_db_path() -> PathBuf {
    let home = std::env::var("HOME").expect("HOME environment variable not set");
    PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join("com.apple.voicememos")
        .join("CloudRecordings.db")
}

fn get_voice_memos_dir() -> PathBuf {
    let home = std::env::var("HOME").expect("HOME environment variable not set");
    PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join("com.apple.voicememos")
        .join("Recordings")
}

fn core_data_to_unix_timestamp(core_data_timestamp: f64) -> i64 {
    // Core Data reference date is January 1, 2001 00:00:00 UTC
    // Unix epoch is January 1, 1970 00:00:00 UTC
    // Difference is 978307200 seconds
    const CORE_DATA_EPOCH_OFFSET: f64 = 978307200.0;
    (core_data_timestamp + CORE_DATA_EPOCH_OFFSET) as i64
}

fn get_all_voice_memos() -> SqlResult<Vec<VoiceMemo>> {
    let db_path = get_voice_memos_db_path();

    // Open database in READ-ONLY mode to prevent any modifications
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;

    let mut stmt = conn.prepare(
        "SELECT ZTITLE, ZDATE, ZDURATION, ZPATH FROM ZCLOUDRECORDING WHERE ZDURATION > 30.0",
    )?;

    let memos = stmt
        .query_map([], |row| {
            Ok(VoiceMemo {
                title: row.get(0).unwrap_or_else(|_| "Untitled".to_string()),
                date: row.get(1)?,
                duration: row.get(2)?,
                path: row.get(3)?,
            })
        })?
        .filter_map(|r| r.ok())
        .collect();

    Ok(memos)
}

fn play_audio_segment(
    file_path: &PathBuf,
    start_sec: f64,
    duration_sec: f64,
) -> Result<(), Box<dyn std::error::Error>> {
    // Open the media source in READ-ONLY mode (File::open is read-only by default)
    let file = File::open(file_path)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    // Create a hint to help the format registry guess the format
    let mut hint = Hint::new();
    hint.with_extension("m4a");

    // Probe the media source
    let probed = symphonia::default::get_probe().format(
        &hint,
        mss,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    )?;

    let mut format = probed.format;

    // Find the first audio track
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or("No audio track found")?;

    let track_id = track.id;
    let sample_rate = track.codec_params.sample_rate.ok_or("No sample rate")?;
    let channels = track.codec_params.channels.ok_or("No channels")?;

    // Create a decoder
    let mut decoder =
        symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default())?;

    // Calculate frame positions
    let start_frame = (start_sec * sample_rate as f64) as u64;
    let end_frame = start_frame + (duration_sec * sample_rate as f64) as u64;

    // Set up audio output
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or("No output device available")?;

    let config = cpal::StreamConfig {
        channels: channels.count() as u16,
        sample_rate: cpal::SampleRate(sample_rate),
        buffer_size: cpal::BufferSize::Default,
    };

    // Shared buffer for audio samples
    let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<f32>>(32);

    // Create output stream
    let stream = device.build_output_stream(
        &config,
        move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
            if let Ok(samples) = rx.try_recv() {
                let len = data.len().min(samples.len());
                data[..len].copy_from_slice(&samples[..len]);
                if len < data.len() {
                    data[len..].fill(0.0);
                }
            } else {
                data.fill(0.0);
            }
        },
        |err| eprintln!("Audio stream error: {}", err),
        None,
    )?;

    stream.play()?;

    // Decode and play
    let mut current_frame = 0u64;
    let mut sample_buf = None;

    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(_) => break,
        };

        if packet.track_id() != track_id {
            continue;
        }

        match decoder.decode(&packet) {
            Ok(decoded) => {
                if sample_buf.is_none() {
                    let spec = SignalSpec::new(sample_rate, channels);
                    sample_buf = Some(SampleBuffer::<f32>::new(decoded.capacity() as u64, spec));
                }

                if let Some(ref mut buf) = sample_buf {
                    buf.copy_interleaved_ref(decoded);
                    let samples = buf.samples();

                    let packet_frames = samples.len() as u64 / channels.count() as u64;
                    let packet_end = current_frame + packet_frames;

                    // Check if this packet contains audio we want to play
                    if packet_end >= start_frame && current_frame < end_frame {
                        let start_sample = if current_frame < start_frame {
                            ((start_frame - current_frame) * channels.count() as u64) as usize
                        } else {
                            0
                        };

                        let end_sample = if packet_end > end_frame {
                            ((end_frame - current_frame) * channels.count() as u64) as usize
                        } else {
                            samples.len()
                        };

                        if start_sample < end_sample {
                            let segment = samples[start_sample..end_sample].to_vec();
                            if tx.send(segment).is_err() {
                                break;
                            }

                            // Small delay to prevent buffer underrun
                            std::thread::sleep(std::time::Duration::from_millis(10));
                        }
                    }

                    current_frame = packet_end;

                    if current_frame >= end_frame {
                        break;
                    }
                }
            }
            Err(_) => continue,
        }
    }

    // Wait for playback to complete
    std::thread::sleep(std::time::Duration::from_secs(1));

    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // NOTE: This script operates in READ-ONLY mode
    // - Database is opened with SQLITE_OPEN_READ_ONLY flag
    // - Audio files are opened with File::open (read-only by default)
    // - No files are created, modified, or deleted

    println!("Loading Voice Memos library...\n");

    let memos = get_all_voice_memos()?;

    if memos.is_empty() {
        eprintln!("No voice memos found (longer than 30 seconds).");
        return Ok(());
    }

    println!(
        "Found {} voice memos longer than 30 seconds.\n",
        memos.len()
    );

    // Select a random memo
    let mut rng = rand::thread_rng();
    let memo = &memos[rng.gen_range(0..memos.len())];

    // Select a random start time (ensuring 30 seconds fits)
    let max_start = memo.duration - 30.0;
    let start_time = rng.gen_range(0.0..max_start);

    // Convert Core Data timestamp to human-readable date
    let unix_timestamp = core_data_to_unix_timestamp(memo.date);
    let datetime = DateTime::<Utc>::from_timestamp(unix_timestamp, 0).unwrap_or_else(|| Utc::now());

    // Display information
    println!("═══════════════════════════════════════════════════");
    println!("  Random Voice Memo Clip");
    println!("═══════════════════════════════════════════════════");
    println!("Title:    {}", memo.title);
    println!(
        "Date:     {}",
        datetime.format("%B %d, %Y at %I:%M:%S %p UTC")
    );
    println!("Duration: {:.1} seconds", memo.duration);
    println!(
        "Clip:     {:.1}s - {:.1}s (30 seconds)",
        start_time,
        start_time + 30.0
    );
    println!("═══════════════════════════════════════════════════\n");

    // Construct full path
    let recordings_dir = get_voice_memos_dir();
    let full_path = recordings_dir.join(&memo.path);

    if !full_path.exists() {
        eprintln!("Error: Recording file not found at {:?}", full_path);
        eprintln!("The recording may be in iCloud and not downloaded locally.");
        return Ok(());
    }

    println!("Playing clip...\n");
    play_audio_segment(&full_path, start_time, 30.0)?;

    println!("Playback complete!");

    Ok(())
}

use chrono::{DateTime, Utc};
use rand::Rng;
use rusqlite::{Connection, OpenFlags, Result as SqlResult};
use std::path::PathBuf;
use std::process::Command;

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
        .join("Group Containers")
        .join("group.com.apple.VoiceMemos.shared")
        .join("Recordings")
        .join("CloudRecordings.db")
}

fn get_voice_memos_dir() -> PathBuf {
    let home = std::env::var("HOME").expect("HOME environment variable not set");
    PathBuf::from(home)
        .join("Library")
        .join("Group Containers")
        .join("group.com.apple.VoiceMemos.shared")
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
        "SELECT ZENCRYPTEDTITLE, ZCUSTOMLABEL, ZDATE, ZDURATION, ZPATH FROM ZCLOUDRECORDING WHERE ZDURATION > 30.0"
    )?;

    const ZENCRYPTEDTITLE_COL_NUMBER: usize = 0;
    const ZCUSTOMLABEL_COL_NUMBER: usize = 1;
    const ZDATE_COL_NUM: usize = 2;
    const ZDURATION_COL_NUM: usize = 3;
    const ZPATH_COL_NUM: usize = 4;

    let memos = stmt
        .query_map([], |row| {
            // Try ZENCRYPTEDTITLE first, fall back to ZCUSTOMLABEL, then "Untitled"
            let title = row
                .get(ZENCRYPTEDTITLE_COL_NUMBER)
                .or_else(|_| row.get(ZCUSTOMLABEL_COL_NUMBER))
                .unwrap_or_else(|_| "Untitled".to_string());

            Ok(VoiceMemo {
                title,
                date: row.get(ZDATE_COL_NUM)?,
                duration: row.get(ZDURATION_COL_NUM)?,
                path: row.get(ZPATH_COL_NUM)?,
            })
        })?
        .filter_map(|r| r.ok())
        .collect();

    Ok(memos)
}

fn extract_and_play_clip(
    source_path: &PathBuf,
    start_sec: f64,
    duration_sec: f64,
    original_date: DateTime<Utc>,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    // Create a temporary file for the clip
    let temp_dir = std::env::temp_dir();
    let clip_path = temp_dir.join(format!("voice_memo_clip_{}.m4a", std::process::id()));

    println!("Extracting 30-second clip with ffmpeg...");
    println!("Original clip path: {}", source_path.to_str().unwrap());

    // Format the date for the comment field
    let comment = format!(
        "Original recording date: {}",
        original_date.format("%B %d, %Y at %I:%M:%S %p UTC")
    );

    // Use ffmpeg to extract the clip and add metadata
    let output = Command::new("ffmpeg")
        .arg("-ss")
        .arg(format!("{}", start_sec))
        .arg("-i")
        .arg(source_path)
        .arg("-t")
        .arg(format!("{}", duration_sec))
        .arg("-c")
        .arg("copy")
        .arg("-metadata")
        .arg(format!("comment={}", comment))
        .arg("-y") // Overwrite without asking
        .arg(&clip_path)
        .output()?;

    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr);
        return Err(format!("ffmpeg failed: {}", error).into());
    }

    println!("Clip saved to: {:?}\n", clip_path);
    println!("Opening with VLC...\n");

    // Open with VLC
    Command::new("open")
        .arg("-g")
        .arg("-a")
        .arg("VLC")
        .arg(&clip_path)
        .spawn()?;

    Ok(clip_path)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // NOTE: This script operates in READ-ONLY mode on Voice Memos
    // - Database is opened with SQLITE_OPEN_READ_ONLY flag
    // - Audio files are only read, never modified
    // - A temporary clip file is created for playback

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

    let clip_path = extract_and_play_clip(&full_path, start_time, 30.0, datetime)?;

    println!("VLC should now be playing the clip.");
    println!("Temporary file will remain at: {:?}", clip_path);
    println!("You can delete it manually or it will be cleaned up on reboot.");

    Ok(())
}

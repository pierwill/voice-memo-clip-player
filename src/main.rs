use chrono::{DateTime, Utc};
use rand::Rng;
use rusqlite::{Connection, OpenFlags, Result as SqlResult};
use std::fs;
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

    let memos = stmt
        .query_map([], |row| {
            // Try ZENCRYPTEDTITLE first, fall back to ZCUSTOMLABEL, then "Untitled"
            let title = row
                .get::<_, String>(0)
                .or_else(|_| row.get::<_, String>(1))
                .unwrap_or_else(|_| "Untitled".to_string());

            Ok(VoiceMemo {
                title,
                date: row.get(2)?,
                duration: row.get(3)?,
                path: row.get(4)?,
            })
        })?
        .filter_map(|r| r.ok())
        .collect();

    Ok(memos)
}

fn download_public_domain_images(
    count: usize,
    temp_dir: &PathBuf,
) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    println!("Downloading {} public domain images from Picsum...", count);

    let mut image_paths = Vec::new();
    let mut rng = rand::thread_rng();

    for i in 0..count {
        // Use Lorem Picsum for reliable random images (public domain)
        // Add a random seed to get different images each time
        let random_seed = rng.gen_range(1..1000);
        let url = format!("https://picsum.photos/1920/1080?random={}", random_seed + i);

        let image_path = temp_dir.join(format!("image_{}.jpg", i));

        // Download the image using curl
        let output = Command::new("curl")
            .arg("-L") // Follow redirects
            .arg("-s") // Silent mode
            .arg("-o")
            .arg(&image_path)
            .arg(&url)
            .output()?;

        if !output.status.success() {
            return Err(format!("Failed to download image {}", i).into());
        }

        // Verify the image is valid by checking file size
        let metadata = fs::metadata(&image_path)?;
        if metadata.len() < 10000 {
            return Err(format!("Downloaded image {} is too small (likely invalid)", i).into());
        }

        image_paths.push(image_path);
        print!(".");
        std::io::Write::flush(&mut std::io::stdout()).ok();
    }

    println!(" Done!");

    Ok(image_paths)
}

fn create_video_with_images(
    audio_path: &PathBuf,
    image_paths: &[PathBuf],
    duration_sec: f64,
    original_date: DateTime<Utc>,
    temp_dir: &PathBuf,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    println!("Creating video slideshow...");

    let video_path = temp_dir.join(format!("voice_memo_video_{}.mp4", std::process::id()));

    // Calculate duration per image
    let duration_per_image = duration_sec / image_paths.len() as f64;

    // Create a concat file for ffmpeg
    let concat_file = temp_dir.join(format!("concat_{}.txt", std::process::id()));
    let mut concat_content = String::new();

    for image_path in image_paths {
        concat_content.push_str(&format!("file '{}'\n", image_path.display()));
        concat_content.push_str(&format!("duration {}\n", duration_per_image));
    }
    // Add the last image again without duration for ffmpeg concat
    if let Some(last_image) = image_paths.last() {
        concat_content.push_str(&format!("file '{}'\n", last_image.display()));
    }

    fs::write(&concat_file, concat_content)?;

    // Format the date for the comment field
    let comment = format!(
        "Original recording date: {}",
        original_date.format("%B %d, %Y at %I:%M:%S %p UTC")
    );

    // Create video from images and audio with more robust settings
    let output = Command::new("ffmpeg")
        .arg("-f")
        .arg("concat")
        .arg("-safe")
        .arg("0")
        .arg("-i")
        .arg(&concat_file)
        .arg("-i")
        .arg(audio_path)
        .arg("-c:v")
        .arg("libx264")
        .arg("-tune")
        .arg("stillimage")
        .arg("-vf")
        .arg("scale=1920:1080:force_original_aspect_ratio=decrease,pad=1920:1080:(ow-iw)/2:(oh-ih)/2")
        .arg("-c:a")
        .arg("aac")
        .arg("-b:a")
        .arg("192k")
        .arg("-pix_fmt")
        .arg("yuv420p")
        .arg("-r")
        .arg("25") // Frame rate
        .arg("-shortest")
        .arg("-metadata")
        .arg(format!("comment={}", comment))
        .arg("-y")
        .arg(&video_path)
        .output()?;

    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr);
        return Err(format!("ffmpeg video creation failed: {}", error).into());
    }

    // Clean up concat file
    fs::remove_file(concat_file).ok();

    println!("Video created: {:?}\n", video_path);

    Ok(video_path)
}

fn extract_and_create_video(
    source_path: &PathBuf,
    start_sec: f64,
    duration_sec: f64,
    original_date: DateTime<Utc>,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    // Create a temporary directory for all our work
    let temp_dir = std::env::temp_dir().join(format!("voice_memo_{}", std::process::id()));
    fs::create_dir_all(&temp_dir)?;

    // Extract audio clip first
    println!("Extracting 30-second audio clip...");
    let clip_path = temp_dir.join("audio_clip.m4a");

    let output = Command::new("ffmpeg")
        .arg("-ss")
        .arg(format!("{}", start_sec))
        .arg("-i")
        .arg(source_path)
        .arg("-t")
        .arg(format!("{}", duration_sec))
        .arg("-c")
        .arg("copy")
        .arg("-y")
        .arg(&clip_path)
        .output()?;

    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr);
        return Err(format!("ffmpeg audio extraction failed: {}", error).into());
    }

    println!("Audio extracted.\n");

    // Download 6 images (each shown for 5 seconds in a 30-second clip)
    let image_paths = download_public_domain_images(6, &temp_dir)?;

    // Create video with images and audio
    let video_path = create_video_with_images(
        &clip_path,
        &image_paths,
        duration_sec,
        original_date,
        &temp_dir,
    )?;

    // Clean up intermediate files
    for image_path in image_paths {
        fs::remove_file(image_path).ok();
    }
    fs::remove_file(clip_path).ok();

    println!("Opening with VLC...\n");

    // Open with VLC
    Command::new("open")
        .arg("-a")
        .arg("VLC")
        .arg(&video_path)
        .spawn()?;

    Ok(video_path)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // NOTE: This script operates in READ-ONLY mode on Voice Memos
    // - Database is opened with SQLITE_OPEN_READ_ONLY flag
    // - Audio files are only read, never modified
    // - Temporary files are created in /tmp for video creation

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

    let video_path = extract_and_create_video(&full_path, start_time, 30.0, datetime)?;

    println!("VLC should now be playing the video.");
    println!("Video file: {:?}", video_path);
    println!("You can delete it manually or it will be cleaned up on reboot.");

    Ok(())
}

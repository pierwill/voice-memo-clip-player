# Voice Memo Clip Player

A Rust application that randomly selects a 30-second clip from your Apple Voice Memos library and plays it in VLC.

## Features

- 🎲 **Random Selection** - Picks a random voice memo from your entire library
- ✂️ **Clipping** - Extracts a random 30-second segment from the recording
- 📅 **Metadata Preservation** - Embeds the original recording date in clip metadata
- 🎬 **Playback** - Opens the audio clip in VLC for immediate listening
- 🔒 **Read-Only** - Never modifies your original Voice Memos

## Requirements

### Software Dependencies

- **macOS Sonoma (14) or later** - Uses the new Voice Memos storage location
- **Rust** - Install from [rustup.rs](https://rustup.rs/)
- **ffmpeg** - Install via Homebrew: `brew install ffmpeg`
- **VLC.app** - Download from [videolan.org](https://www.videolan.org/)

### System Permissions

The app requires **Full Disk Access** to read Voice Memos:

1. Open **System Settings** → **Privacy & Security** → **Full Disk Access**
2. Click the lock icon and authenticate
3. Click **+** and add your terminal app (Terminal.app, iTerm2, etc.)
4. Restart your terminal completely

## Usage

### Basic Usage

Simply run the application:

```bash
cargo run --release
```

### What Happens

1. The app scans your Voice Memos library
2. Randomly selects a memo longer than 30 seconds
3. Picks a random 30-second segment from that memo
4. Extracts the clip with metadata
5. Opens the audio clip in VLC

### Example Output

```
Loading Voice Memos library...

Found 47 voice memos longer than 30 seconds.

═══════════════════════════════════════════════════
  Random Voice Memo Clip
═══════════════════════════════════════════════════
Title:    Meeting Notes
Date:     October 12, 2025 at 02:30:45 PM UTC
Duration: 245.3 seconds
Clip:     87.2s - 117.2s (30 seconds)
═══════════════════════════════════════════════════

Extracting 30-second clip with ffmpeg...
Clip saved to: "/var/folders/.../voice_memo_clip_12345.m4a"

Opening with VLC...

VLC should now be playing the clip.
Temporary file will remain at: "/var/folders/.../voice_memo_clip_12345.m4a"
```

## How It Works

### 1. Database Access

The app reads the Voice Memos SQLite database located at:
```
~/Library/Group Containers/group.com.apple.VoiceMemos.shared/Recordings/CloudRecordings.db
```

It opens the database in **read-only mode** to prevent any modifications.

### 2. Audio Extraction

Uses `ffmpeg` to extract the 30-second clip with metadata:
```bash
ffmpeg -ss <start_time> -i <source> -t 30 -c copy -metadata comment="Original recording date: ..." audio_clip.m4a
```

### 3. Playback

Opens the audio clip in VLC using macOS's `open` command.

## File Locations

### Input (Read-Only)
- Database: `~/Library/Group Containers/group.com.apple.VoiceMemos.shared/Recordings/CloudRecordings.db`
- Audio files: `~/Library/Group Containers/group.com.apple.VoiceMemos.shared/Recordings/*.m4a`

### Output (Temporary)
- Clip files: `/tmp/voice_memo_clip_<pid>.m4a`
- Files persist until manual deletion or system reboot

## Technical Details

### Database Schema

The app queries the `ZCLOUDRECORDING` table:
- `ZENCRYPTEDTITLE` - Recording title (encrypted)
- `ZCUSTOMLABEL` - Custom label (fallback)
- `ZDATE` - Core Data timestamp (seconds since Jan 1, 2001)
- `ZDURATION` - Duration in seconds
- `ZPATH` - Relative path to .m4a file

### Safety & Privacy

- Database opened with `SQLITE_OPEN_READ_ONLY` flag
- Audio files opened with `File::open` (read-only by default)
- No modifications to Voice Memos library
- All output files created in temporary directories
- **All data stays on your computer** - nothing is uploaded anywhere
- **Voice memos are never modified** - the app only reads them
- **Temporary clip files** remain local in `/tmp`
- **Read-only database access** - guaranteed by SQLite flags

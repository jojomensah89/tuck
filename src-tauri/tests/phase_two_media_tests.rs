use tuck_lib::{lyrics, telemetry};

use tuck_lib::media::{MediaSessionInfo, MediaState};

#[test]
fn test_media_state_phase_two_fields() {
    let state = MediaState {
        active: true,
        track_id: "spotify:artist:track:240000".to_string(),
        title: "Song Title".to_string(),
        artist: "Artist Name".to_string(),
        playing: true,
        position_ms: 60_000,
        duration_ms: 240_000,
        artwork: Some("data:image/jpeg;base64,Zm9v".to_string()),
        session_id: Some("Spotify.exe:1".to_string()),
        source_app_id: Some("Spotify.exe".to_string()),
        can_play_pause: true,
        can_skip_next: true,
        can_skip_prev: true,
        can_seek: true,
        generation: 5,
    };

    let serialized = serde_json::to_string(&state).expect("Failed to serialize MediaState");
    assert!(serialized.contains("\"sessionId\":\"Spotify.exe:1\""));
    assert!(serialized.contains("\"sourceAppId\":\"Spotify.exe\""));
    assert!(serialized.contains("\"canPlayPause\":true"));
    assert!(serialized.contains("\"canSkipNext\":true"));
    assert!(serialized.contains("\"canSkipPrev\":true"));
    assert!(serialized.contains("\"canSeek\":true"));
    assert!(serialized.contains("\"generation\":5"));

    let deserialized: MediaState =
        serde_json::from_str(&serialized).expect("Deserialization failed");
    assert_eq!(deserialized, state);
}

#[test]
fn test_media_session_info_serialization() {
    let session = MediaSessionInfo {
        id: "Spotify.exe:0".to_string(),
        source_app_id: "Spotify.exe".to_string(),
        title: "Test".to_string(),
        artist: "Singer".to_string(),
        playing: true,
        is_selected: true,
    };

    let serialized = serde_json::to_string(&session).expect("Failed to serialize MediaSessionInfo");
    assert!(serialized.contains("\"sourceAppId\":\"Spotify.exe\""));
    assert!(serialized.contains("\"isSelected\":true"));
}

#[test]
fn test_lyrics_lrc_multiline_and_metadata_filtering() {
    let lrc = r#"
[ti:Midnight City]
[ar:M83]
[al:Hurry Up, We're Dreaming]
[length:04:03]
[00:00.00]
[00:15.50]Waiting in a car
[00:20.10][00:25.80]Waiting for a ride in the dark
[00:30.00]The night city grows
"#;

    let lines = lyrics::parse_lrc(lrc);
    // [00:00.00] produces an empty text line
    // [00:15.50] produces 1 line
    // [00:20.10][00:25.80] produces 2 lines
    // [00:30.00] produces 1 line
    assert_eq!(lines.len(), 5);

    assert_eq!(lines[0].time_ms, 0);
    assert_eq!(lines[0].text, "");

    assert_eq!(lines[1].time_ms, 15_500);
    assert_eq!(lines[1].text, "Waiting in a car");

    assert_eq!(lines[2].time_ms, 20_100);
    assert_eq!(lines[2].text, "Waiting for a ride in the dark");

    assert_eq!(lines[3].time_ms, 25_800);
    assert_eq!(lines[3].text, "Waiting for a ride in the dark");

    assert_eq!(lines[4].time_ms, 30_000);
    assert_eq!(lines[4].text, "The night city grows");
}

#[test]
fn test_lyrics_timestamp_formats() {
    assert_eq!(lyrics::parse_lrc_timestamp("[01:23.45]"), Some(83_450));
    assert_eq!(lyrics::parse_lrc_timestamp("[00:05.1]"), Some(5_100));
    assert_eq!(lyrics::parse_lrc_timestamp("[02:10.999]"), Some(130_999));
    assert_eq!(lyrics::parse_lrc_timestamp("[00:12]"), Some(12_000));
    assert_eq!(lyrics::parse_lrc_timestamp("[bad:timestamp]"), None);
}

#[test]
fn test_telemetry_snapshot_lifecycle() {
    let mut snap = telemetry::TelemetrySnapshot::default();
    assert_eq!(snap.status, "warming_up");
    assert!(snap.cpu.is_none());
    assert!(snap.ram.is_none());

    snap.status = "ready".to_string();
    snap.cpu = Some(telemetry::CpuStats {
        usage_percent: 24.5,
        frequency_mhz: None,
    });
    snap.ram = Some(telemetry::RamStats {
        total_bytes: 32_000_000_000,
        used_bytes: 16_000_000_000,
        free_bytes: 16_000_000_000,
        usage_percent: 50.0,
    });

    let json = serde_json::to_string(&snap).expect("Failed to serialize TelemetrySnapshot");
    assert!(json.contains("\"status\":\"ready\""));
    assert!(json.contains("\"usagePercent\":24.5"));
    assert!(json.contains("\"totalBytes\":32000000000"));
}

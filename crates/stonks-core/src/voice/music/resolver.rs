//! Local-file, direct-URL and bundled-tool media resolution.
use super::protocol::{public_source_url, safe_artwork_url, transferable_url, MAX_QUEUE_ITEMS};
use serde::Deserialize;
use std::{path::PathBuf, time::Duration};
use tokio::process::Command;

#[derive(Debug, Clone)]
pub(super) struct ResolvedMedia {
    pub(super) title: String,
    pub(super) artist: Option<String>,
    pub(super) stream_url: String,
    pub(super) transfer_url: Option<String>,
    pub(super) source_url: String,
    pub(super) artwork_url: Option<String>,
    pub(super) duration_seconds: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct YtDlpInfo {
    title: Option<String>,
    id: Option<String>,
    artist: Option<String>,
    uploader: Option<String>,
    url: Option<String>,
    webpage_url: Option<String>,
    availability: Option<String>,
    thumbnail: Option<String>,
    duration: Option<f64>,
    requested_formats: Option<Vec<YtDlpFormat>>,
    entries: Option<Vec<Option<YtDlpInfo>>>,
}

#[derive(Debug, Deserialize)]
struct YtDlpFormat {
    url: Option<String>,
    acodec: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct ResolvedCollectionEntry {
    pub(super) title: String,
    pub(super) playback_source: String,
    pub(super) source_url: String,
    pub(super) artwork_url: Option<String>,
}

pub(super) fn is_collection_url(value: &str) -> bool {
    let Ok(url) = url::Url::parse(value) else {
        return false;
    };
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return false;
    }
    let has_video = if url.host_str() == Some("youtu.be") {
        url.path_segments()
            .and_then(|mut segments| segments.next())
            .is_some_and(|id| !id.is_empty())
    } else {
        url.path() == "/watch"
            && url
                .query_pairs()
                .any(|(key, value)| key == "v" && !value.is_empty())
    };
    // YouTube radio mixes can return no entries in flat-playlist mode.
    // Play the seed video instead of importing the generated mix.
    let radio_track = has_video && is_youtube_url(value);
    url.query_pairs().any(|(key, value)| {
        key == "list" && !value.is_empty() && !(radio_track && value.starts_with("RD"))
    }) || url.path_segments().is_some_and(|mut segments| {
        segments.any(|segment| {
            matches!(
                segment.to_ascii_lowercase().as_str(),
                "playlist" | "playlists" | "album" | "albums" | "set" | "sets"
            )
        })
    })
}

fn is_youtube_url(value: &str) -> bool {
    let Ok(url) = url::Url::parse(value) else {
        return false;
    };
    matches!(
        url.host_str()
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "youtube.com" | "www.youtube.com" | "m.youtube.com" | "music.youtube.com" | "youtu.be"
    )
}

fn tool_directories(executable_directory: Option<PathBuf>) -> Vec<PathBuf> {
    let mut directories = Vec::new();
    if let Some(directory) = executable_directory {
        // The Windows installer places media tools beside the app under resources/media.
        directories.push(directory.join("resources").join("media"));
        directories.push(directory.join("media"));
        directories.push(directory);
    }
    #[cfg(target_os = "linux")]
    directories.push(PathBuf::from("/usr/lib/stonks/media"));
    directories
}

pub(super) fn tool_path(name: &str) -> PathBuf {
    let env_name = format!(
        "STONKS_{}_PATH",
        name.replace('-', "_").to_ascii_uppercase()
    );
    if let Some(path) = std::env::var_os(env_name) {
        return PathBuf::from(path);
    }
    let executable = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    };
    let directories = tool_directories(
        std::env::current_exe()
            .ok()
            .and_then(|current| current.parent().map(PathBuf::from)),
    );
    find_tool(&executable, directories)
}

fn find_tool(executable: &str, directories: Vec<PathBuf>) -> PathBuf {
    // The packages ship canonical names; use the override for custom filenames.
    // Prefix matches can select directories, notices, or unrelated binaries.
    directories
        .into_iter()
        .map(|directory| directory.join(executable))
        .find(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from(executable))
}

pub(super) async fn resolve_media(
    requested: &str,
    local_file: bool,
) -> Result<ResolvedMedia, String> {
    if local_file {
        let path = PathBuf::from(requested);
        if !path.is_file() {
            return Err("The selected audio file is no longer available.".into());
        }
        let title = path
            .file_stem()
            .and_then(|value| value.to_str())
            .filter(|value| !value.is_empty())
            .unwrap_or("Local audio")
            .to_owned();
        return Ok(ResolvedMedia {
            title,
            artist: None,
            transfer_url: None,
            stream_url: path.to_string_lossy().into_owned(),
            source_url: "Local file".into(),
            artwork_url: None,
            duration_seconds: None,
        });
    }
    let parsed = url::Url::parse(requested).map_err(|_| "The media URL is invalid.".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err("The media URL is invalid.".into());
    }
    if !is_youtube_url(requested) {
        let title = parsed.host_str().unwrap_or("Audio stream").to_owned();
        return Ok(ResolvedMedia {
            title,
            artist: None,
            transfer_url: None,
            stream_url: requested.into(),
            source_url: public_source_url(requested),
            artwork_url: None,
            duration_seconds: None,
        });
    }

    let js_runtime = format!("deno:{}", tool_path("deno").to_string_lossy());
    let output = tokio::time::timeout(
        Duration::from_secs(45),
        yt_dlp_command()
            .args([
                "--no-config",
                "--no-playlist",
                "--no-warnings",
                "--dump-single-json",
                "--js-runtimes",
                &js_runtime,
                "-f",
                "bestaudio/best",
                requested,
            ])
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| "The media resolver timed out.".to_string())?
    .map_err(|_| "The media resolver is unavailable.".to_string())?;
    if !output.status.success() {
        return Err("The media resolver could not load this track.".into());
    }
    parse_youtube_metadata(requested, &output.stdout)
}

fn yt_dlp_command() -> Command {
    #[allow(unused_mut)]
    let mut command = Command::new(tool_path("yt-dlp"));
    #[cfg(target_os = "windows")]
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    command
}

pub(super) async fn resolve_collection(
    requested: &str,
) -> Result<Vec<ResolvedCollectionEntry>, String> {
    if !is_collection_url(requested) {
        return Err("The playlist or album URL is invalid.".into());
    }
    let js_runtime = format!("deno:{}", tool_path("deno").to_string_lossy());
    let playlist_end = (MAX_QUEUE_ITEMS + 1).to_string();
    let output = tokio::time::timeout(
        Duration::from_secs(90),
        yt_dlp_command()
            .args([
                "--no-config",
                "--flat-playlist",
                "--no-warnings",
                "--dump-single-json",
                "--playlist-end",
                &playlist_end,
                "--js-runtimes",
                &js_runtime,
                requested,
            ])
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| "The playlist resolver timed out.".to_string())?
    .map_err(|_| "The playlist resolver is unavailable.".to_string())?;
    if !output.status.success() {
        return Err("The playlist or album could not be loaded.".into());
    }
    parse_collection_metadata(requested, &output.stdout)
}

fn parse_collection_metadata(
    requested: &str,
    bytes: &[u8],
) -> Result<Vec<ResolvedCollectionEntry>, String> {
    let info: YtDlpInfo = serde_json::from_slice(bytes)
        .map_err(|_| "The playlist resolver returned invalid metadata.".to_string())?;
    let entries = info
        .entries
        .ok_or_else(|| "The URL does not contain a playlist or album.".to_string())?;
    if entries.is_empty() {
        return Err("The playlist or album is empty.".into());
    }
    if entries.len() > MAX_QUEUE_ITEMS {
        return Err("The shared music queue is full (100 tracks).".into());
    }
    entries
        .into_iter()
        .map(|entry| {
            let entry =
                entry.ok_or_else(|| "The playlist contains an unavailable track.".to_string())?;
            let playback_source = entry
                .webpage_url
                .or_else(|| entry.url.filter(|value| url::Url::parse(value).is_ok()))
                .or_else(|| {
                    (is_youtube_url(requested))
                        .then(|| {
                            entry
                                .id
                                .map(|id| format!("https://www.youtube.com/watch?v={id}"))
                        })
                        .flatten()
                })
                .ok_or_else(|| "The playlist contains an invalid track.".to_string())?;
            let source_url = public_source_url(&playback_source);
            if source_url.is_empty() {
                return Err("The playlist contains an invalid track.".to_string());
            }
            Ok(ResolvedCollectionEntry {
                title: entry.title.unwrap_or_else(|| "Queued track".into()),
                playback_source,
                source_url,
                artwork_url: entry.thumbnail.as_deref().and_then(safe_artwork_url),
            })
        })
        .collect()
}

fn parse_youtube_metadata(requested: &str, bytes: &[u8]) -> Result<ResolvedMedia, String> {
    let info: YtDlpInfo = serde_json::from_slice(bytes)
        .map_err(|_| "The media resolver returned invalid metadata.".to_string())?;
    let stream_url = info
        .url
        .or_else(|| {
            info.requested_formats.and_then(|formats| {
                formats
                    .into_iter()
                    .find(|format| {
                        format
                            .acodec
                            .as_deref()
                            .is_some_and(|codec| codec != "none")
                    })
                    .and_then(|format| format.url)
            })
        })
        .ok_or("The media resolver did not return an audio stream.")?;
    Ok(ResolvedMedia {
        title: info.title.unwrap_or_else(|| "YouTube audio".into()),
        artist: info.artist.or(info.uploader),
        stream_url,
        transfer_url: (info.availability.as_deref() == Some("public"))
            .then(|| info.webpage_url.as_deref().and_then(transferable_url))
            .flatten(),
        source_url: public_source_url(info.webpage_url.as_deref().unwrap_or(requested)),
        artwork_url: info.thumbnail.as_deref().and_then(safe_artwork_url),
        duration_seconds: info
            .duration
            .filter(|value| value.is_finite() && *value >= 0.0)
            .map(|value| value.round() as u64),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_confirmed_public_canonical_video_pages_are_transferable() {
        let page = "https://www.youtube.com/watch?v=dQw4w9WgXcQ";
        for (availability, webpage, expected) in [
            ("public", page, Some(page)),
            ("unlisted", page, None),
            ("private", page, None),
            (
                "public",
                "https://www.youtube.com/watch?v=dQw4w9WgXcQ&token=secret",
                None,
            ),
        ] {
            let metadata = serde_json::json!({
                "availability": availability,
                "webpage_url": webpage,
                "url": "https://example.com/stream?signature=private"
            });
            let media =
                parse_youtube_metadata(page, &serde_json::to_vec(&metadata).unwrap()).unwrap();
            assert_eq!(media.transfer_url.as_deref(), expected);
        }
    }

    #[test]
    fn metadata_selects_audio_and_preserves_public_source() {
        let media = parse_youtube_metadata(
            "https://youtu.be/test",
            br#"{
            "title":"Track", "uploader":"Creator", "duration":12.6,
            "requested_formats":[
                {"url":"https://cdn.test/video", "acodec":"none"},
                {"url":"https://cdn.test/audio", "acodec":"opus"}
            ]
        }"#,
        )
        .unwrap();
        assert_eq!(media.stream_url, "https://cdn.test/audio");
        assert_eq!(media.source_url, "https://youtu.be/");
        assert_eq!(media.title, "Track");
        assert_eq!(media.artist.as_deref(), Some("Creator"));
        assert_eq!(media.duration_seconds, Some(13));
    }

    #[test]
    fn metadata_prefers_direct_stream_and_rejects_negative_duration() {
        let media = parse_youtube_metadata(
            "https://youtu.be/test",
            br#"{
            "url":"https://cdn.test/direct", "artist":"Artist", "uploader":"Uploader",
            "webpage_url":"https://www.youtube.com/watch?v=test", "duration":-1,
            "requested_formats":[{"url":"https://cdn.test/fallback", "acodec":"opus"}]
        }"#,
        )
        .unwrap();
        assert_eq!(media.stream_url, "https://cdn.test/direct");
        assert_eq!(media.artist.as_deref(), Some("Artist"));
        assert_eq!(media.source_url, "https://www.youtube.com/");
        assert_eq!(media.duration_seconds, None);
    }

    #[test]
    fn malformed_metadata_and_missing_audio_fail() {
        assert!(parse_youtube_metadata("https://youtu.be/test", b"not json").is_err());
        assert!(parse_youtube_metadata(
            "https://youtu.be/test",
            br#"{
            "requested_formats":[{"url":"https://cdn.test/video", "acodec":"none"}]
        }"#
        )
        .is_err());
    }

    #[test]
    fn collection_metadata_preserves_order_and_keeps_locators_private() {
        let entries = parse_collection_metadata(
            "https://www.youtube.com/playlist?list=ordered",
            br#"{
                "entries": [
                    {"id":"first", "title":"First", "url":"first"},
                    {"id":"second", "title":"Second", "webpage_url":"https://www.youtube.com/watch?v=second&token=private"}
                ]
            }"#,
        )
        .unwrap();
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.title.as_str())
                .collect::<Vec<_>>(),
            vec!["First", "Second"]
        );
        assert_eq!(
            entries[0].playback_source,
            "https://www.youtube.com/watch?v=first"
        );
        assert!(entries[1].playback_source.contains("token=private"));
        assert_eq!(entries[1].source_url, "https://www.youtube.com/");
        assert!(!serde_json::to_string(&entries[1].source_url)
            .unwrap()
            .contains("private"));
    }

    #[test]
    fn malformed_and_empty_collections_fail() {
        assert!(parse_collection_metadata("https://example.com/playlist/1", b"not json").is_err());
        assert!(
            parse_collection_metadata("https://example.com/playlist/1", br#"{"entries":[]}"#,)
                .is_err()
        );
        assert!(parse_collection_metadata(
            "https://example.com/playlist/1",
            br#"{"entries":[null]}"#,
        )
        .is_err());
    }

    #[test]
    fn recognizes_playlist_and_album_urls_without_treating_tracks_as_collections() {
        assert!(is_collection_url("https://youtube.com/watch?v=a&list=b"));
        assert!(is_collection_url("https://example.com/albums/example"));
        assert!(!is_collection_url("https://youtu.be/abc"));
        assert!(!is_collection_url("not a url"));
    }

    #[test]
    fn finds_canonical_windows_tools_in_the_installed_layout() {
        let root = tempfile::tempdir().unwrap();
        let app = root.path().join("Stonks GPUI");
        let media = app.join("resources").join("media");
        std::fs::create_dir_all(&media).unwrap();
        for tool in ["ffmpeg.exe", "yt-dlp.exe", "deno.exe"] {
            let bundled = media.join(tool);
            std::fs::write(&bundled, []).unwrap();
            std::fs::write(app.join(tool), []).unwrap();
            assert_eq!(
                find_tool(tool, tool_directories(Some(app.clone()))),
                bundled
            );
        }
    }

    #[test]
    fn skips_non_tools_and_falls_back_to_the_path_name() {
        let root = tempfile::tempdir().unwrap();
        let media = root.path().join("resources").join("media");
        std::fs::create_dir_all(media.join("ffmpeg.exe")).unwrap();
        std::fs::write(media.join("ffmpeg-license.txt"), []).unwrap();
        let directories = vec![media, root.path().to_owned()];
        let alternate = root.path().join("ffmpeg.exe");
        std::fs::write(&alternate, []).unwrap();
        assert_eq!(find_tool("ffmpeg.exe", directories.clone()), alternate);
        std::fs::remove_file(alternate).unwrap();
        assert_eq!(
            find_tool("ffmpeg.exe", directories),
            PathBuf::from("ffmpeg.exe")
        );
    }

    #[test]
    fn plays_youtube_radio_seed_without_importing_the_mix() {
        assert!(!is_collection_url(
            "https://music.youtube.com/watch?v=jNQXAC9IVRw&list=RDAMVMjNQXAC9IVRw"
        ));
        assert!(!is_collection_url(
            "https://youtu.be/jNQXAC9IVRw?list=RDjNQXAC9IVRw"
        ));
        assert!(is_collection_url(
            "https://www.youtube.com/playlist?list=RDjNQXAC9IVRw"
        ));
        assert!(is_collection_url(
            "https://music.youtube.com/playlist?list=OLAK5uy_example"
        ));
        assert!(is_collection_url(
            "https://www.youtube.com/watch?v=a&list=PLexample"
        ));
        assert!(is_collection_url(
            "https://example.com/watch?v=a&list=RDexample"
        ));
    }

    #[test]
    fn recognizes_supported_youtube_hosts_only() {
        assert!(is_youtube_url("https://youtu.be/abc"));
        assert!(is_youtube_url("https://music.youtube.com/watch?v=abc"));
        assert!(!is_youtube_url(
            "https://youtube.com.evil.example/watch?v=abc"
        ));
        assert!(!is_youtube_url("https://example.com/audio.mp3"));
    }
}

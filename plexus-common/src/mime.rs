//! MIME type detection by file extension and magic bytes.
//!
//! Returns `&'static str` so callers avoid allocation. Unknown or missing
//! extensions resolve to `application/octet-stream`.

/// Map a filename (or path) to a MIME type based on its extension.
///
/// - Case-insensitive (`FOO.PNG` → `image/png`).
/// - Handles compound extensions (`a.tar.gz` / `a.tgz` → `application/gzip`).
/// - Inputs without a `.` (e.g. `"png"`, `"README"`) return the default
///   `application/octet-stream` — we only classify actual extensions.
pub fn detect_mime_from_extension(filename: &str) -> &'static str {
    let lower = filename.to_ascii_lowercase();

    // Compound extensions first.
    if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        return "application/gzip";
    }

    // Require an actual '.' — bare tokens like "png" are not extensions.
    let Some(dot_idx) = lower.rfind('.') else {
        return "application/octet-stream";
    };
    let ext = &lower[dot_idx + 1..];

    match ext {
        // Images
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "heic" => "image/heic",
        "heif" => "image/heif",

        // Documents
        "pdf" => "application/pdf",

        // Text / markup
        "txt" | "log" => "text/plain",
        "md" => "text/markdown",
        "csv" => "text/csv",

        // Data / config
        "json" => "application/json",
        "toml" => "application/toml",
        "yaml" | "yml" => "application/yaml",

        // Source code
        "rs" => "text/x-rust",
        "py" => "text/x-python",
        "ts" | "tsx" => "application/typescript",
        "js" => "application/javascript",

        // Archives
        "zip" => "application/zip",
        "gz" => "application/gzip",

        // Audio
        "mp3" => "audio/mpeg",
        "ogg" | "oga" => "audio/ogg",
        "wav" => "audio/wav",

        // Video
        "mp4" => "video/mp4",
        "webm" => "video/webm",

        _ => "application/octet-stream",
    }
}

/// Sniff MIME from the first few bytes of a file. Returns `None` if the
/// signature is unrecognized (callers typically fall back to extension-based
/// detection).
pub fn detect_mime_from_bytes(data: &[u8]) -> Option<&'static str> {
    if data.len() < 4 {
        return None;
    }
    match &data[..4] {
        [0x89, b'P', b'N', b'G'] => Some("image/png"),
        [0xFF, 0xD8, 0xFF, _] => Some("image/jpeg"),
        [b'G', b'I', b'F', b'8'] => Some("image/gif"),
        [b'R', b'I', b'F', b'F'] if data.len() >= 12 && &data[8..12] == b"WEBP" => {
            Some("image/webp")
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comprehensive_coverage() {
        // From plexus-common baseline
        assert_eq!(detect_mime_from_extension("foo.png"), "image/png");
        assert_eq!(detect_mime_from_extension("foo.jpg"), "image/jpeg");
        assert_eq!(detect_mime_from_extension("foo.jpeg"), "image/jpeg");
        assert_eq!(detect_mime_from_extension("foo.gif"), "image/gif");
        assert_eq!(detect_mime_from_extension("foo.webp"), "image/webp");
        assert_eq!(detect_mime_from_extension("foo.bmp"), "image/bmp");
        assert_eq!(detect_mime_from_extension("foo.svg"), "image/svg+xml");
        assert_eq!(detect_mime_from_extension("foo.pdf"), "application/pdf");
        assert_eq!(
            detect_mime_from_extension("archive.tar.gz"),
            "application/gzip"
        );
        assert_eq!(
            detect_mime_from_extension("archive.tgz"),
            "application/gzip"
        );
        assert_eq!(detect_mime_from_extension("foo.zip"), "application/zip");

        // From api.rs — code/source types
        assert_eq!(detect_mime_from_extension("foo.rs"), "text/x-rust");
        assert_eq!(
            detect_mime_from_extension("foo.ts"),
            "application/typescript"
        );
        assert_eq!(
            detect_mime_from_extension("foo.tsx"),
            "application/typescript"
        );
        assert_eq!(
            detect_mime_from_extension("foo.js"),
            "application/javascript"
        );
        assert_eq!(detect_mime_from_extension("foo.py"), "text/x-python");
        assert_eq!(detect_mime_from_extension("foo.toml"), "application/toml");
        assert_eq!(detect_mime_from_extension("foo.yaml"), "application/yaml");
        assert_eq!(detect_mime_from_extension("foo.yml"), "application/yaml");
        assert_eq!(detect_mime_from_extension("foo.json"), "application/json");
        assert_eq!(detect_mime_from_extension("foo.md"), "text/markdown");
        assert_eq!(detect_mime_from_extension("foo.txt"), "text/plain");
        assert_eq!(detect_mime_from_extension("foo.log"), "text/plain");

        // From context.rs — audio/video/heic
        assert_eq!(detect_mime_from_extension("foo.heic"), "image/heic");
        assert_eq!(detect_mime_from_extension("foo.heif"), "image/heif");
        assert_eq!(detect_mime_from_extension("foo.csv"), "text/csv");
        assert_eq!(detect_mime_from_extension("foo.mp3"), "audio/mpeg");
        assert_eq!(detect_mime_from_extension("foo.ogg"), "audio/ogg");
        assert_eq!(detect_mime_from_extension("foo.oga"), "audio/ogg");
        assert_eq!(detect_mime_from_extension("foo.wav"), "audio/wav");
        assert_eq!(detect_mime_from_extension("foo.mp4"), "video/mp4");
        assert_eq!(detect_mime_from_extension("foo.webm"), "video/webm");

        // Default
        assert_eq!(
            detect_mime_from_extension("foo"),
            "application/octet-stream"
        );
        assert_eq!(
            detect_mime_from_extension("foo.unknown_ext"),
            "application/octet-stream"
        );
    }

    #[test]
    fn case_insensitive_extension() {
        assert_eq!(detect_mime_from_extension("FOO.PNG"), "image/png");
        assert_eq!(detect_mime_from_extension("Image.Jpg"), "image/jpeg");
        assert_eq!(
            detect_mime_from_extension("Archive.TAR.GZ"),
            "application/gzip"
        );
    }

    #[test]
    fn dotless_input() {
        // Bare extension tokens are NOT extensions — default.
        assert_eq!(
            detect_mime_from_extension("png"),
            "application/octet-stream"
        );
        assert_eq!(
            detect_mime_from_extension("README"),
            "application/octet-stream"
        );
        assert_eq!(detect_mime_from_extension(""), "application/octet-stream");
    }

    #[test]
    fn bytes_png() {
        assert_eq!(
            detect_mime_from_bytes(&[0x89, b'P', b'N', b'G']),
            Some("image/png")
        );
    }
    #[test]
    fn bytes_jpeg() {
        assert_eq!(
            detect_mime_from_bytes(&[0xFF, 0xD8, 0xFF, 0xE0]),
            Some("image/jpeg")
        );
    }
    #[test]
    fn bytes_gif() {
        assert_eq!(detect_mime_from_bytes(b"GIF89a"), Some("image/gif"));
    }
    #[test]
    fn bytes_webp() {
        assert_eq!(
            detect_mime_from_bytes(b"RIFF\x00\x00\x00\x00WEBP"),
            Some("image/webp")
        );
    }
    #[test]
    fn bytes_unknown() {
        assert_eq!(detect_mime_from_bytes(b"hello world!!"), None);
    }
}

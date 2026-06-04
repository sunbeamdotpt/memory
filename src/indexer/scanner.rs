use crate::error::Result;
use std::path::{Path, PathBuf};

/// Scan a directory (or single file) and return all file paths to ingest.
/// Respects `.gitignore` files when scanning inside git repositories.
pub fn scan_target(path: &Path) -> Result<Vec<PathBuf>> {
    if path.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }

    let mut files = Vec::new();

    for result in ignore::WalkBuilder::new(path)
        .hidden(false)           // we want hidden files unless gitignore excludes them
        .git_ignore(true)        // respect .gitignore
        .git_global(true)        // respect global gitignore
        .git_exclude(true)       // respect .git/info/exclude
        .follow_links(false)
        .build()
    {
        let entry = match result {
            Ok(e) => e,
            Err(_) => continue,
        };
        // Skip .git directories — ignore::WalkBuilder respects .gitignore
        // but does not automatically exclude .git/ internals.
        if entry.path().components().any(|c| {
            c.as_os_str() == ".git"
        }) {
            continue;
        }
        if entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            files.push(entry.path().to_path_buf());
        }
    }

    Ok(files)
}

/// Check if a file is likely binary by extension, magic bytes, or null-byte content.
///
/// This reads up to the first 8 KB of the file — fast enough to call from async code.
pub fn is_likely_binary(path: &Path) -> bool {
    // 1. Fast path: known binary extensions
    if has_binary_extension(path) {
        return true;
    }

    // 2. Read the first chunk and inspect magic bytes / null bytes
    let head = match std::fs::read(path) {
        Ok(data) => data,
        Err(_) => return false, // Can't read — let downstream handle it
    };

    if head.is_empty() {
        return false;
    }

    // Check magic bytes
    if has_binary_magic(&head) {
        return true;
    }

    // Check for null bytes in the first 8 KB (strong signal for binary data)
    let scan_len = head.len().min(8192);
    head[..scan_len].contains(&0)
}

fn has_binary_extension(path: &Path) -> bool {
    const BINARY_EXTS: &[&str] = &[
        "exe", "dll", "so", "dylib", "bin", "o", "a", "obj",
        "png", "jpg", "jpeg", "gif", "bmp", "ico", "webp", "avif",
        "mp3", "mp4", "wav", "avi", "mov", "mkv", "flac", "ogg",
        "zip", "tar", "gz", "bz2", "xz", "7z", "rar", "br",
        "doc", "docx", "xls", "xlsx", "ppt", "pptx",
        "ttf", "otf", "woff", "woff2", "eot",
        "class", "jar", "war", "pyc", "pyo", "wasm",
        "db", "sqlite", "sqlite3", "mdb", "accdb",
        "iso", "dmg", "img", "vmdk",
    ];

    if let Some(ext) = path.extension() {
        let ext = ext.to_string_lossy().to_lowercase();
        if BINARY_EXTS.contains(&ext.as_str()) {
            return true;
        }
    }
    false
}

/// Check file header against known magic signatures.
fn has_binary_magic(buf: &[u8]) -> bool {
    if buf.len() < 4 {
        return false;
    }

    match &buf[..4] {
        // Images
        b"\x89PNG" => true,                    // PNG
        [0xFF, 0xD8, 0xFF, ..] => true,       // JPEG
        b"GIF8" => true,                       // GIF
        b"RIFF" => true,                       // WEBP / WAV (RIFF container)
        b"MM\x00*" | b"II*\x00" => true,      // TIFF
        b"BM\x00\x00" | b"BM\x36\x00" => true, // BMP
        b"\x00\x00\x01\x00" | b"\x00\x00\x02\x00" => true, // ICO

        // Archives / containers
        b"PK\x03\x04" => true,                 // ZIP (also docx, xlsx, jar)
        [0x1F, 0x8B, ..] => true,              // GZIP
        b"BZh" => true,                        // BZIP2
        [0xFD, 0x37, 0x7A, 0x58] => true,      // XZ
        [0x52, 0x61, 0x72, 0x21] => true,      // RAR v4
        b"7z\xBC\xAF" => true,                 // 7z

        // Executables / libraries
        [0x7F, b'E', b'L', b'F'] => true,      // ELF
        b"MZ\x00\x00" | b"MZ\x90\x00" => true, // DOS/PE (Windows exe/dll)
        [0xCA, 0xFE, 0xBA, 0xBE] => true,      // Java class / Mach-O universal
        [0xCF, 0xFA, 0xED, 0xFE] => true,      // Mach-O 64-bit (little-endian)
        [0xFE, 0xED, 0xFA, 0xCF] => true,      // Mach-O 64-bit (big-endian)

        // Documents
        b"%PDF" => true,                       // PDF
        b"\xD0\xCF\x11\xE0" => true,           // OLE2 (old Office docs)

        // Audio / Video
        b"ID3" => true,                        // MP3 with ID3v2
        [0xFF, 0xFB, ..] | [0xFF, 0xF3, ..] | [0xFF, 0xF2, ..] => true, // MP3 without ID3
        b"ftyp" => true,                       // MP4 / MOV (if offset 4)
        b"\x1A\x45\xDF\xA3" => true,           // MKV / WebM (EBML)

        // Database / misc
        b"SQLite" => true,                     // SQLite 3

        _ => false,
    }
}

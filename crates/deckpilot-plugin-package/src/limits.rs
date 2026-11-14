//! Hard limits applied to every package, trusted or not.

use deckpilot_plugin_manifest::{MAX_PACKAGE_BYTES, MAX_PACKAGE_ENTRIES};

/// Tar header and padding blocks are 512 bytes.
pub(crate) const BLOCK: u64 = 512;

/// Size and shape limits (architecture section 8). [`Limits::default`] is
/// the #191 contract; tests and tools may tighten it, nothing should loosen
/// it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Compressed archive (`.dpplugin`): 5 MiB.
    pub max_archive_bytes: u64,
    /// Sum of all file sizes once unpacked: 10 MiB.
    pub max_unpacked_bytes: u64,
    /// Any single file: 5 MiB (an incompressible asset cannot be larger
    /// than the archive anyway).
    pub max_file_bytes: u64,
    /// `plugin.js`: 2 MiB.
    pub max_bundle_bytes: u64,
    /// Entries in the archive: 256.
    pub max_entries: usize,
    /// Largest decompressed-to-compressed ratio, checked once the
    /// decompressed tar stream is larger than [`Limits::ratio_exempt_bytes`].
    /// Real packages (JavaScript, PNG) stay far below 100:1; deflate can
    /// reach about 1000:1 on zeros.
    pub max_compression_ratio: u64,
    /// Below this decompressed size the ratio is not checked: a package of
    /// a few small files is mostly tar padding, which compresses very well.
    pub ratio_exempt_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_archive_bytes: MAX_PACKAGE_BYTES,
            max_unpacked_bytes: 10 * 1024 * 1024,
            max_file_bytes: 5 * 1024 * 1024,
            max_bundle_bytes: 2 * 1024 * 1024,
            max_entries: MAX_PACKAGE_ENTRIES,
            max_compression_ratio: 100,
            ratio_exempt_bytes: 1024 * 1024,
        }
    }
}

impl Limits {
    /// Largest decompressed tar stream that can hold a package within the
    /// limits: the file contents, one header and up to one padding block per
    /// entry, and the end-of-archive marker plus record padding (tar writers
    /// pad to 10 KiB records).
    pub fn max_tar_stream_bytes(&self) -> u64 {
        let entries = u64::try_from(self.max_entries).unwrap_or(u64::MAX);
        self.max_unpacked_bytes
            .saturating_add(entries.saturating_mul(2 * BLOCK))
            .saturating_add(20 * BLOCK)
    }
}

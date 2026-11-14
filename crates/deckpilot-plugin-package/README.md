# deckpilot-plugin-package

**Status: experimental.** Not used by DeckPilot yet. No app, agent or module
crate depends on it, and plugins stay disabled; nothing a user sees changes.

The plugin package format (`.dpplugin`) of the plugin marketplace design
(`docs/marketplace/architecture.md`, proposed in pull request #191),
sections 8, 9.3 step 5, 12.3 steps 4 to 8 and 12.5. It promotes the
package half of `experiments/marketplace-spikes/trust` (spike 4).

## API

- `PackageBuilder::add(path, bytes)` then `build()`: a deterministic
  gzip-compressed ustar archive and its `PackageIntegrity` (from
  `deckpilot-plugin-manifest`). The builder refuses anything the reader
  would refuse.
- `verify_package(bytes, &ExpectedPackage, &Limits) -> VerifiedPackage`:
  size and sha256 against the registry pins **before** anything is
  decompressed, then the safe read, then the manifest checks.
- `VerifiedPackage::unpack(staging_root, target)`: only a verified
  package can be unpacked.
- `verify_installed(dir, &PackageIntegrity)`: the load-time check of
  section 12.5.
- `read_archive(bytes, &Limits)`: the structural read alone, without any
  authenticity (tools, development sideloading).

## Determinism

Entries sorted bytewise by path; ustar headers with mode 0644, mtime 0, uid
and gid 0, empty owner names, typeflag `0`; no directory entries; gzip
header with mtime 0, no name, XFL 2 and OS 255, at level 9. The same input
gives the same bytes with the same compressor. Identical output across
deflate implementations (miniz_oxide, zlib, zlib-rs, Node's zlib) is **not**
promised: the registry pins whatever it built, so clients never need to
rebuild. Cross-tool reproducibility stays spike S6.

## Reader checks, in order

1. Archive at most 5 MiB; size equal to the pinned size; sha256 equal to
   the pinned digest.
2. gzip magic; one gzip member that ends exactly at the end of the input
   (no trailing data, no second member); CRC and length checked by flate2.
3. Decompressed stream capped at the unpacked limit plus header overhead
   (absolute bomb limit), and at most 100:1 once above 1 MiB (ratio limit).
4. Per entry, read raw (pax and GNU extensions are not interpreted): ustar
   magic only (GNU, v7 and pax refused), typeflag `0` only (directories,
   symlinks, hard links, devices, FIFOs, sparse, pax `x`/`g`, GNU `L`/`K`
   refused), no link target, UTF-8 name, `$defs/relativePath` (no absolute
   paths, `..`, `.`, empty segments, backslashes, characters outside
   `[A-Za-z0-9._-]`, segments starting with `.` or `-`, over 128
   characters), no Windows device names (`CON`, `NUL`, `COM1`… with any
   extension) or segments ending in `.`, at most 256 entries, the declared
   size at most 5 MiB per file before reading, at most 10 MiB in total,
   and zero padding.
5. Nothing after the last entry but the zero end-of-archive marker.
6. The path set: no duplicates, no two paths that differ only in case, no
   file that is also another file's directory.
7. `manifest.json`, `plugin.js` (at most 2 MiB) and `LICENSE` present;
   `manifest.json` sha256 equal to the pinned `manifest_sha256` before it
   is parsed; the manifest validates; its `id` and `version` equal the
   pinned ones; its `permissions` equal (as JSON values) the permissions
   shown for consent; every file named by the manifest present and no
   other file (`PackageIntegrity::validate`).

PNG decoding (section 7 rule 12) and the load check of section 12.3 step 6
belong to the installer and are not done here.

## Unpacking

`unpack` refuses an existing target (also an empty directory, which a Unix
rename would replace), creates a fresh `unpack-*` directory under the
staging root, writes every file with `create_new`, syncs it, marks it
read-only, syncs the directories, checks the target again and moves the
tree into place with one `rename` (staging and target must be on the same
volume). On any error the staging directory is removed. The check of an
existing target and the rename are not one atomic step; the agent is the
only writer of the plugin directories (section 12.2).

## Tests

`cargo test -p deckpilot-plugin-package`: 26 tests. Determinism and header
properties; a full build, verify, unpack and load check; every single-bit
change at every byte offset (two bits per byte) rejected against the pins,
and with the flipped bytes re-pinned never accepted with different
contents; truncation to every length; trailing data and a second gzip
member; another plugin or version served; manifest hash and permissions
mismatches; zip-slip, absolute and Windows paths; symlinks (also pointing
outside), hard links, devices, directories, FIFOs, pax path overrides, GNU
long names, GNU and v7 headers; duplicates, case collisions and file versus
directory conflicts; Windows device names and non-UTF-8 names; an 11 MiB
gzip bomb and a 1000:1 ratio bomb; per-file, total, bundle and entry-count
limits including a lying size field; hidden data in padding and after the
end marker; a missing end marker and bad header checksums; missing
required or referenced files and files the manifest does not name; refusal
to overwrite; modified, extra, missing and symlinked files at load.

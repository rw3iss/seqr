//! Attachment encryption, chunking, and streaming reassembly.
//!
//! A file is encrypted **per chunk** with the conversation key (ChaCha20-Poly1305),
//! the AAD binding each chunk to its attachment id and index — so a chunk cannot be
//! swapped, reordered, or replayed into another attachment. Chunks travel as ordinary
//! packets through the existing delivery path (direct or mailbox). The receiver writes
//! decrypted chunks straight to disk by index, so a 1 GB file never sits in memory.
//!
//! At-rest note: reassembled files are stored decrypted under `attachments/`. The
//! network transfer is fully E2E-encrypted; encrypting attachments at rest with the
//! vault key is a planned follow-up.

use std::collections::HashSet;
use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use seqr_crypto::{aead, SymmetricKey};

use super::vault::AttachmentInfo;
use super::CoreError;

/// Plaintext bytes per chunk (ciphertext is a little larger: +nonce +tag).
pub const CHUNK_SIZE: usize = 512 * 1024;
/// Maximum attachment size accepted.
pub const MAX_ATTACHMENT: u64 = 1024 * 1024 * 1024; // 1 GiB

fn chunk_aad(att_id: &str, index: u32) -> Vec<u8> {
    format!("seqr-att|{att_id}|{index}").into_bytes()
}

/// Number of chunks for a file of `size` bytes.
pub fn chunk_count(size: u64) -> u32 {
    if size == 0 {
        return 1;
    }
    ((size + CHUNK_SIZE as u64 - 1) / CHUNK_SIZE as u64) as u32
}

pub fn seal_chunk(key: &SymmetricKey, att_id: &str, index: u32, plaintext: &[u8]) -> Vec<u8> {
    aead::seal(key, plaintext, &chunk_aad(att_id, index))
}

pub fn open_chunk(
    key: &SymmetricKey,
    att_id: &str,
    index: u32,
    ciphertext: &[u8],
) -> Result<Vec<u8>, CoreError> {
    aead::open(key, ciphertext, &chunk_aad(att_id, index)).map_err(Into::into)
}

pub fn new_attachment_id() -> String {
    hex::encode(&seqr_crypto::group::generate_group_key()[..16])
}

pub fn attachments_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("attachments")
}

/// Final on-disk path for a reassembled attachment.
pub fn attachment_path(data_dir: &Path, att_id: &str) -> PathBuf {
    attachments_dir(data_dir).join(att_id)
}

/// Best-effort MIME type from a filename extension.
pub fn guess_mime(filename: &str) -> String {
    let ext = filename.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    let m = match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "pdf" => "application/pdf",
        "txt" => "text/plain",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    };
    m.to_string()
}

/// Accumulates incoming chunks for one attachment, writing them to a temp file by
/// index. When all chunks have arrived, finalize() promotes it to its final path.
pub struct Reassembler {
    pub info: AttachmentInfo,
    pub conversation_id: String,
    pub sender: String,
    pub seq: u64,
    key: SymmetricKey,
    expected: u32,
    received: HashSet<u32>,
    temp_path: PathBuf,
    final_path: PathBuf,
    file: File,
}

impl Reassembler {
    pub fn new(
        data_dir: &Path,
        info: AttachmentInfo,
        conversation_id: String,
        sender: String,
        seq: u64,
        key: SymmetricKey,
        expected: u32,
    ) -> Result<Self, CoreError> {
        std::fs::create_dir_all(attachments_dir(data_dir))?;
        let final_path = attachment_path(data_dir, &info.id);
        let temp_path = final_path.with_extension("part");
        let file = File::create(&temp_path)?;
        Ok(Self {
            info,
            conversation_id,
            sender,
            seq,
            key,
            expected,
            received: HashSet::new(),
            temp_path,
            final_path,
            file,
        })
    }

    /// Decrypt and write one chunk by index. Returns true when the attachment is complete.
    pub fn add_chunk(&mut self, index: u32, ciphertext: &[u8]) -> Result<bool, CoreError> {
        if self.received.contains(&index) {
            return Ok(self.received.len() as u32 == self.expected);
        }
        let plain = open_chunk(&self.key, &self.info.id, index, ciphertext)?;
        self.file.seek(SeekFrom::Start(index as u64 * CHUNK_SIZE as u64))?;
        self.file.write_all(&plain)?;
        self.received.insert(index);
        Ok(self.received.len() as u32 == self.expected)
    }

    /// Flush and move the temp file to its final location.
    pub fn finalize(mut self) -> Result<AttachmentInfo, CoreError> {
        self.file.flush()?;
        drop(self.file);
        std::fs::rename(&self.temp_path, &self.final_path)?;
        Ok(self.info)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_roundtrip_and_aad_binding() {
        let key = [5u8; 32];
        let ct = seal_chunk(&key, "abc", 3, b"hello chunk");
        assert_eq!(open_chunk(&key, "abc", 3, &ct).unwrap(), b"hello chunk");
        // Wrong index (AAD) must fail.
        assert!(open_chunk(&key, "abc", 4, &ct).is_err());
        // Wrong attachment id must fail.
        assert!(open_chunk(&key, "xyz", 3, &ct).is_err());
    }

    #[test]
    fn chunk_count_math() {
        assert_eq!(chunk_count(0), 1);
        assert_eq!(chunk_count(1), 1);
        assert_eq!(chunk_count(CHUNK_SIZE as u64), 1);
        assert_eq!(chunk_count(CHUNK_SIZE as u64 + 1), 2);
    }

    #[test]
    fn reassemble_two_chunks_out_of_order() {
        let dir = std::env::temp_dir().join(format!("seqr-att-{}", new_attachment_id()));
        let key = [9u8; 32];
        let id = "att1".to_string();
        // Realistic layout: chunk 0 is full-size, chunk 1 is the remainder. The
        // reassembler places chunk i at offset i*CHUNK_SIZE.
        let chunk0 = vec![b'A'; CHUNK_SIZE];
        let chunk1 = b"BBB".to_vec();
        let size = (CHUNK_SIZE + chunk1.len()) as u64;
        let info = AttachmentInfo { id: id.clone(), filename: "f.bin".into(), mime: "x".into(), size };
        let c0 = seal_chunk(&key, &id, 0, &chunk0);
        let c1 = seal_chunk(&key, &id, 1, &chunk1);
        let mut r = Reassembler::new(&dir, info, "c".into(), "s".into(), 0, key, 2).unwrap();
        // out of order: chunk 1 then 0
        assert!(!r.add_chunk(1, &c1).unwrap());
        assert!(r.add_chunk(0, &c0).unwrap());
        let info = r.finalize().unwrap();
        let bytes = std::fs::read(attachment_path(&dir, &info.id)).unwrap();
        assert_eq!(bytes.len() as u64, size);
        assert_eq!(&bytes[..CHUNK_SIZE], &chunk0[..]);
        assert_eq!(&bytes[CHUNK_SIZE..], b"BBB");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

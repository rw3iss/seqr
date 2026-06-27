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
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use seqr_crypto::{aead, SymmetricKey};

use super::vault::AttachmentInfo;
use super::CoreError;

// ---- At-rest encryption ----
//
// On disk, an attachment is stored as a sequence of `[u32 BE sealed_len][sealed]`
// records, each chunk sealed with the **vault key** (separate from the conversation key
// used in transit). So local storage is encrypted too; the file never sits in plaintext
// on disk beyond a transient reassembly temp.

fn rest_aad(att_id: &str, index: u32) -> Vec<u8> {
    format!("seqr-att-rest|{att_id}|{index}").into_bytes()
}

/// Encrypt a plaintext file (`src`) to the at-rest sealed file (`dest`) with `vault_key`.
pub fn encrypt_file_to_rest(
    vault_key: &SymmetricKey,
    att_id: &str,
    src: &Path,
    dest: &Path,
) -> Result<(), CoreError> {
    let mut input = File::open(src)?;
    let mut out = File::create(dest)?;
    let mut buf = vec![0u8; CHUNK_SIZE];
    let mut index = 0u32;
    loop {
        let n = fill(&mut input, &mut buf)?;
        if n == 0 && index > 0 {
            break; // done (handles empty-file case via the index==0 seal below)
        }
        let sealed = aead::seal(vault_key, &buf[..n], &rest_aad(att_id, index));
        out.write_all(&(sealed.len() as u32).to_be_bytes())?;
        out.write_all(&sealed)?;
        index += 1;
        if n < CHUNK_SIZE {
            break;
        }
    }
    Ok(())
}

/// Decrypt an at-rest file, writing plaintext to `out`.
pub fn decrypt_rest_to_writer(
    vault_key: &SymmetricKey,
    att_id: &str,
    src: &Path,
    out: &mut impl Write,
) -> Result<(), CoreError> {
    let mut input = File::open(src)?;
    let mut index = 0u32;
    loop {
        let mut len_buf = [0u8; 4];
        if input.read_exact(&mut len_buf).is_err() {
            break; // EOF
        }
        let len = u32::from_be_bytes(len_buf) as usize;
        let mut sealed = vec![0u8; len];
        input.read_exact(&mut sealed).map_err(|e| CoreError::Storage(e.to_string()))?;
        let plain = aead::open(vault_key, &sealed, &rest_aad(att_id, index))?;
        out.write_all(&plain)?;
        index += 1;
    }
    Ok(())
}

/// Decrypt an at-rest file fully into memory (for inline image preview; caller caps size).
pub fn read_rest_bytes(vault_key: &SymmetricKey, att_id: &str, src: &Path) -> Result<Vec<u8>, CoreError> {
    let mut out = Vec::new();
    decrypt_rest_to_writer(vault_key, att_id, src, &mut out)?;
    Ok(out)
}

/// Read up to `buf.len()` bytes, looping until full or EOF. Returns bytes read.
fn fill(file: &mut File, buf: &mut [u8]) -> Result<usize, CoreError> {
    let mut filled = 0;
    while filled < buf.len() {
        match file.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) => return Err(CoreError::Storage(e.to_string())),
        }
    }
    Ok(filled)
}

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
    vault_key: SymmetricKey,
    expected: u32,
    received: HashSet<u32>,
    temp_path: PathBuf,
    final_path: PathBuf,
    file: File,
}

impl Reassembler {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        data_dir: &Path,
        info: AttachmentInfo,
        conversation_id: String,
        sender: String,
        seq: u64,
        key: SymmetricKey,
        vault_key: SymmetricKey,
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
            vault_key,
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

    /// Flush the plaintext temp file, encrypt it to the at-rest file with the vault key,
    /// then delete the temp.
    pub fn finalize(mut self) -> Result<AttachmentInfo, CoreError> {
        self.file.flush()?;
        drop(self.file);
        encrypt_file_to_rest(&self.vault_key, &self.info.id, &self.temp_path, &self.final_path)?;
        let _ = std::fs::remove_file(&self.temp_path);
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
        let vault_key = [3u8; 32];
        let c0 = seal_chunk(&key, &id, 0, &chunk0);
        let c1 = seal_chunk(&key, &id, 1, &chunk1);
        let mut r =
            Reassembler::new(&dir, info, "c".into(), "s".into(), 0, key, vault_key, 2).unwrap();
        // out of order: chunk 1 then 0
        assert!(!r.add_chunk(1, &c1).unwrap());
        assert!(r.add_chunk(0, &c0).unwrap());
        let info = r.finalize().unwrap();
        // On disk it's encrypted at rest; decrypt with the vault key to verify.
        let bytes = read_rest_bytes(&vault_key, &info.id, &attachment_path(&dir, &info.id)).unwrap();
        assert_eq!(bytes.len() as u64, size);
        assert_eq!(&bytes[..CHUNK_SIZE], &chunk0[..]);
        assert_eq!(&bytes[CHUNK_SIZE..], b"BBB");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

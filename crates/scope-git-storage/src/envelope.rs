use crate::GitStorageError;
use chacha20poly1305::{
    ChaCha20Poly1305, Key, Nonce,
    aead::{Aead, KeyInit, Payload},
};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use tokio::io::{AsyncRead, AsyncReadExt};

pub const ENCODING_VERSION: u32 = 2;
const MAGIC: &[u8; 8] = b"SCGSEG02";
const TAG_BYTES: usize = 16;
const FINAL_FLAG: u8 = 1;
const HEADER_FIXED_BYTES: usize = MAGIC.len() + 4 + 2 + 8 + 4;
const FRAME_HEADER_BYTES: usize = 4 + 4 + 1;
const MAX_KEY_ID_BYTES: usize = 1024;
const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone)]
pub struct SegmentEncryptionKey {
    key_id: String,
    key: [u8; 32],
}

impl SegmentEncryptionKey {
    pub fn new(key_id: impl Into<String>, key: [u8; 32]) -> Result<Self, GitStorageError> {
        let key_id = key_id.into();
        if key_id.is_empty() || key_id.len() > MAX_KEY_ID_BYTES {
            return Err(GitStorageError::InvalidConfiguration(format!(
                "encryption key id must contain 1 to {MAX_KEY_ID_BYTES} bytes"
            )));
        }
        Ok(Self { key_id, key })
    }

    pub fn key_id(&self) -> &str {
        &self.key_id
    }
}

pub(crate) struct EnvelopeWriter {
    cipher: ChaCha20Poly1305,
    header: Vec<u8>,
    nonce_prefix: [u8; 8],
    repository_id: String,
    segment_id: String,
    next_counter: u32,
}

impl EnvelopeWriter {
    pub(crate) fn new(
        key: &SegmentEncryptionKey,
        repository_id: &str,
        segment_id: &str,
        frame_bytes: usize,
    ) -> Result<Self, GitStorageError> {
        let frame_bytes = u32::try_from(frame_bytes).map_err(|_| {
            GitStorageError::InvalidConfiguration("encryption frame size exceeds u32".into())
        })?;
        let key_id_bytes = key.key_id.as_bytes();
        let key_id_len = u16::try_from(key_id_bytes.len()).map_err(|_| {
            GitStorageError::InvalidConfiguration("encryption key id is too long".into())
        })?;
        let mut nonce_prefix = [0_u8; 8];
        getrandom::fill(&mut nonce_prefix).map_err(|error| {
            GitStorageError::InvalidConfiguration(format!("creating segment nonce: {error}"))
        })?;
        let mut header = Vec::with_capacity(HEADER_FIXED_BYTES + key_id_bytes.len());
        header.extend_from_slice(MAGIC);
        header.extend_from_slice(&ENCODING_VERSION.to_be_bytes());
        header.extend_from_slice(&key_id_len.to_be_bytes());
        header.extend_from_slice(&nonce_prefix);
        header.extend_from_slice(&frame_bytes.to_be_bytes());
        header.extend_from_slice(key_id_bytes);
        Ok(Self {
            cipher: segment_cipher(key, repository_id, segment_id),
            header,
            nonce_prefix,
            repository_id: repository_id.to_string(),
            segment_id: segment_id.to_string(),
            next_counter: 0,
        })
    }

    pub(crate) fn header(&self) -> &[u8] {
        &self.header
    }

    pub(crate) fn encrypt_data(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, GitStorageError> {
        if plaintext.is_empty() {
            return Err(GitStorageError::InvalidEnvelope(
                "data frames cannot be empty".into(),
            ));
        }
        self.encrypt_frame(plaintext, 0)
    }

    pub(crate) fn encrypt_final(&mut self) -> Result<Vec<u8>, GitStorageError> {
        self.encrypt_frame(&[], FINAL_FLAG)
    }

    fn encrypt_frame(&mut self, plaintext: &[u8], flags: u8) -> Result<Vec<u8>, GitStorageError> {
        let counter = self.next_counter;
        self.next_counter = self
            .next_counter
            .checked_add(1)
            .ok_or_else(|| GitStorageError::InvalidEnvelope("too many encryption frames".into()))?;
        let plaintext_len = u32::try_from(plaintext.len())
            .map_err(|_| GitStorageError::InvalidEnvelope("encryption frame exceeds u32".into()))?;
        let frame_header = frame_header(counter, plaintext_len, flags);
        let aad = associated_data(
            &self.header,
            &self.repository_id,
            &self.segment_id,
            &frame_header,
        );
        let nonce = nonce(self.nonce_prefix, counter);
        let ciphertext = self
            .cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad: &aad,
                },
            )
            .map_err(|_| GitStorageError::Encryption)?;
        let mut frame = Vec::with_capacity(FRAME_HEADER_BYTES + ciphertext.len());
        frame.extend_from_slice(&frame_header);
        frame.extend_from_slice(&ciphertext);
        Ok(frame)
    }
}

pub(crate) struct EnvelopeReader {
    cipher: ChaCha20Poly1305,
    header: Vec<u8>,
    nonce_prefix: [u8; 8],
    repository_id: String,
    segment_id: String,
    frame_bytes: usize,
    next_counter: u32,
    saw_final: bool,
}

pub(crate) enum DecryptedFrame {
    Data(Vec<u8>),
    Final,
}

impl EnvelopeReader {
    pub(crate) async fn read_header<R: AsyncRead + Unpin>(
        source: &mut R,
        key: &SegmentEncryptionKey,
        repository_id: &str,
        segment_id: &str,
    ) -> Result<Self, GitStorageError> {
        let mut fixed = [0_u8; HEADER_FIXED_BYTES];
        read_exact_envelope(source, &mut fixed).await?;
        if &fixed[..MAGIC.len()] != MAGIC {
            return Err(GitStorageError::InvalidEnvelope("wrong magic".into()));
        }
        let version = u32::from_be_bytes(fixed[8..12].try_into().expect("fixed slice"));
        if version != ENCODING_VERSION {
            return Err(GitStorageError::InvalidEnvelope(format!(
                "unsupported encoding version {version}"
            )));
        }
        let key_id_len =
            u16::from_be_bytes(fixed[12..14].try_into().expect("fixed slice")) as usize;
        if key_id_len == 0 || key_id_len > MAX_KEY_ID_BYTES {
            return Err(GitStorageError::InvalidEnvelope(
                "invalid encryption key id length".into(),
            ));
        }
        let nonce_prefix = fixed[14..22].try_into().expect("fixed slice");
        let frame_bytes =
            u32::from_be_bytes(fixed[22..26].try_into().expect("fixed slice")) as usize;
        if frame_bytes == 0 || frame_bytes > MAX_FRAME_BYTES {
            return Err(GitStorageError::InvalidEnvelope(
                "invalid encryption frame size".into(),
            ));
        }
        let mut key_id = vec![0_u8; key_id_len];
        read_exact_envelope(source, &mut key_id).await?;
        if key_id != key.key_id.as_bytes() {
            return Err(GitStorageError::InvalidEnvelope(
                "encryption key id does not match".into(),
            ));
        }
        let mut header = fixed.to_vec();
        header.extend_from_slice(&key_id);
        Ok(Self {
            cipher: segment_cipher(key, repository_id, segment_id),
            header,
            nonce_prefix,
            repository_id: repository_id.to_string(),
            segment_id: segment_id.to_string(),
            frame_bytes,
            next_counter: 0,
            saw_final: false,
        })
    }

    pub(crate) async fn next<R: AsyncRead + Unpin>(
        &mut self,
        source: &mut R,
    ) -> Result<DecryptedFrame, GitStorageError> {
        if self.saw_final {
            return Err(GitStorageError::InvalidEnvelope(
                "data follows the final frame".into(),
            ));
        }
        let mut frame_header_bytes = [0_u8; FRAME_HEADER_BYTES];
        read_exact_envelope(source, &mut frame_header_bytes).await?;
        let counter = u32::from_be_bytes(frame_header_bytes[..4].try_into().expect("fixed slice"));
        let plaintext_len =
            u32::from_be_bytes(frame_header_bytes[4..8].try_into().expect("fixed slice")) as usize;
        let flags = frame_header_bytes[8];
        if counter != self.next_counter {
            return Err(GitStorageError::InvalidEnvelope(format!(
                "frame {counter} arrived where frame {} was required",
                self.next_counter
            )));
        }
        if flags & !FINAL_FLAG != 0 {
            return Err(GitStorageError::InvalidEnvelope(
                "frame has unknown flags".into(),
            ));
        }
        let is_final = flags == FINAL_FLAG;
        if is_final != (plaintext_len == 0) {
            return Err(GitStorageError::InvalidEnvelope(
                "only the final frame may be empty".into(),
            ));
        }
        if plaintext_len > self.frame_bytes {
            return Err(GitStorageError::InvalidEnvelope(
                "frame exceeds the envelope frame size".into(),
            ));
        }
        let mut ciphertext = vec![0_u8; plaintext_len + TAG_BYTES];
        read_exact_envelope(source, &mut ciphertext).await?;
        let aad = associated_data(
            &self.header,
            &self.repository_id,
            &self.segment_id,
            &frame_header_bytes,
        );
        let nonce = nonce(self.nonce_prefix, counter);
        let plaintext = self
            .cipher
            .decrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &ciphertext,
                    aad: &aad,
                },
            )
            .map_err(|_| GitStorageError::InvalidEnvelope("frame authentication failed".into()))?;
        self.next_counter = self
            .next_counter
            .checked_add(1)
            .ok_or_else(|| GitStorageError::InvalidEnvelope("too many encryption frames".into()))?;
        if is_final {
            self.saw_final = true;
            Ok(DecryptedFrame::Final)
        } else {
            Ok(DecryptedFrame::Data(plaintext))
        }
    }
}

fn frame_header(counter: u32, plaintext_len: u32, flags: u8) -> [u8; FRAME_HEADER_BYTES] {
    let mut header = [0_u8; FRAME_HEADER_BYTES];
    header[..4].copy_from_slice(&counter.to_be_bytes());
    header[4..8].copy_from_slice(&plaintext_len.to_be_bytes());
    header[8] = flags;
    header
}

fn segment_cipher(
    key: &SegmentEncryptionKey,
    repository_id: &str,
    segment_id: &str,
) -> ChaCha20Poly1305 {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&key.key)
        .expect("HMAC accepts a 32-byte segment key");
    mac.update(b"scope-git-segment-v2\0");
    mac.update(&(repository_id.len() as u64).to_be_bytes());
    mac.update(repository_id.as_bytes());
    mac.update(&(segment_id.len() as u64).to_be_bytes());
    mac.update(segment_id.as_bytes());
    let derived_key = mac.finalize().into_bytes();
    ChaCha20Poly1305::new(Key::from_slice(&derived_key))
}

fn nonce(prefix: [u8; 8], counter: u32) -> [u8; 12] {
    let mut nonce = [0_u8; 12];
    nonce[..8].copy_from_slice(&prefix);
    nonce[8..].copy_from_slice(&counter.to_be_bytes());
    nonce
}

fn associated_data(
    header: &[u8],
    repository_id: &str,
    segment_id: &str,
    frame_header: &[u8; FRAME_HEADER_BYTES],
) -> Vec<u8> {
    let repository = repository_id.as_bytes();
    let segment = segment_id.as_bytes();
    let mut aad = Vec::with_capacity(header.len() + repository.len() + segment.len() + 17);
    aad.extend_from_slice(header);
    aad.extend_from_slice(&(repository.len() as u32).to_be_bytes());
    aad.extend_from_slice(repository);
    aad.extend_from_slice(&(segment.len() as u32).to_be_bytes());
    aad.extend_from_slice(segment);
    aad.extend_from_slice(frame_header);
    aad
}

async fn read_exact_envelope<R: AsyncRead + Unpin>(
    source: &mut R,
    target: &mut [u8],
) -> Result<(), GitStorageError> {
    source
        .read_exact(target)
        .await
        .map(|_| ())
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::UnexpectedEof {
                GitStorageError::InvalidEnvelope("truncated stream".into())
            } else {
                GitStorageError::Multipart(crate::MultipartError::new(error.to_string()))
            }
        })
}

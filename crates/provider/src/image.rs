//! image: one bounded frame a model may look at.
//!
//! Owner: this module owns the image part, its media types and caps, and the
//! one base64 data URL the chat wire needs. Units: bytes. Determinism: the
//! same bytes give the same part and the same data URL; nothing here reads a
//! clock, the network or a file. Errors: [`ImageError`] carries a label only,
//! never image bytes.
//!
//! The part is data, not an instruction: it rides beside the prompt as one
//! `image_url` content block on the chat wires. Other wires stay text-only
//! until their own part lands, so a request with an image is never silently
//! reshaped on a wire that cannot carry it.

/// Largest image part one request may carry, in bytes.
pub const MAX_IMAGE_BYTES: usize = 1024 * 1024;

/// PNG signature: the eight bytes every PNG starts with.
const PNG_SIGNATURE: &[u8] = &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
/// JPEG frame prefix: the three bytes every JPEG starts with.
const JPEG_SIGNATURE: &[u8] = &[0xff, 0xd8, 0xff];
/// Standard base64 alphabet, the one a data URL uses.
const BASE64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Media type of one image part. The set is closed; unknown types are refused
/// before they reach a wire.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ImageMedia {
    /// PNG, the door's capture format.
    Png,
    /// JPEG, for a smaller frame.
    Jpeg,
}

impl ImageMedia {
    /// MIME type string the vendor body spells.
    pub const fn mime(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
        }
    }

    /// Bytes every file of this type starts with.
    const fn signature(self) -> &'static [u8] {
        match self {
            Self::Png => PNG_SIGNATURE,
            Self::Jpeg => JPEG_SIGNATURE,
        }
    }
}

/// Why an image part was refused.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ImageError {
    /// The part carries no bytes.
    Empty,
    /// The part is over [`MAX_IMAGE_BYTES`].
    TooLong,
    /// The bytes do not start with the media type's signature.
    BadMedia,
}

impl std::fmt::Display for ImageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            Self::Empty => "image part is empty",
            Self::TooLong => "image part is over the size cap",
            Self::BadMedia => "image part does not match its media type",
        };
        f.write_str(text)
    }
}

impl std::error::Error for ImageError {}

/// One bounded image: a media type plus its bytes. Fields are private, so an
/// over-cap or mislabelled part cannot be built; [`ImagePart::new`] is the
/// only door.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ImagePart {
    media: ImageMedia,
    bytes: Vec<u8>,
}

impl ImagePart {
    /// Check `bytes` against the cap and `media`'s signature, then wrap them.
    pub fn new(media: ImageMedia, bytes: impl Into<Vec<u8>>) -> Result<Self, ImageError> {
        let bytes = bytes.into();
        if bytes.is_empty() {
            return Err(ImageError::Empty);
        }
        if bytes.len() > MAX_IMAGE_BYTES {
            return Err(ImageError::TooLong);
        }
        if !bytes.starts_with(media.signature()) {
            return Err(ImageError::BadMedia);
        }
        Ok(Self { media, bytes })
    }

    /// Media type of this part.
    pub const fn media(&self) -> ImageMedia {
        self.media
    }

    /// Image bytes, as the caller supplied them.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Size in bytes.
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Always false: [`ImagePart::new`] refuses empty bytes.
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// The vendor-edge form: one `data:` URL whose payload is standard base64.
    /// This is the only place image bytes are encoded for a wire.
    pub fn data_url(&self) -> String {
        let mut url =
            String::with_capacity(self.media.mime().len() + 16 + self.bytes.len() * 4 / 3);
        url.push_str("data:");
        url.push_str(self.media.mime());
        url.push_str(";base64,");
        base64(&self.bytes, &mut url);
        url
    }
}

/// Standard base64 with padding, appended to `out`.
fn base64(bytes: &[u8], out: &mut String) {
    for chunk in bytes.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = chunk.get(1).copied().map_or(0, u32::from);
        let b2 = chunk.get(2).copied().map_or(0, u32::from);
        let triple = (b0 << 16) | (b1 << 8) | b2;
        for (position, slot) in [18, 12, 6, 0].into_iter().enumerate() {
            if position <= chunk.len() {
                let index = usize::try_from((triple >> slot) & 0x3f).unwrap_or(0);
                out.push(char::from(BASE64_ALPHABET[index]));
            } else {
                out.push('=');
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The eight-byte PNG signature plus one payload byte.
    fn png() -> Vec<u8> {
        let mut bytes = PNG_SIGNATURE.to_vec();
        bytes.extend_from_slice(b"\x01\x02\x03");
        bytes
    }

    #[test]
    fn the_part_checks_size_media_and_emptiness() {
        let part = ImagePart::new(ImageMedia::Png, png()).expect("a small png");
        assert_eq!(part.media(), ImageMedia::Png);
        assert_eq!(part.len(), 11);
        assert!(!part.is_empty());
        assert_eq!(
            ImagePart::new(ImageMedia::Png, Vec::new()),
            Err(ImageError::Empty)
        );
        assert_eq!(
            ImagePart::new(ImageMedia::Jpeg, png()),
            Err(ImageError::BadMedia),
            "a png cannot claim to be a jpeg"
        );
        let over = vec![0u8; MAX_IMAGE_BYTES + 1];
        assert_eq!(
            ImagePart::new(ImageMedia::Png, over),
            Err(ImageError::TooLong)
        );
        assert!(ImageError::TooLong.to_string().contains("size cap"));
    }

    #[test]
    fn the_data_url_is_standard_base64_at_the_edge() {
        // The PNG signature plus one `a`: nine bytes, three full groups, so no
        // padding. `iVBORw0KGgph` is their standard base64.
        let mut bytes = PNG_SIGNATURE.to_vec();
        bytes.push(b'a');
        let part = ImagePart::new(ImageMedia::Png, bytes).expect("png signature plus a byte");
        let url = part.data_url();
        assert_eq!(url, "data:image/png;base64,iVBORw0KGgph");
        // A one-byte tail carries two pad marks: `TQ==` is `M`.
        let mut tail = PNG_SIGNATURE.to_vec();
        tail.extend_from_slice(b"aM");
        let ten = ImagePart::new(ImageMedia::Png, tail).expect("png signature plus two bytes");
        assert!(ten.data_url().ends_with("TQ=="), "{}", ten.data_url());
    }
}

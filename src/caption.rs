use std::{
    borrow::Cow,
    collections::BTreeMap,
    io::{Cursor, Write},
};

use anyhow::Context;
use keyvalues_parser::Vdf;

use byteorder::{LittleEndian, WriteBytesExt};
use crc32fast::Hasher;

/// The size of a block in the VDF file.
pub const BLOCK_SIZE: usize = 8192;
/// The size of the header in the VDF file.
pub const HEADER_SIZE: usize = 24;
/// The size of a directory entry in the VDF file.
/// CRC: 4 bytes
/// Block index: 4 bytes
/// Offset: 2 bytes
/// Length: 2 bytes
pub const DIRECTORY_ENTRY_SIZE: usize = 12;

/// A representation of a caption to be compiled into a dat file.
pub struct Caption {
    /// The language of the caption.
    language: String,
    /// The tokens of the caption.
    tokens: BTreeMap<String, String>,
}

impl Caption {
    /// Create a new caption.
    #[must_use]
    pub fn new(language: String, tokens: BTreeMap<String, String>) -> Self {
        Self { language, tokens }
    }

    /// Set a token.
    /// If the token already exists, it will be overwritten.
    /// If the token does not exist, it will be added.
    pub fn set_token(&mut self, key: String, value: String) {
        self.tokens.insert(key, value);
    }

    /// Remove a token.
    pub fn remove_token(&mut self, key: &str) {
        self.tokens.remove(key);
    }

    /// Create a Vdf representation of the caption.
    /// This can be used to write the caption to a file, you can use the `to_string` method to get the string representation.
    #[must_use]
    pub fn to_vdf(&self) -> Vdf {
        let mut vdf = Vdf::new(
            Cow::from("lang"),
            keyvalues_parser::Value::Obj(keyvalues_parser::Obj::new()),
        );

        if let Some(obj) = vdf.value.get_mut_obj() {
            obj.insert(
                Cow::from("Language"),
                vec![keyvalues_parser::Value::Str(Cow::from(
                    self.language.clone(),
                ))],
            );
        }

        let mut tokens_vdf = keyvalues_parser::Obj::new();
        for (key, value) in &self.tokens {
            tokens_vdf.insert(
                Cow::from(key),
                vec![keyvalues_parser::Value::Str(Cow::from(value))],
            );
        }

        if let Some(obj) = vdf.value.get_mut_obj() {
            obj.insert(
                Cow::from("Tokens"),
                vec![keyvalues_parser::Value::Obj(tokens_vdf)],
            );
        }

        vdf
    }

    /// Create a caption from a Vdf caption file.
    ///
    /// # Errors
    ///
    /// This function will return an error:
    /// - If parsing the VDF string fails. This could be due to an improperly formatted VDF string.
    /// - If the VDF does not contain an object at its root.
    /// - If the "Language" or "Tokens" field is missing in the VDF object.
    /// - If the "Language" field is present but its value is not a string.
    /// - If the "Tokens" field is present but its value is not an object.
    /// - If a token key is not a string.
    /// - If a token value is not a string.
    pub fn from_vdf(vdf: &str) -> Result<Self, anyhow::Error> {
        let parsed_vdf = Vdf::parse(vdf).context("Failed to parse localconfig.vdf")?;
        let language = parsed_vdf
            .value
            .get_obj()
            .context("Missing object in VDF")?
            .get("Language")
            .context("Language not found")?
            .first()
            .context("Language value missing")?
            .get_str()
            .context("Language is not a string")?;

        let tokens_obj = parsed_vdf
            .value
            .get_obj()
            .context("Missing object in VDF")?
            .get("Tokens")
            .context("Tokens not found")?
            .first()
            .context("Tokens value missing")?
            .get_obj()
            .context("Tokens is not an object")?;

        let tokens = tokens_obj
            .iter()
            .map(|(key, value)| {
                let token_value = value
                    .first()
                    .context("Token value missing")?
                    .get_str()
                    .context("Token value is not a string")?;
                Ok((key.to_string(), token_value.to_string()))
            })
            .collect::<Result<BTreeMap<String, String>, anyhow::Error>>()?;

        Ok(Self {
            language: language.to_string(),
            tokens,
        })
    }
    /// Compile the caption into a dat file.
    ///
    /// # Errors
    ///
    /// This function will return an error:
    /// - If the header size is invalid after writing the initial header information. This is checked by comparing the current position of the buffer against the expected header size (`HEADER_SIZE`). If they do not match, an error is returned.
    /// - If converting the block size, number of entries, or data offset to an `i32` fails. This could happen if the values exceed the maximum value that can be represented by an `i32`.
    /// - If writing to the buffer fails at any point due to issues like insufficient memory or other IO errors.
    /// - If converting the position of the block cursor or the length of the encoded string to a `usize` or other numeric types fails. This could occur if the values exceed the maximum value that can be represented by the target numeric type.
    /// - If calculating the CRC32 checksum fails. This could happen if there are issues with the input data or the hashing process.
    pub fn compile(&self) -> Result<Vec<u8>, anyhow::Error> {
        let tokens_len = self.tokens.len();
        let mut buf = Cursor::new(Vec::<u8>::new());
        let mut data_buf = Cursor::new(Vec::<u8>::new());

        let mut block = vec![0u8; BLOCK_SIZE];
        let mut block_cursor = Cursor::new(&mut block);

        // Write the header
        buf.write_all(b"VCCD")?; // Magic number
        buf.write_i32::<LittleEndian>(1)?; // Version
        let num_blocks_pos = buf.position();
        buf.write_i32::<LittleEndian>(0)?; // Number of blocks (placeholder)
        buf.write_i32::<LittleEndian>(i32::try_from(BLOCK_SIZE)?)?; // Block size
        buf.write_i32::<LittleEndian>(i32::try_from(tokens_len)?)?; // Number of entries

        let dict_padding = (512 - (HEADER_SIZE + tokens_len * DIRECTORY_ENTRY_SIZE) % 512) % 512;
        let data_offset = HEADER_SIZE + tokens_len * DIRECTORY_ENTRY_SIZE + dict_padding;
        buf.write_i32::<LittleEndian>(i32::try_from(data_offset)?)?;

        if usize::try_from(buf.position())? != HEADER_SIZE {
            return Err(anyhow::anyhow!("Invalid header size"));
        }

        let mut block_num = 0;

        for (token, str) in &self.tokens {
            let encoded_str = str
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<u8>>();
            let len = encoded_str.len() + 2; // utf16 + null terminator
            let block_cursor_pos = usize::try_from(block_cursor.position())?;
            if block_cursor_pos + len >= BLOCK_SIZE {
                // Finalize current block
                data_buf
                    // .write_all(&block[..block_cursor.position() as usize])
                    .write_all(&block_cursor.get_ref()[..block_cursor_pos])?;
                block_cursor.set_position(0);
                block_num += 1;
            }
            let old_offset = block_cursor.position();
            block_cursor.write_all(&encoded_str)?;
            block_cursor.write_u16::<LittleEndian>(0)?; // null terminator

            // Dictionary entry
            let mut hasher = Hasher::new();
            hasher.update(token.to_lowercase().as_bytes());
            let crc = hasher.finalize();
            buf.write_u32::<LittleEndian>(crc)?;
            buf.write_u32::<LittleEndian>(u32::try_from(block_num)?)?;
            buf.write_u16::<LittleEndian>(u16::try_from(old_offset)?)?;
            buf.write_u16::<LittleEndian>(u16::try_from(len)?)?;
        }

        let block_cursor_pos = usize::try_from(block_cursor.position())?;
        // Finalize last block
        if block_cursor_pos > 0 {
            data_buf.write_all(&block_cursor.get_ref()[..block_cursor_pos])?;
        }

        // Dictionary padding
        buf.write_all(&vec![0u8; dict_padding])?;

        // Append data buffer to main buffer
        buf.get_mut().extend_from_slice(&data_buf.into_inner());

        // Update numblocks
        let final_size = buf.get_ref().len();
        buf.set_position(num_blocks_pos);
        buf.write_i32::<LittleEndian>(block_num + 1)?;

        // Reset position to end to get the full buffer
        buf.set_position(final_size as u64);

        Ok(buf.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn test_to_vdf() {
        let language = "english".to_string();
        let mut tokens = BTreeMap::new();
        tokens.insert(
            "DIAG_TEXT_01".to_string(),
            "<clr:255,125,240>A test caption!".to_string(),
        );
        tokens.insert(
            "DIAG_TEXT_02".to_string(),
            "<clr:55,250,240>Another test caption!".to_string(),
        );

        let caption = Caption::new(language, tokens);

        let created_vdf = caption.to_vdf();

        let test_vdf =
            Vdf::parse(include_str!("../tests/vdf_caption.txt")).expect("Failed to parse VDF");

        assert_eq!(created_vdf, test_vdf);
    }

    #[test]
    fn test_from_vdf() {
        let vdf = include_str!("../tests/vdf_caption.txt");

        let caption = Caption::from_vdf(vdf).expect("Failed to create caption from VDF");

        let language = "english".to_string();
        let mut tokens = BTreeMap::new();
        tokens.insert(
            "DIAG_TEXT_01".to_string(),
            "<clr:255,125,240>A test caption!".to_string(),
        );
        tokens.insert(
            "DIAG_TEXT_02".to_string(),
            "<clr:55,250,240>Another test caption!".to_string(),
        );

        let test_caption = Caption::new(language, tokens);

        assert_eq!(caption.language, test_caption.language);
        assert_eq!(caption.tokens, test_caption.tokens);
    }

    #[test]
    fn test_compile_header() {
        let language = "english".to_string();
        let mut tokens = BTreeMap::new();
        tokens.insert(
            "DIAG_TEXT_01".to_string(),
            "<clr:255,125,240>A test caption!".to_string(),
        );
        tokens.insert(
            "DIAG_TEXT_02".to_string(),
            "<clr:55,250,240>Another test caption!".to_string(),
        );

        let caption = Caption::new(language, tokens);

        let compiled = caption.compile().expect("Failed to compile caption");

        // std::fs::write("compiled_caption_test.dat", &compiled).expect("Failed to write compiled caption");

        assert_eq!(compiled, include_bytes!("../tests/compiled_caption.dat"));
    }
}

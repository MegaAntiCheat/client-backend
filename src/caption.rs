use std::{
    borrow::Cow,
    collections::BTreeMap,
    io::{Cursor, Write},
};

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
    pub fn to_vdf(&self) -> Vdf {
        let mut vdf = Vdf::new(
            Cow::from("lang"),
            keyvalues_parser::Value::Obj(keyvalues_parser::Obj::new()),
        );

        vdf.value.get_mut_obj().unwrap().insert(
            Cow::from("Language"),
            vec![keyvalues_parser::Value::Str(Cow::from(
                self.language.clone(),
            ))],
        );

        let mut tokens_vdf = keyvalues_parser::Obj::new();
        for (key, value) in &self.tokens {
            tokens_vdf.insert(
                Cow::from(key),
                vec![keyvalues_parser::Value::Str(Cow::from(value))],
            );
        }

        vdf.value.get_mut_obj().unwrap().insert(
            Cow::from("Tokens"),
            vec![keyvalues_parser::Value::Obj(tokens_vdf)],
        );

        vdf
    }

    /// Create a caption from a Vdf caption file.
    pub fn from_vdf(vdf: &str) -> Self {
        let parsed_vdf = Vdf::parse(vdf).unwrap();
        let language = parsed_vdf
            .value
            .get_obj()
            .unwrap()
            .get("Language")
            .unwrap()
            .first()
            .unwrap()
            .get_str();

        let tokens = parsed_vdf
            .value
            .get_obj()
            .unwrap()
            .get("Tokens")
            .unwrap()
            .first()
            .unwrap()
            .get_obj()
            .unwrap()
            .iter()
            .map(|(key, value)| {
                (
                    key.to_string(),
                    value
                        .first()
                        .unwrap()
                        .get_str()
                        .unwrap_or_default()
                        .to_string(),
                )
            })
            .collect();

        Self {
            language: language.unwrap_or_default().to_string(),
            tokens,
        }
    }

    /// Compile the caption into a dat file.
    #[must_use]
    pub fn compile(&self) -> Vec<u8> {
        let tokens_len = self.tokens.len();
        let mut buf = Cursor::new(Vec::<u8>::new());
        let mut data_buf = Cursor::new(Vec::<u8>::new());

        let mut block = vec![0u8; BLOCK_SIZE];
        let mut block_cursor = Cursor::new(&mut block);

        // Write the header
        buf.write_all(b"VCCD").unwrap(); // Magic number
        buf.write_i32::<LittleEndian>(1).unwrap(); // Version
        let num_blocks_pos = buf.position() as usize;
        buf.write_i32::<LittleEndian>(0).unwrap(); // Number of blocks (placeholder)
        buf.write_i32::<LittleEndian>(BLOCK_SIZE as i32).unwrap(); // Block size
        buf.write_i32::<LittleEndian>(tokens_len as i32).unwrap(); // Number of entries

        let dict_padding = (512 - (HEADER_SIZE + tokens_len * DIRECTORY_ENTRY_SIZE) % 512) % 512;
        let data_offset = HEADER_SIZE + tokens_len * DIRECTORY_ENTRY_SIZE + dict_padding;
        buf.write_i32::<LittleEndian>(data_offset as i32).unwrap();

        if buf.position() as usize != HEADER_SIZE {
            panic!("Header size is incorrect");
        }

        println!("Header: {:?}", buf.get_ref());

        let mut block_num = 0;

        for (token, str) in self.tokens.iter() {
            println!("Token: {}, String: {}", token, str);
            let encoded_str = str
                .encode_utf16()
                .flat_map(|c| c.to_le_bytes())
                .collect::<Vec<u8>>();
            let len = encoded_str.len() + 2; // utf16 + null terminator
            let block_cursor_pos = block_cursor.position() as usize;
            if block_cursor_pos + len >= BLOCK_SIZE {
                // Finalize current block
                data_buf
                    // .write_all(&block[..block_cursor.position() as usize])
                    .write_all(&block_cursor.get_ref()[..block_cursor_pos])
                    .unwrap();
                block_cursor.set_position(0);
                block_num += 1;
            }
            let old_offset = block_cursor.position();
            block_cursor.write_all(&encoded_str).unwrap();
            block_cursor.write_u16::<LittleEndian>(0).unwrap(); // null terminator

            // Dictionary entry
            let mut hasher = Hasher::new();
            hasher.update(token.to_lowercase().as_bytes());
            let crc = hasher.finalize();
            buf.write_u32::<LittleEndian>(crc).unwrap();
            buf.write_u32::<LittleEndian>(block_num as u32).unwrap();
            buf.write_u16::<LittleEndian>(old_offset as u16).unwrap();
            buf.write_u16::<LittleEndian>(len as u16).unwrap();
        }

        let block_cursor_pos = block_cursor.position() as usize;
        println!("Block cursor position: {:?}", block_cursor_pos);
        // Finalize last block
        if block_cursor_pos > 0 {
            data_buf
                .write_all(&block_cursor.get_ref()[..block_cursor_pos])
                .unwrap();
        }

        // Dictionary padding
        buf.write_all(&vec![0u8; dict_padding]).unwrap();

        // Append data buffer to main buffer
        buf.get_mut().extend_from_slice(&data_buf.into_inner());

        // Update numblocks
        let final_size = buf.get_ref().len();
        buf.set_position(num_blocks_pos as u64);
        buf.write_i32::<LittleEndian>((block_num + 1) as i32)
            .unwrap();

        // Reset position to end to get the full buffer
        buf.set_position(final_size as u64);

        buf.into_inner()
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

        let test_vdf = Vdf::parse(include_str!("../tests/vdf_caption.txt")).unwrap();

        assert_eq!(created_vdf, test_vdf);
    }

    #[test]
    fn test_from_vdf() {
        let vdf = include_str!("../tests/vdf_caption.txt");

        let caption = Caption::from_vdf(vdf);

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

        let compiled = caption.compile();

        // std::fs::write("compiled_caption_test.dat", &compiled).unwrap();

        assert_eq!(compiled, include_bytes!("../tests/compiled_caption.dat"));
    }
}

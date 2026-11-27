//! Layer ids: `u64` inside FigCraft, `"12:34"` inside a Figma plugin.
//!
//! Figma hands plugins strings like `"12:34"` (`figma.getNodeById`, `node.id`), and plugins keep
//! them: in `pluginData`, in `setRelaunchData`, in their own JSON settings, and as map keys across
//! runs. The mapping must therefore be **injective** and **derived from the id alone**: the same
//! layer gets the same string after undo, after a copy, and after the file is saved and reopened.
//!
//! FigCraft's ids are `client_id << 32 | counter` (see `figcraft-doc::ids`): the high half is the
//! live-session client space (0 outside a session), the low half the counter. Rendering both as
//! decimal numbers separated by `:` keeps them injective on the whole `u64` range and stable.
//!
//! Two ids are reserved by Figma: `"0:0"` is the document root (`figma.root`), and layer 0 does not
//! exist. [`figma_id`] refuses 0, and [`node_id`] refuses `"0:0"`.

use thiserror::Error;

/// The id Figma gives the document root (`figma.root`).
pub const DOCUMENT_ID: &str = "0:0";

/// Why an id can't be mapped.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IdError {
    #[error("a layer id looks like `12:34`, got `{0}`")]
    Shape(String),
    #[error("a layer id's halves are 32-bit numbers, got `{0}`")]
    Range(String),
    #[error("`{0}` is the document root, not a layer")]
    Document(String),
    #[error("layer 0 is reserved for the document")]
    Reserved,
}

/// The string a Figma plugin sees for FigCraft layer `id`.
pub fn figma_id(id: u64) -> Result<String, IdError> {
    if id == 0 {
        return Err(IdError::Reserved);
    }
    Ok(format!("{}:{}", id >> 32, id & 0xFFFF_FFFF))
}

/// The FigCraft layer an id string names.
pub fn node_id(text: &str) -> Result<u64, IdError> {
    let text = text.trim();
    if text == DOCUMENT_ID {
        return Err(IdError::Document(text.to_string()));
    }
    let Some((high, low)) = text.split_once(':') else { return Err(IdError::Shape(text.to_string())) };
    let high: u32 = high.parse().map_err(|_| IdError::Range(text.to_string()))?;
    let low: u32 = low.parse().map_err(|_| IdError::Range(text.to_string()))?;
    let id = (u64::from(high) << 32) | u64::from(low);
    if id == 0 {
        return Err(IdError::Document(text.to_string()));
    }
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::{DOCUMENT_ID, IdError, figma_id, node_id};

    #[test]
    fn round_trips_every_shape_of_id() {
        for id in [1u64, 7, 0x1_0000_0001, 0xFFFF_FFFF, u64::MAX, 42 << 32] {
            let text = figma_id(id).unwrap();
            assert_eq!(node_id(&text).unwrap(), id, "{id} -> {text}");
        }
        assert_eq!(figma_id(0x1_0000_0001).unwrap(), "1:1");
        assert_eq!(figma_id(u64::MAX).unwrap(), "4294967295:4294967295");
    }

    #[test]
    fn refuses_the_document_and_layer_zero() {
        assert_eq!(node_id(DOCUMENT_ID), Err(IdError::Document(DOCUMENT_ID.into())));
        assert_eq!(figma_id(0), Err(IdError::Reserved));
        assert_eq!(node_id("0:0"), Err(IdError::Document("0:0".into())));
    }

    #[test]
    fn refuses_malformed_strings() {
        assert!(matches!(node_id("12"), Err(IdError::Shape(_))));
        assert!(matches!(node_id("a:b"), Err(IdError::Range(_))));
        assert!(matches!(node_id("1:2:3"), Err(IdError::Range(_))));
        assert!(matches!(node_id("4294967296:1"), Err(IdError::Range(_))));
        assert!(matches!(node_id("-1:2"), Err(IdError::Range(_))));
    }

    #[test]
    fn trims_whitespace_like_a_plugin_typing_an_id() {
        assert_eq!(node_id(" 3:4 ").unwrap(), 3 << 32 | 4);
    }
}

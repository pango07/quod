use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

const PREFIX: &str = "b3:";

/// A BLAKE3 digest. Written as `b3:` followed by 64 lowercase hex characters.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hash([u8; 32]);

impl Hash {
    pub fn of(bytes: &[u8]) -> Hash {
        Hash(*blake3::hash(bytes).as_bytes())
    }

    pub fn from_blake3(h: blake3::Hash) -> Hash {
        Hash(*h.as_bytes())
    }

    pub fn hex(&self) -> String {
        let mut s = String::with_capacity(64);
        for b in self.0 {
            s.push(char::from_digit((b >> 4) as u32, 16).unwrap());
            s.push(char::from_digit((b & 0xf) as u32, 16).unwrap());
        }
        s
    }

    /// The first 8 hex characters, as receipts print them.
    pub fn short(&self) -> String {
        self.hex()[..8].to_string()
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{PREFIX}{}", self.hex())
    }
}

impl fmt::Debug for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{PREFIX}{}", self.short())
    }
}

#[derive(Debug, thiserror::Error)]
#[error("not a b3: hash: {0:?}")]
pub struct HashParseError(String);

impl FromStr for Hash {
    type Err = HashParseError;

    fn from_str(s: &str) -> Result<Hash, HashParseError> {
        let err = || HashParseError(s.to_string());
        let hex = s.strip_prefix(PREFIX).ok_or_else(err)?;
        if hex.len() != 64 {
            return Err(err());
        }
        let mut out = [0u8; 32];
        for (i, pair) in hex.as_bytes().chunks(2).enumerate() {
            let nib = |c: u8| match c {
                b'0'..=b'9' => Some(c - b'0'),
                b'a'..=b'f' => Some(c - b'a' + 10),
                _ => None,
            };
            out[i] = (nib(pair[0]).ok_or_else(err)? << 4) | nib(pair[1]).ok_or_else(err)?;
        }
        Ok(Hash(out))
    }
}

impl Serialize for Hash {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Hash {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Hash, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_round_trips() {
        let h = Hash::of(b"hello");
        let s = h.to_string();
        assert!(s.starts_with("b3:") && s.len() == 67);
        assert_eq!(s.parse::<Hash>().unwrap(), h);
    }

    #[test]
    fn rejects_uppercase_and_bad_length() {
        let s = Hash::of(b"x")
            .to_string()
            .to_uppercase()
            .replace("B3:", "b3:");
        assert!(s.parse::<Hash>().is_err());
        assert!("b3:abcd".parse::<Hash>().is_err());
        assert!(Hash::of(b"x").hex().parse::<Hash>().is_err());
    }
}

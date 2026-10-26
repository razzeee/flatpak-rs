use serde_json::Value;
use std::io::{self, BufReader, Read, Write};
use std::os::unix::net::UnixStream;

/// Preserve member uniqueness and bound nesting before generic Value dispatch.
pub(crate) fn strict_json(bytes: &[u8]) -> serde_json::Result<Value> {
    use serde::de::{DeserializeSeed, Error, MapAccess, SeqAccess, Visitor};
    struct Seed(usize);
    impl<'de> DeserializeSeed<'de> for Seed {
        type Value = Value;
        fn deserialize<D: serde::Deserializer<'de>>(self, decoder: D) -> Result<Value, D::Error> {
            if self.0 > 16 {
                return Err(D::Error::custom("JSON nesting exceeds 16 containers"));
            }
            decoder.deserialize_any(self)
        }
    }
    impl<'de> Visitor<'de> for Seed {
        type Value = Value;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("bounded JSON with unique object members")
        }
        fn visit_bool<E: Error>(self, v: bool) -> Result<Value, E> {
            Ok(Value::Bool(v))
        }
        fn visit_i64<E: Error>(self, v: i64) -> Result<Value, E> {
            Ok(v.into())
        }
        fn visit_u64<E: Error>(self, v: u64) -> Result<Value, E> {
            Ok(v.into())
        }
        fn visit_f64<E: Error>(self, v: f64) -> Result<Value, E> {
            Ok(serde_json::json!(v))
        }
        fn visit_str<E: Error>(self, v: &str) -> Result<Value, E> {
            Ok(v.into())
        }
        fn visit_unit<E: Error>(self) -> Result<Value, E> {
            Ok(Value::Null)
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
            let mut values = Vec::new();
            while let Some(value) = seq.next_element_seed(Seed(self.0 + 1))? {
                values.push(value);
            }
            Ok(Value::Array(values))
        }
        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
            let mut values = serde_json::Map::new();
            while let Some(key) = map.next_key::<String>()? {
                if values.contains_key(&key) {
                    return Err(A::Error::custom("duplicate object member"));
                }
                values.insert(key, map.next_value_seed(Seed(self.0 + 1))?);
            }
            Ok(Value::Object(values))
        }
    }
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let value = Seed(0).deserialize(&mut decoder)?;
    decoder.end()?;
    Ok(value)
}

pub(crate) fn send(stream: &mut UnixStream, value: &Value) -> io::Result<()> {
    let mut bytes = serde_json::to_vec(value)?;
    if bytes.len() > crate::MAX_MESSAGE {
        return Err(io::Error::other("oversized reply"));
    }
    bytes.push(0);
    stream.write_all(&bytes)
}

pub(crate) fn send_until(
    stream: &mut UnixStream,
    value: &Value,
    canceled: impl Fn() -> bool,
) -> io::Result<()> {
    let mut bytes = serde_json::to_vec(value)?;
    if bytes.len() > crate::MAX_MESSAGE {
        return Err(io::Error::other("oversized reply"));
    }
    bytes.push(0);
    let mut offset = 0;
    while offset < bytes.len() {
        if offset > 0 && canceled() {
            let _ = stream.shutdown(std::net::Shutdown::Both);
            return Err(io::Error::from(io::ErrorKind::Interrupted));
        }
        match stream.write(&bytes[offset..]) {
            Ok(0) => return Err(io::Error::from(io::ErrorKind::WriteZero)),
            Ok(n) => offset += n,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) =>
            {
                if canceled() {
                    // A partial frame cannot be followed by another reply.
                    let _ = stream.shutdown(std::net::Shutdown::Both);
                    return Err(io::Error::from(io::ErrorKind::Interrupted));
                }
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

pub(crate) struct Reader {
    stream: BufReader<UnixStream>,
    frame: Vec<u8>,
}
impl Reader {
    pub fn new(stream: UnixStream) -> Self {
        Self {
            stream: BufReader::new(stream),
            frame: Vec::new(),
        }
    }
    pub fn next(&mut self) -> io::Result<Option<Value>> {
        loop {
            let mut byte = [0];
            let count = match self.stream.read(&mut byte) {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                result => result?,
            };
            if count == 0 {
                return Ok(None);
            }
            if byte[0] == 0 {
                let result = serde_json::from_slice(&self.frame).map_err(io::Error::other);
                self.frame.clear();
                return result.map(Some);
            }
            if self.frame.len() == crate::MAX_MESSAGE {
                return Err(io::Error::other("oversized frame"));
            }
            self.frame.push(byte[0]);
        }
    }
}

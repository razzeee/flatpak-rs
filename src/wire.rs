use serde_json::Value;
use std::io::{self, BufReader, Read, Write};
use std::os::unix::net::UnixStream;

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
